//! Rivest's sequential-squaring time-lock puzzle.
//!
//! Decrypting requires `t` modular squarings, which are inherently sequential:
//! no parallel shortcut exists. The creator uses the factorization of `n` as a
//! trapdoor to build the puzzle instantly, then discards it.

use std::fmt;
use std::hint::black_box;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};

use hmac::{Hmac, KeyInit, Mac};
use num_bigint::BigUint;
use num_traits::{One, Zero};
use sha2::{Digest, Sha256};

use crate::random;

/// Sealing never produces fewer squarings than this.
pub const MIN_SQUARINGS: u64 = 1000;

/// The longest secret a puzzle can hold, in bytes: the pad that hides it is
/// one hash long.
pub const MAX_SECRET_BYTES: usize = 32;

const SPEED_TEST: Duration = Duration::from_millis(400);
const PROGRESS_EVERY: u64 = 20_000;
const PRIME_BITS: u64 = 1024;
const MILLER_RABIN_ROUNDS: usize = 40;
const SMALL_PRIMES: [u32; 25] = [
    2, 3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47, 53, 59, 61, 67, 71, 73, 79, 83, 89, 97,
];

/// Everything needed to open a sealed code, given enough time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Puzzle {
    pub n: BigUint,
    pub a: BigUint,
    pub t: u64,
    pub ct: Vec<u8>,
    pub tag: String,
    pub delay_seconds: u64,
}

impl Puzzle {
    /// Roughly how long a reveal takes on this machine, in seconds. This costs
    /// a speed test. Prefer it to `delay_seconds`, which only says how long the
    /// sealing machine and program would have taken.
    pub fn estimated_seconds(&self) -> Option<f64> {
        Some(self.t as f64 / squarings_per_second(&self.n)?)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolveError {
    /// The file failed its integrity check.
    Damaged,
    /// The caller asked to stop. Progress is not saved.
    Cancelled,
}

impl fmt::Display for SolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Damaged => "the file looks damaged",
            Self::Cancelled => "cancelled",
        })
    }
}

impl std::error::Error for SolveError {}

/// Repeated squaring modulo an odd `n`, in Montgomery form.
///
/// This is the one code path for squaring. Sealing's speed test and reveals
/// both run it, so the delay someone picks is measured on what will really run.
/// It is the fastest of the pure-Rust options measured and stays close to GMP,
/// which leaves little room for a faster program to shorten the delay.
struct Squarer {
    n: Vec<u64>,
    /// -n⁻¹ mod 2⁶⁴
    n_inv: u64,
    /// The current value times 2^(64 × limbs), mod n
    x: Vec<u64>,
    scratch: Vec<u64>,
}

impl Squarer {
    /// None if `n` can't be a real modulus: even, zero or one.
    fn new(n: &BigUint, start: &BigUint) -> Option<Self> {
        if !n.bit(0) || n.is_one() {
            return None;
        }
        let modulus = n.to_u64_digits();
        let limbs = modulus.len();
        let mut x = (((start % n) << (64 * limbs)) % n).to_u64_digits();
        x.resize(limbs, 0);
        // Newton's method doubles the correct low bits on each step
        let mut inv = 1u64;
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(modulus[0].wrapping_mul(inv)));
        }
        Some(Self {
            n: modulus,
            n_inv: inv.wrapping_neg(),
            x,
            scratch: vec![0; 2 * limbs],
        })
    }

    fn square_many(&mut self, count: u64) {
        for _ in 0..count {
            self.square();
        }
    }

    /// x ← x² / R mod n, computing the square and then reducing it.
    #[inline]
    fn square(&mut self) {
        let limbs = self.n.len();
        let (x, t) = (&mut self.x[..], &mut self.scratch[..]);
        t.fill(0);
        // the products of different limbs, which each appear twice
        for i in 0..limbs - 1 {
            let xi = x[i];
            let mut carry = 0;
            for (slot, xj) in t[2 * i + 1..i + limbs].iter_mut().zip(&x[i + 1..limbs]) {
                (*slot, carry) = add_mul(*slot, xi, *xj, carry);
            }
            t[i + limbs] = carry;
        }
        // double them and add the limbs' own squares
        let (mut shifted_out, mut carry) = (0, 0);
        for i in 0..limbs {
            let square = (x[i] as u128) * (x[i] as u128);
            let (low, high) = (t[2 * i], t[2 * i + 1]);
            let doubled_low = (low << 1) | shifted_out;
            let doubled_high = (high << 1) | (low >> 63);
            shifted_out = high >> 63;
            let sum_low = doubled_low as u128 + (square as u64) as u128 + carry as u128;
            let sum_high = doubled_high as u128 + (square >> 64) + (sum_low >> 64);
            t[2 * i] = sum_low as u64;
            t[2 * i + 1] = sum_high as u64;
            carry = (sum_high >> 64) as u64;
        }
        reduce(t, &self.n, self.n_inv, x);
    }

    /// The current value, out of Montgomery form.
    fn value(&mut self) -> BigUint {
        let limbs = self.n.len();
        let mut out = vec![0; limbs];
        let t = &mut self.scratch[..];
        t[..limbs].copy_from_slice(&self.x);
        t[limbs..].fill(0);
        reduce(t, &self.n, self.n_inv, &mut out);
        let bytes: Vec<u8> = out.iter().flat_map(|limb| limb.to_le_bytes()).collect();
        BigUint::from_bytes_le(&bytes)
    }
}

#[inline(always)]
fn add_mul(accumulator: u64, a: u64, b: u64, carry: u64) -> (u64, u64) {
    let v = accumulator as u128 + (a as u128) * (b as u128) + carry as u128;
    (v as u64, (v >> 64) as u64)
}

/// Montgomery reduction: out ← t / R mod n, for t < n·R.
#[inline]
fn reduce(t: &mut [u64], n: &[u64], n_inv: u64, out: &mut [u64]) {
    let limbs = n.len();
    let mut top = 0;
    for i in 0..limbs {
        let m = t[i].wrapping_mul(n_inv);
        let mut carry = 0;
        for (slot, nj) in t[i..i + limbs].iter_mut().zip(n) {
            (*slot, carry) = add_mul(*slot, m, *nj, carry);
        }
        let (sum, overflow_a) = t[i + limbs].overflowing_add(carry);
        let (sum, overflow_b) = sum.overflowing_add(top);
        t[i + limbs] = sum;
        top = overflow_a as u64 + overflow_b as u64;
    }
    out.copy_from_slice(&t[limbs..]);

    // the result is below 2n, so subtracting once is enough
    let mut at_least_n = top != 0;
    if !at_least_n {
        at_least_n = true;
        for (a, b) in out.iter().rev().zip(n.iter().rev()) {
            if a != b {
                at_least_n = a > b;
                break;
            }
        }
    }
    if at_least_n {
        let mut borrow = false;
        for (a, b) in out.iter_mut().zip(n) {
            let (diff, borrow_a) = a.overflowing_sub(*b);
            let (diff, borrow_b) = diff.overflowing_sub(borrow as u64);
            *a = diff;
            borrow = borrow_a | borrow_b;
        }
    }
}

fn is_probable_prime(k: &BigUint) -> bool {
    for p in SMALL_PRIMES {
        if (k % p).is_zero() {
            return *k == BigUint::from(p);
        }
    }
    let one = BigUint::one();
    let k_minus_1 = k - &one;
    let r = k_minus_1.trailing_zeros().expect("k is odd and above 97");
    let d = &k_minus_1 >> r;
    let three = BigUint::from(3u32);
    'rounds: for _ in 0..MILLER_RABIN_ROUNDS {
        let a = random::big_below(&(k - &three)) + 2u32; // in 2..k-1
        let mut x = a.modpow(&d, k);
        if x == one || x == k_minus_1 {
            continue;
        }
        for _ in 1..r {
            x = &x * &x % k;
            if x == k_minus_1 {
                continue 'rounds;
            }
        }
        return false;
    }
    true
}

fn random_prime(bits: u64) -> BigUint {
    loop {
        let mut candidate = random::bits(bits);
        candidate.set_bit(bits - 1, true);
        candidate.set_bit(0, true);
        if is_probable_prime(&candidate) {
            return candidate;
        }
    }
}

/// How many squarings this machine does per second with this modulus, or None
/// if `n` can't be a real modulus.
pub fn squarings_per_second(n: &BigUint) -> Option<f64> {
    let start_value = if *n > BigUint::from(2u32) {
        random::big_below(&(n - 2u32)) + 2u32
    } else {
        BigUint::from(2u32)
    };
    let mut squarer = Squarer::new(n, &start_value)?;
    let start = Instant::now();
    let mut count = 0u64;
    while start.elapsed() < SPEED_TEST {
        squarer.square_many(1000);
        black_box(&squarer.x);
        count += 1000;
    }
    Some(count as f64 / start.elapsed().as_secs_f64())
}

/// SHA-256 of `x` written big-endian at the byte width of `n`. The padding
/// matters: `x` has a leading zero byte in roughly 1 file out of 128-256.
fn derive_key(x: &BigUint, n: &BigUint) -> [u8; 32] {
    let width = n.bits().div_ceil(8) as usize;
    let digits = x.to_bytes_be();
    let mut bytes = vec![0; width];
    bytes[width - digits.len()..].copy_from_slice(&digits);
    Sha256::digest(&bytes).into()
}

fn keystream(key: &[u8; 32]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(key);
    hash.update(b"stream");
    hash.finalize().into()
}

fn hex_tag(key: &[u8; 32], ct: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(ct);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn seal_with_primes(secret: &[u8], p: &BigUint, q: &BigUint, t: u64, delay_seconds: u64) -> Puzzle {
    // a longer secret would silently be cut off
    assert!(
        secret.len() <= MAX_SECRET_BYTES,
        "sealing supports at most {MAX_SECRET_BYTES} bytes"
    );
    let n = p * q;
    let two = BigUint::from(2u32);
    // trapdoor: reduce the exponent mod phi(n), then forget p and q
    let phi = (p - 1u32) * (q - 1u32);
    let x = two.modpow(&two.modpow(&BigUint::from(t), &phi), &n);
    let key = derive_key(&x, &n);
    let ct: Vec<u8> = secret
        .iter()
        .zip(keystream(&key))
        .map(|(s, k)| s ^ k)
        .collect();
    let tag = hex_tag(&key, &ct);
    Puzzle {
        n,
        a: two,
        t,
        ct,
        tag,
        delay_seconds,
    }
}

/// Seals `secret` so that opening it takes about `delay_seconds` of nonstop
/// computation on this machine, or at least [`MIN_SQUARINGS`] squarings.
pub fn seal(secret: &[u8], delay_seconds: f64) -> Puzzle {
    assert!(delay_seconds.is_finite() && delay_seconds >= 0.0);
    let p = random_prime(PRIME_BITS);
    let q = random_prime(PRIME_BITS);
    let rate =
        squarings_per_second(&(&p * &q)).expect("a product of odd primes is a valid modulus");
    let t = ((delay_seconds * rate) as u64).max(MIN_SQUARINGS);
    seal_with_primes(secret, &p, &q, t, delay_seconds as u64)
}

/// Seals `secret` with a chosen number of squarings. Tests use this to keep
/// puzzles small; a real seal should measure the machine with [`seal`].
pub fn seal_with_squarings(secret: &[u8], t: u64, delay_seconds: u64) -> Puzzle {
    let p = random_prime(PRIME_BITS);
    let q = random_prime(PRIME_BITS);
    seal_with_primes(secret, &p, &q, t, delay_seconds)
}

/// Grinds through the squarings and returns the sealed text.
///
/// `progress` hears `(done, total)` after every batch, and stopping it there
/// cancels. A batch takes a few hundredths of a second, so that is quick.
pub fn solve(
    puzzle: &Puzzle,
    mut progress: impl FnMut(u64, u64) -> ControlFlow<()>,
) -> Result<String, SolveError> {
    let mut squarer = Squarer::new(&puzzle.n, &puzzle.a).ok_or(SolveError::Damaged)?;
    let mut done = 0;
    while done < puzzle.t {
        let step = PROGRESS_EVERY.min(puzzle.t - done);
        squarer.square_many(step);
        done += step;
        if progress(done, puzzle.t).is_break() {
            return Err(SolveError::Cancelled);
        }
    }
    let key = derive_key(&squarer.value(), &puzzle.n);
    // a plain comparison gives nothing away: reaching it already took the work
    if hex_tag(&key, &puzzle.ct) != puzzle.tag {
        return Err(SolveError::Damaged);
    }
    let plain: Vec<u8> = puzzle
        .ct
        .iter()
        .zip(keystream(&key))
        .map(|(c, k)| c ^ k)
        .collect();
    String::from_utf8(plain).map_err(|_| SolveError::Damaged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keep_going(_: u64, _: u64) -> ControlFlow<()> {
        ControlFlow::Continue(())
    }

    #[test]
    fn round_trip() {
        let puzzle = seal_with_squarings(b"1234", 5000, 0);
        assert_eq!(solve(&puzzle, keep_going).as_deref(), Ok("1234"));
    }

    #[test]
    fn detects_tampering() {
        let mut puzzle = seal_with_squarings(b"1234", 5000, 0);
        puzzle.ct[0] ^= 1;
        assert_eq!(solve(&puzzle, keep_going), Err(SolveError::Damaged));
    }

    #[test]
    fn reports_progress_in_order_and_finishes_at_the_total() {
        let puzzle = seal_with_squarings(b"42", 50_000, 0);
        let mut seen = Vec::new();
        solve(&puzzle, |done, total| {
            seen.push((done, total));
            ControlFlow::Continue(())
        })
        .unwrap();
        assert_eq!(seen, [(20_000, 50_000), (40_000, 50_000), (50_000, 50_000)]);
    }

    #[test]
    fn cancelling_stops_at_the_next_batch() {
        let puzzle = seal_with_squarings(b"42", 50_000, 0);
        let mut calls = 0;
        let result = solve(&puzzle, |_, _| {
            calls += 1;
            ControlFlow::Break(())
        });
        assert_eq!(result, Err(SolveError::Cancelled));
        assert_eq!(calls, 1);
    }

    #[test]
    fn modulus_size_and_sealing_floor() {
        let puzzle = seal(b"1234", 0.0001);
        assert!(puzzle.t >= MIN_SQUARINGS);
        assert_eq!(puzzle.delay_seconds, 0);
        assert_eq!(puzzle.a, BigUint::from(2u32));
        // two 1024-bit primes multiply to 2047 or 2048 bits
        assert!((2047..=2048).contains(&puzzle.n.bits()));
        assert_eq!(solve(&puzzle, keep_going).as_deref(), Ok("1234"));
    }

    #[test]
    fn a_modulus_that_cant_be_real_is_damage_not_a_crash() {
        let template = seal_with_squarings(b"1234", 1000, 0);
        for n in [0u32, 1, 2, 4, 1024] {
            let puzzle = Puzzle {
                n: BigUint::from(n),
                ..template.clone()
            };
            assert_eq!(
                solve(&puzzle, keep_going),
                Err(SolveError::Damaged),
                "n = {n}"
            );
        }
    }

    #[test]
    fn zero_squarings_keys_off_the_base_and_reports_no_progress() {
        // the original Locker never writes t = 0, but it would open such a file
        let (p, q) = (BigUint::from(1000003u32), BigUint::from(1000033u32));
        let puzzle = seal_with_primes(b"12", &p, &q, 0, 0);
        let mut calls = 0;
        let code = solve(&puzzle, |_, _| {
            calls += 1;
            ControlFlow::Continue(())
        });
        assert_eq!(code.as_deref(), Ok("12"));
        assert_eq!(calls, 0);
    }

    /// The Montgomery path must agree with plain `x * x % n` on every shape of
    /// modulus, including ones that force the top-carry branch.
    #[test]
    fn squarer_matches_plain_arithmetic() {
        let mut moduli = vec![
            BigUint::from(3u32),
            BigUint::from(15u32),
            BigUint::from(u64::MAX),
            (BigUint::one() << 64u32) + 1u32,
            (BigUint::one() << 128u32) - 1u32,
            (BigUint::one() << 2048u32) - 1u32,
            (BigUint::one() << 2048u32) - 159u32,
            (BigUint::one() << 2047u32) + 1u32,
        ];
        for bits in [1, 2, 3, 5, 8, 16, 31, 32, 33] {
            for _ in 0..4 {
                let mut n = random::bits(64 * bits);
                n.set_bit(0, true);
                n.set_bit(64 * bits - 1, true);
                moduli.push(n);
            }
        }
        for n in moduli {
            for start in [BigUint::from(2u32), &n - 1u32, random::big_below(&n)] {
                let mut squarer = Squarer::new(&n, &start).unwrap();
                let mut plain = &start % &n;
                for _ in 0..200 {
                    squarer.square();
                    plain = &plain * &plain % &n;
                }
                assert_eq!(squarer.value(), plain, "n = {n}, start = {start}");
            }
        }
    }

    #[test]
    fn start_values_above_the_modulus_are_reduced() {
        let n = BigUint::from(15u32);
        let mut squarer = Squarer::new(&n, &BigUint::from(32u32)).unwrap();
        assert_eq!(squarer.value(), BigUint::from(2u32));
        squarer.square();
        assert_eq!(squarer.value(), BigUint::from(4u32));
    }

    #[test]
    fn the_key_pads_to_the_width_of_n() {
        let n = (BigUint::one() << 2048u32) - 159u32;
        let small = BigUint::from(5u32);
        let mut padded = vec![0u8; 256];
        padded[255] = 5;
        assert_eq!(
            derive_key(&small, &n),
            <[u8; 32]>::from(Sha256::digest(&padded))
        );

        let n_2047 = (BigUint::one() << 2047u32) - 1u32;
        assert_eq!(
            derive_key(&small, &n_2047),
            <[u8; 32]>::from(Sha256::digest(&padded))
        );

        let n_small = BigUint::from(1000u32);
        assert_eq!(
            derive_key(&small, &n_small),
            <[u8; 32]>::from(Sha256::digest([0u8, 5]))
        );

        assert_eq!(
            derive_key(&BigUint::zero(), &n),
            <[u8; 32]>::from(Sha256::digest([0u8; 256]))
        );
    }

    #[test]
    fn primality_on_known_values() {
        for prime in [2u32, 3, 5, 97, 101, 7919, 1000003, 2147483647] {
            assert!(is_probable_prime(&BigUint::from(prime)), "{prime}");
        }
        // Carmichael numbers and products of primes beyond the trial division
        for composite in [561u32, 41041, 825265, 101 * 103, 10403, 1000003 * 2003] {
            assert!(!is_probable_prime(&BigUint::from(composite)), "{composite}");
        }
    }

    #[test]
    fn random_primes_have_the_requested_size() {
        for _ in 0..5 {
            let p = random_prime(64);
            assert_eq!(p.bits(), 64);
            assert!(p.bit(0));
            assert!(is_probable_prime(&p));
        }
    }

    #[test]
    fn speed_test_works_on_a_real_modulus_and_refuses_a_fake_one() {
        let puzzle = seal_with_squarings(b"1", 1000, 0);
        assert!(squarings_per_second(&puzzle.n).unwrap() > 1000.0);
        assert!(puzzle.estimated_seconds().unwrap() > 0.0);
        assert_eq!(squarings_per_second(&BigUint::from(4u32)), None);
    }

    #[test]
    #[should_panic(expected = "at most 32 bytes")]
    fn a_secret_longer_than_the_pad_is_refused() {
        seal_with_squarings(&[b'1'; 33], 1000, 0);
    }
}
