//! The operating system's secure random source, the one Python's `secrets`
//! uses. Codes, decoys and primes all come from here.

use num_bigint::BigUint;

/// Fills `buf` from the OS. A failure panics, because nothing here may carry
/// on with weaker randomness.
pub(crate) fn fill(buf: &mut [u8]) {
    getrandom::fill(buf).expect("the operating system's random source failed");
}

/// A uniform integer in `0..bound`.
pub(crate) fn below(bound: u64) -> u64 {
    assert!(bound > 0);
    // reject the partial range at the top, which would favor small results
    let excess = (u64::MAX % bound + 1) % bound;
    loop {
        let mut bytes = [0; 8];
        fill(&mut bytes);
        let value = u64::from_le_bytes(bytes);
        if value <= u64::MAX - excess {
            return value % bound;
        }
    }
}

/// A fair coin flip.
pub(crate) fn coin() -> bool {
    let mut byte = [0];
    fill(&mut byte);
    byte[0] & 1 == 1
}

/// An integer with `bits` random bits, like `secrets.randbits`.
pub(crate) fn bits(bits: u64) -> BigUint {
    let mut bytes = vec![0; bits.div_ceil(8) as usize];
    fill(&mut bytes);
    if !bits.is_multiple_of(8) {
        bytes[0] &= (1 << (bits % 8)) - 1;
    }
    BigUint::from_bytes_be(&bytes)
}

/// A uniform integer in `0..bound`, for any size of bound.
pub(crate) fn big_below(bound: &BigUint) -> BigUint {
    assert!(bound.bits() > 0);
    loop {
        let value = bits(bound.bits());
        if &value < bound {
            return value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_stays_in_range_and_reaches_every_value() {
        let mut seen = [false; 10];
        for _ in 0..1000 {
            seen[below(10) as usize] = true;
        }
        assert!(seen.iter().all(|s| *s));
        assert_eq!(below(1), 0);
    }

    #[test]
    fn bits_never_exceeds_the_requested_size() {
        for size in [1, 7, 8, 9, 63, 64, 65, 1024] {
            for _ in 0..50 {
                assert!(bits(size).bits() <= size);
            }
        }
    }

    #[test]
    fn big_below_stays_in_range() {
        let bound = BigUint::from(1000u32);
        for _ in 0..1000 {
            assert!(big_below(&bound) < bound);
        }
    }
}
