//! Tap sequences: long streams of digits and deletes that leave the code
//! behind. Decoy taps and real ones look identical, so the person following
//! them never learns the code.
//!
//! No two taps in a row ever share a digit, because a doubled digit is easy
//! to remember. Codes never contain one, and a decoy never repeats the tap
//! just before it.

use std::fmt;

use crate::random;

pub const MIN_DIGITS: usize = 3;
pub const MAX_DIGITS: usize = 16;
pub const DEFAULT_DIGITS: usize = 4;
pub const DEFAULT_TAPS: i64 = 28;

/// Rounds land within this many taps of the target.
pub const TAP_JITTER: i64 = 4;

/// One instruction for the keypad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Tap(char),
    Delete,
}

/// The digits a code and its decoys come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DigitSet {
    /// 0-9.
    #[default]
    All,
    /// 2-9, the keys with letters under them. Letter mode needs this, since
    /// 0 and 1 would stand out as the only plain digits.
    Lettered,
}

impl DigitSet {
    fn digits(self) -> &'static [char] {
        match self {
            Self::All => &['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'],
            Self::Lettered => &['2', '3', '4', '5', '6', '7', '8', '9'],
        }
    }

    pub fn contains(self, digit: char) -> bool {
        self.digits().contains(&digit)
    }
}

/// The letters a phone keypad shows under a digit, or "" for 0 and 1.
pub fn letters_on(digit: char) -> &'static str {
    match digit {
        '2' => "ABC",
        '3' => "DEF",
        '4' => "GHI",
        '5' => "JKL",
        '6' => "MNO",
        '7' => "PQRS",
        '8' => "TUV",
        '9' => "WXYZ",
        _ => "",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceError {
    /// The code isn't 3-16 digits from its set with no digit twice in a row.
    InvalidCode,
    /// DELETE on an empty field.
    DeleteOnEmpty,
    /// The field filled up before the final tap, which submits it on iOS.
    FilledEarly,
    /// The sequence doesn't leave the code behind.
    WrongResult,
    /// Two taps in a row share a digit.
    RepeatedDigit,
}

impl fmt::Display for SequenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidCode => "the code must be 3-16 digits with no digit twice in a row",
            Self::DeleteOnEmpty => "DELETE on an empty field",
            Self::FilledEarly => "the field filled before the final tap",
            Self::WrongResult => "the sequence does not produce the code",
            Self::RepeatedDigit => "two taps in a row share a digit",
        })
    }
}

impl std::error::Error for SequenceError {}

/// A uniform digit from the set, other than `avoid`.
fn random_digit(set: DigitSet, avoid: Option<char>) -> char {
    let choices: Vec<char> = set
        .digits()
        .iter()
        .copied()
        .filter(|digit| Some(*digit) != avoid)
        .collect();
    choices[random::below(choices.len() as u64) as usize]
}

/// A random code with no digit twice in a row.
pub fn random_code(digits: usize, set: DigitSet) -> String {
    let mut code = String::with_capacity(digits);
    let mut last = None;
    for _ in 0..digits {
        let digit = random_digit(set, last);
        code.push(digit);
        last = Some(digit);
    }
    code
}

/// A random letter from the keypad key for `digit`, or None for 0 and 1.
pub fn random_letter(digit: char) -> Option<char> {
    let letters: Vec<char> = letters_on(digit).chars().collect();
    if letters.is_empty() {
        return None;
    }
    Some(letters[random::below(letters.len() as u64) as usize])
}

fn last_tap(ops: &[Op]) -> Option<char> {
    match ops.last() {
        Some(Op::Tap(digit)) => Some(*digit),
        _ => None,
    }
}

/// Emits `pairs` decoy digits and their deletes as a random walk.
///
/// Nesting depth never exceeds `cap`, and the walk always returns to the level
/// it started at, so every decoy is deleted again.
fn decoy_run(ops: &mut Vec<Op>, pairs: u64, cap: usize, set: DigitSet) {
    let mut depth = 0;
    let mut pushes_left = pairs;
    while pushes_left > 0 || depth > 0 {
        let can_push = pushes_left > 0 && depth < cap;
        if can_push && (depth == 0 || random::coin()) {
            let digit = random_digit(set, last_tap(ops));
            ops.push(Op::Tap(digit));
            pushes_left -= 1;
            depth += 1;
        } else {
            ops.push(Op::Delete);
            depth -= 1;
        }
    }
}

/// Builds one sequence whose surviving digits equal `code`.
///
/// The field never reaches the code's length until the very last tap, because
/// a full field submits itself on iOS. The total is random within
/// `target_taps` ± [`TAP_JITTER`], so consecutive rounds don't even have the
/// same length. Every sequence is checked with [`simulate`] and
/// [`check_no_repeats`] before use.
pub fn build_round(code: &str, target_taps: i64, set: DigitSet) -> Result<Vec<Op>, SequenceError> {
    let digits: Vec<char> = code.chars().collect();
    let n = digits.len();
    if !(MIN_DIGITS..=MAX_DIGITS).contains(&n)
        || !digits.iter().all(|digit| set.contains(*digit))
        || digits.windows(2).any(|pair| pair[0] == pair[1])
    {
        return Err(SequenceError::InvalidCode);
    }

    let len = n as i64;
    let min_taps = target_taps.saturating_sub(TAP_JITTER).max(len);
    let max_taps = target_taps.saturating_add(TAP_JITTER).max(3 * len);
    // beyond the real digits, taps come in decoy push/delete pairs, so the
    // total must share the code length's parity
    let totals: Vec<i64> = (min_taps..=max_taps)
        .filter(|t| (t - len) % 2 == 0)
        .collect();
    let total = totals[random::below(totals.len() as u64) as usize];

    // scatter the decoy pairs across the gaps before each real digit (no gap
    // after the last one: its tap must be the final, submitting tap, and decoy
    // headroom there is zero anyway)
    let mut gaps = vec![0; n - 1];
    for _ in 0..(total - len) / 2 {
        gaps[random::below(n as u64 - 1) as usize] += 1;
    }

    let mut ops = Vec::with_capacity(total as usize);
    for (i, &digit) in digits.iter().enumerate() {
        if i < n - 1 && gaps[i] > 0 {
            decoy_run(&mut ops, gaps[i], n - 1 - i, set);
        }
        ops.push(Op::Tap(digit));
    }

    if simulate(&ops, n)? != code {
        return Err(SequenceError::WrongResult);
    }
    check_no_repeats(&ops)?;
    Ok(ops)
}

/// Fails if two taps in a row share a digit.
pub fn check_no_repeats(ops: &[Op]) -> Result<(), SequenceError> {
    let repeated = ops
        .windows(2)
        .any(|pair| matches!(pair, [Op::Tap(a), Op::Tap(b)] if a == b));
    if repeated {
        Err(SequenceError::RepeatedDigit)
    } else {
        Ok(())
    }
}

/// Runs ops through a model of the passcode field and returns what's left.
///
/// Fails if the sequence is invalid: a delete on an empty field, or a full
/// field (which submits itself on iOS) anywhere but the final op.
pub fn simulate(ops: &[Op], digits: usize) -> Result<String, SequenceError> {
    let mut field = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        match op {
            Op::Tap(digit) => field.push(*digit),
            Op::Delete => {
                field.pop().ok_or(SequenceError::DeleteOnEmpty)?;
            }
        }
        if field.len() == digits && i != ops.len() - 1 {
            return Err(SequenceError::FilledEarly);
        }
    }
    Ok(field.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::Op::{Delete, Tap};
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn rounds_keep_the_invariants() {
        for digits in [3, 4, 6, 12, 16] {
            for taps in [0, 10, 28, 60] {
                for _ in 0..50 {
                    let code = random_code(digits, DigitSet::All);
                    let ops = build_round(&code, taps, DigitSet::All).unwrap();
                    assert_eq!(simulate(&ops, digits).unwrap(), code);
                    assert_eq!(*ops.last().unwrap(), Tap(code.chars().last().unwrap()));
                    assert!(ops.len() >= digits);
                }
            }
        }
    }

    #[test]
    fn round_lengths_vary_within_the_jitter() {
        let lengths: BTreeSet<usize> = (0..200)
            .map(|_| build_round("1234", 28, DigitSet::All).unwrap().len())
            .collect();
        assert!(lengths.len() > 1);
        assert!(*lengths.first().unwrap() >= (28 - TAP_JITTER) as usize);
        assert!(*lengths.last().unwrap() <= (28 + TAP_JITTER) as usize);
    }

    #[test]
    fn rounds_differ_for_the_same_code() {
        let code = random_code(4, DigitSet::All);
        assert_ne!(
            build_round(&code, 28, DigitSet::All).unwrap(),
            build_round(&code, 28, DigitSet::All).unwrap()
        );
    }

    #[test]
    fn nested_decoys_are_common() {
        let has_double_decoy = |ops: &[Op]| {
            ops.windows(4)
                .any(|w| matches!(w, [Tap(_), Tap(_), Delete, Delete]))
        };
        let hits = (0..100)
            .filter(|_| {
                has_double_decoy(
                    &build_round(&random_code(4, DigitSet::All), 28, DigitSet::All).unwrap(),
                )
            })
            .count();
        assert!(hits >= 60, "only {hits} of 100 rounds nested a decoy");
    }

    #[test]
    fn simulate_rejects_delete_on_empty() {
        assert_eq!(
            simulate(&[Delete, Tap('1')], 4),
            Err(SequenceError::DeleteOnEmpty)
        );
    }

    #[test]
    fn simulate_rejects_an_early_full_field() {
        let ops = [Tap('1'), Tap('2'), Tap('3'), Tap('4'), Delete, Tap('5')];
        assert_eq!(simulate(&ops, 4), Err(SequenceError::FilledEarly));
    }

    #[test]
    fn totals_share_the_codes_parity_and_cover_the_range() {
        let lengths: BTreeSet<usize> = (0..500)
            .map(|_| build_round("12345", 28, DigitSet::All).unwrap().len())
            .collect();
        assert_eq!(lengths, BTreeSet::from([25, 27, 29, 31]));
    }

    #[test]
    fn a_long_code_can_exceed_the_target_up_to_three_times_its_length() {
        let lengths: BTreeSet<usize> = (0..2000)
            .map(|_| {
                build_round(&"12".repeat(6), 28, DigitSet::All)
                    .unwrap()
                    .len()
            })
            .collect();
        assert_eq!(*lengths.first().unwrap(), 24);
        assert_eq!(*lengths.last().unwrap(), 36);
    }

    #[test]
    fn a_low_target_still_uses_decoys() {
        let lengths: BTreeSet<usize> = (0..500)
            .map(|_| build_round("1234", 0, DigitSet::All).unwrap().len())
            .collect();
        assert_eq!(lengths, BTreeSet::from([4, 6, 8, 10, 12]));
    }

    #[test]
    fn invalid_codes_are_refused() {
        for code in [
            "",
            "12",
            "12345678901234567",
            "12a4",
            "١٢٣٤",
            "1123",
            "1233",
        ] {
            let result = build_round(code, 28, DigitSet::All);
            assert_eq!(result, Err(SequenceError::InvalidCode), "{code:?}");
        }
        for code in ["1234", "2340", "2345"] {
            let result = build_round(code, 28, DigitSet::Lettered);
            assert_eq!(result.is_ok(), code == "2345", "{code:?}");
        }
    }

    #[test]
    fn totals_and_decoy_digits_are_evenly_spread() {
        let mut totals: BTreeMap<usize, u32> = BTreeMap::new();
        let mut decoys = [0u32; 10];
        for _ in 0..5000 {
            let ops = build_round("1234", 28, DigitSet::All).unwrap();
            *totals.entry(ops.len()).or_default() += 1;
            // every digit that gets deleted was a decoy
            let mut field = Vec::new();
            for op in &ops {
                match op {
                    Tap(digit) => field.push(*digit),
                    Delete => {
                        let decoy = field.pop().unwrap();
                        decoys[decoy.to_digit(10).unwrap() as usize] += 1;
                    }
                }
            }
        }
        assert_eq!(totals.len(), 5);
        for (total, count) in totals {
            assert!(
                (800..=1200).contains(&count),
                "{total} taps came up {count} times"
            );
        }
        let mean = decoys.iter().sum::<u32>() / 10;
        for (digit, count) in decoys.iter().enumerate() {
            assert!(
                count.abs_diff(mean) < mean / 4,
                "decoy {digit} came up {count} times"
            );
        }
    }

    #[test]
    fn codes_never_hold_a_digit_twice_in_a_row() {
        for set in [DigitSet::All, DigitSet::Lettered] {
            let mut seen = BTreeSet::new();
            for digits in MIN_DIGITS..=MAX_DIGITS {
                for _ in 0..200 {
                    let code: Vec<char> = random_code(digits, set).chars().collect();
                    assert_eq!(code.len(), digits);
                    assert!(code.windows(2).all(|pair| pair[0] != pair[1]), "{code:?}");
                    assert!(code.iter().all(|digit| set.contains(*digit)), "{code:?}");
                    seen.extend(code);
                }
            }
            assert_eq!(
                seen.len(),
                set.digits().len(),
                "every digit of the set appears"
            );
        }
    }

    #[test]
    fn no_two_taps_in_a_row_share_a_digit() {
        for set in [DigitSet::All, DigitSet::Lettered] {
            for digits in [3, 4, 6, 12, 16] {
                for taps in [0, 10, 28, 60] {
                    for _ in 0..50 {
                        let code = random_code(digits, set);
                        let ops = build_round(&code, taps, set).unwrap();
                        assert_eq!(check_no_repeats(&ops), Ok(()));
                        assert_eq!(simulate(&ops, digits).unwrap(), code);
                        for op in &ops {
                            if let Tap(digit) = op {
                                assert!(set.contains(*digit), "{digit} in {set:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_repeat_check_catches_doubles_but_not_ones_split_by_a_delete() {
        assert_eq!(
            check_no_repeats(&[Tap('5'), Tap('5')]),
            Err(SequenceError::RepeatedDigit)
        );
        assert_eq!(check_no_repeats(&[Tap('5'), Delete, Tap('5')]), Ok(()));
        assert_eq!(
            check_no_repeats(&[Tap('5'), Tap('6'), Delete, Delete]),
            Ok(())
        );
    }

    #[test]
    fn every_lettered_digit_has_letters_and_only_those() {
        for digit in '0'..='9' {
            let letters = letters_on(digit);
            assert_eq!(
                !letters.is_empty(),
                DigitSet::Lettered.contains(digit),
                "{digit}"
            );
            for _ in 0..40 {
                match random_letter(digit) {
                    Some(letter) => assert!(letters.contains(letter), "{digit} {letter}"),
                    None => assert!(letters.is_empty()),
                }
            }
        }
        let all: String = ('2'..='9').map(letters_on).collect();
        assert_eq!(all, "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    }
}
