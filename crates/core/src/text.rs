//! Text the person brings themselves, locked away as it is.
//!
//! Nothing goes on a phone here, so there are no taps: the text is sealed and
//! saved in one step, as a recovery file like any other. The person has seen
//! this text, so the lock only helps if no other copy of it is left.

use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::storage::Store;
use crate::timelock::MAX_SECRET_BYTES;

/// The longest text that can be locked, in UTF-8 bytes. A recovery file holds
/// no more than this.
pub const MAX_TEXT_BYTES: usize = MAX_SECRET_BYTES;

#[derive(Debug)]
pub enum TextError {
    /// Nothing but spaces, or nothing at all.
    Empty,
    /// More than [`MAX_TEXT_BYTES`] bytes.
    TooLong,
    /// A space at the start or the end, which nobody could see on a reveal.
    SpaceAtAnEnd,
    /// A line break, a tab or another control character.
    ControlCharacter,
    /// The delay isn't a positive number of seconds.
    InvalidDelay,
    Io(io::Error),
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("there is no text to lock"),
            Self::TooLong => write!(f, "the text is longer than {MAX_TEXT_BYTES} bytes"),
            Self::SpaceAtAnEnd => f.write_str("the text starts or ends with a space"),
            Self::ControlCharacter => f.write_str("the text has a line break or a tab in it"),
            Self::InvalidDelay => f.write_str("the unlock time must be more than zero"),
            Self::Io(error) => write!(f, "could not save the sealed text ({error})"),
        }
    }
}

impl std::error::Error for TextError {}

impl From<io::Error> for TextError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Checks that `text` can be locked exactly as it is. Nothing is trimmed or
/// changed on the way in, because a reveal must give back what was typed. So
/// what a person couldn't see or type again is refused instead.
pub fn check(text: &str) -> Result<(), TextError> {
    if text.trim().is_empty() {
        return Err(TextError::Empty);
    }
    if text.chars().any(char::is_control) {
        return Err(TextError::ControlCharacter);
    }
    if text.trim() != text {
        return Err(TextError::SpaceAtAnEnd);
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(TextError::TooLong);
    }
    Ok(())
}

/// Seals `text` behind a time-lock measured on this machine and saves it.
/// This takes about a second.
pub fn lock(store: &Store, text: &str, label: &str, delay: f64) -> Result<PathBuf, TextError> {
    check(text)?;
    if !(delay.is_finite() && delay > 0.0) {
        return Err(TextError::InvalidDelay);
    }
    Ok(store.save_recovery(text, delay, label)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recovery;
    use crate::timelock::solve;
    use std::ops::ControlFlow;
    use tempfile::TempDir;

    fn store() -> (TempDir, Store) {
        let root = TempDir::new().unwrap();
        let store = Store {
            main: root.path().join("main"),
            legacy: root.path().join("legacy"),
            backup: None,
        };
        (root, store)
    }

    #[test]
    fn locked_text_comes_back_exactly_as_typed() {
        let longest = "x".repeat(MAX_TEXT_BYTES);
        for text in [
            "hunter2",
            "correct horse battery staple",
            "two  spaces, kept",
            "pässwörd ✓ 鍵",
            "0042",
            longest.as_str(),
        ] {
            let (_root, store) = store();
            let path = lock(&store, text, "router", 0.0001).unwrap();
            assert!(store.pending_files().is_empty());

            let data = recovery::load(&path).unwrap();
            assert_eq!(data.label().as_deref(), Some("router"));
            let opened = solve(&data.puzzle().unwrap(), |_, _| ControlFlow::Continue(()));
            assert_eq!(opened.as_deref(), Ok(text));
            assert_eq!(store.all_recoveries(), [path]);
        }
    }

    #[test]
    fn the_limit_counts_bytes() {
        assert!(check(&"x".repeat(32)).is_ok());
        assert!(matches!(check(&"x".repeat(33)), Err(TextError::TooLong)));
        // é is two bytes
        assert!(check(&"é".repeat(16)).is_ok());
        assert!(matches!(check(&"é".repeat(17)), Err(TextError::TooLong)));
    }

    #[test]
    fn what_nobody_could_see_or_retype_is_refused() {
        for text in ["", " ", "   ", "\n", "\t "] {
            assert!(matches!(check(text), Err(TextError::Empty)), "{text:?}");
        }
        for text in [" a", "a ", " a ", "a\u{a0}", "\u{3000}a"] {
            assert!(
                matches!(check(text), Err(TextError::SpaceAtAnEnd)),
                "{text:?}"
            );
        }
        for text in ["a\nb", "a\tb", "a\r\n", "a\u{0}b", "\u{7f}a"] {
            assert!(
                matches!(check(text), Err(TextError::ControlCharacter)),
                "{text:?}"
            );
        }
    }

    #[test]
    fn bad_text_and_bad_delays_are_refused_before_anything_is_written() {
        let (_root, store) = store();
        let too_long = "x".repeat(MAX_TEXT_BYTES + 1);
        for text in ["", " padded", "line\nbreak", too_long.as_str()] {
            assert!(lock(&store, text, "x", 60.0).is_err(), "{text:?}");
        }
        for delay in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                matches!(
                    lock(&store, "fine", "x", delay),
                    Err(TextError::InvalidDelay)
                ),
                "{delay}"
            );
        }
        assert!(!store.main.exists());
    }
}
