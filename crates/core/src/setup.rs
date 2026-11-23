//! A guided setup that hands out one instruction at a time.
//!
//! The code is sealed into a pending recovery file before the first tap, so a
//! crash from then on can't lose it. The code never leaves this module: front
//! ends only see the next instruction, which never says whether a tap is real
//! or a decoy.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::sequence::{self, DigitSet, MAX_DIGITS, MIN_DIGITS, Op, SequenceError};
use crate::storage::{self, Store};
use crate::timelock;

/// Round 1 sets the passcode on the phone, and round 2 confirms it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Round {
    Set,
    Confirm,
}

/// A key on the phone's keypad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Digit(char),
    /// The key with this letter on it, in letter mode.
    Letter(char),
    Delete,
}

/// What the person did with the current screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Next,
    Mistyped,
    Quit,
}

/// What to show next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    /// The memory number is up, and no taps have started.
    Memory,
    /// Tap this key on the phone.
    Tap { round: Round, key: Key },
    /// After a mistyped tap: empty the phone's field, then a fresh sequence
    /// starts for the same round.
    ClearField { round: Round },
    /// Round 1 is done, and the phone asks for the passcode again.
    ReEnter,
    /// Both rounds are done, so the code is on the phone and saved at `path`.
    /// If `pending` is true, the file couldn't take its ordinary name and is
    /// still pending, which Home offers to finish.
    Finished { path: PathBuf, pending: bool },
    /// The person quit. `halfway` means round 1 was done. A quit in round 2
    /// keeps the pending file (`kept_file`), because the phone may already
    /// have accepted the code; otherwise the file is deleted.
    Stopped { halfway: bool, kept_file: bool },
}

/// How long the memory number may be, and how long it is unless asked
/// otherwise. Past nine digits nobody can hold it.
pub const MIN_MEMORY_DIGITS: usize = 3;
pub const MAX_MEMORY_DIGITS: usize = 9;
pub const DEFAULT_MEMORY_DIGITS: usize = 5;

#[derive(Debug)]
pub enum SetupError {
    /// The code length is outside 3-16 digits.
    InvalidDigits,
    /// The memory number's length is outside 3-9 digits.
    InvalidMemoryDigits,
    /// The delay isn't a positive number of seconds.
    InvalidDelay,
    /// Saving the sealed code failed, so the setup didn't start.
    Io(io::Error),
    /// A generated sequence failed its check. This is a bug.
    Sequence(SequenceError),
}

impl fmt::Display for SetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDigits => write!(f, "the code must be {MIN_DIGITS}-{MAX_DIGITS} digits"),
            Self::InvalidMemoryDigits => write!(
                f,
                "the memory number must be {MIN_MEMORY_DIGITS}-{MAX_MEMORY_DIGITS} digits"
            ),
            Self::InvalidDelay => f.write_str("the unlock time must be more than zero"),
            Self::Io(error) => write!(f, "could not save the sealed code ({error})"),
            Self::Sequence(error) => write!(f, "a tap sequence failed its check ({error})"),
        }
    }
}

impl std::error::Error for SetupError {}

impl From<io::Error> for SetupError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<SequenceError> for SetupError {
    fn from(error: SequenceError) -> Self {
        Self::Sequence(error)
    }
}

/// The choices made before a setup starts.
#[derive(Clone, Debug)]
pub struct Options {
    pub digits: usize,
    /// Target taps per round; the actual count lands within a few of this.
    pub target_taps: i64,
    pub label: String,
    /// How long a reveal should take, in seconds.
    pub delay: f64,
    /// Name each key by one of its letters instead of its digit. Codes and
    /// decoys then use only 2-9, the keys that have letters.
    pub letters: bool,
    /// How many digits the memory number has.
    pub memory_digits: usize,
}

enum State {
    Memory,
    Tapping {
        round: Round,
        keys: Vec<Key>,
        next: usize,
    },
    Clearing {
        round: Round,
    },
    ReEnter,
    Finished {
        path: PathBuf,
        pending: bool,
    },
    Stopped {
        halfway: bool,
        kept_file: bool,
    },
}

pub struct Setup {
    store: Store,
    code: String,
    target_taps: i64,
    letters: bool,
    memory_number: String,
    pending: PathBuf,
    state: State,
}

fn digit_set(letters: bool) -> DigitSet {
    if letters {
        DigitSet::Lettered
    } else {
        DigitSet::All
    }
}

impl Setup {
    /// Picks a random code, seals it with a delay measured on this machine,
    /// and saves it as pending. This takes about a second.
    pub fn begin(store: Store, options: &Options) -> Result<Self, SetupError> {
        if !(MIN_DIGITS..=MAX_DIGITS).contains(&options.digits) {
            return Err(SetupError::InvalidDigits);
        }
        if !(options.delay.is_finite() && options.delay > 0.0) {
            return Err(SetupError::InvalidDelay);
        }
        if !(MIN_MEMORY_DIGITS..=MAX_MEMORY_DIGITS).contains(&options.memory_digits) {
            return Err(SetupError::InvalidMemoryDigits);
        }
        let code = sequence::random_code(options.digits, digit_set(options.letters));
        let mut memory_number = sequence::random_code(options.memory_digits, DigitSet::All);
        while memory_number == code {
            memory_number = sequence::random_code(options.memory_digits, DigitSet::All);
        }
        let stamp = storage::now_stamp();
        let puzzle = timelock::seal(code.as_bytes(), options.delay);
        let pending = store.write_pending(&puzzle, &stamp, options.delay, &options.label)?;
        Ok(Self {
            store,
            code,
            target_taps: options.target_taps,
            letters: options.letters,
            memory_number,
            pending,
            state: State::Memory,
        })
    }

    /// The number to rehearse during the taps. It is never equal to the code.
    pub fn memory_number(&self) -> &str {
        &self.memory_number
    }

    /// Where the sealed code was saved before the first tap.
    pub fn pending_path(&self) -> &Path {
        &self.pending
    }

    /// True once the setup has finished or stopped.
    pub fn is_over(&self) -> bool {
        matches!(self.state, State::Finished { .. } | State::Stopped { .. })
    }

    /// The screen to show now. Asking again gives the same answer.
    pub fn screen(&self) -> Screen {
        match &self.state {
            State::Memory => Screen::Memory,
            State::Tapping { round, keys, next } => Screen::Tap {
                round: *round,
                key: keys[*next],
            },
            State::Clearing { round } => Screen::ClearField { round: *round },
            State::ReEnter => Screen::ReEnter,
            State::Finished { path, pending } => Screen::Finished {
                path: path.clone(),
                pending: *pending,
            },
            State::Stopped { halfway, kept_file } => Screen::Stopped {
                halfway: *halfway,
                kept_file: *kept_file,
            },
        }
    }

    /// Moves on after the person acts, and returns the next screen. Actions
    /// that mean nothing on the current screen leave it as it is.
    pub fn advance(&mut self, action: Action) -> Result<Screen, SetupError> {
        let next = match (&mut self.state, action) {
            (State::Memory, Action::Next) => Some(self.start_round(Round::Set)?),
            (State::Memory, Action::Quit) => Some(self.stop(false, false)),
            (State::Tapping { round, keys, next }, Action::Next) => {
                *next += 1;
                match (*next == keys.len(), *round) {
                    (false, _) => None,
                    (true, Round::Set) => Some(State::ReEnter),
                    (true, Round::Confirm) => Some(self.finish()),
                }
            }
            (State::Tapping { round, .. }, Action::Mistyped) => {
                Some(State::Clearing { round: *round })
            }
            (State::Clearing { round }, Action::Next) => {
                let round = *round;
                Some(self.start_round(round)?)
            }
            (State::Tapping { round, .. } | State::Clearing { round }, Action::Quit) => {
                let in_round_two = *round == Round::Confirm;
                Some(self.stop(in_round_two, in_round_two))
            }
            (State::ReEnter, Action::Next) => Some(self.start_round(Round::Confirm)?),
            // iOS discards a passcode whose confirmation never happens
            (State::ReEnter, Action::Quit) => Some(self.stop(true, false)),
            _ => None,
        };
        if let Some(state) = next {
            self.state = state;
        }
        Ok(self.screen())
    }

    /// Compares a guess with the memory number, ignoring surrounding spaces.
    pub fn memory_matches(&self, guess: &str) -> bool {
        guess.trim() == self.memory_number
    }

    /// Builds a fresh sequence for a round. In letter mode each tap gets its
    /// letter now, so asking for the same screen twice shows the same letter.
    fn start_round(&self, round: Round) -> Result<State, SetupError> {
        let ops = sequence::build_round(&self.code, self.target_taps, digit_set(self.letters))?;
        let keys = ops
            .iter()
            .map(|op| match *op {
                Op::Tap(digit) if self.letters => sequence::random_letter(digit)
                    .map(Key::Letter)
                    .ok_or(SetupError::Sequence(SequenceError::InvalidCode)),
                Op::Tap(digit) => Ok(Key::Digit(digit)),
                Op::Delete => Ok(Key::Delete),
            })
            .collect::<Result<Vec<Key>, SetupError>>()?;
        Ok(State::Tapping {
            round,
            keys,
            next: 0,
        })
    }

    fn finish(&self) -> State {
        match self.store.finish_pending(&self.pending) {
            Ok(path) => State::Finished {
                path,
                pending: false,
            },
            Err(_) => State::Finished {
                path: self.pending.clone(),
                pending: true,
            },
        }
    }

    fn stop(&self, halfway: bool, keep_file: bool) -> State {
        let kept_file = keep_file
            || match fs::remove_file(&self.pending) {
                Ok(()) => false,
                Err(error) => error.kind() != io::ErrorKind::NotFound,
            };
        State::Stopped { halfway, kept_file }
    }

    #[cfg(test)]
    fn code(&self) -> &str {
        &self.code
    }
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

    fn options(digits: usize) -> Options {
        Options {
            digits,
            target_taps: 28,
            label: "kids iPad".to_string(),
            delay: 0.0001,
            letters: false,
            memory_digits: DEFAULT_MEMORY_DIGITS,
        }
    }

    fn open(path: &Path) -> String {
        let puzzle = recovery::load(path).unwrap().puzzle().unwrap();
        solve(&puzzle, |_, _| ControlFlow::Continue(())).unwrap()
    }

    /// A model of the phone's passcode field. It only ever learns the code
    /// by doing what the screens say, the way a person would.
    #[derive(Default)]
    struct Phone {
        field: String,
        entered: Vec<String>,
    }

    impl Phone {
        fn tap(&mut self, key: Key, digits: usize) {
            match key {
                Key::Digit(d) => self.field.push(d),
                Key::Letter(letter) => self.field.push(
                    ('2'..='9')
                        .find(|digit| sequence::letters_on(*digit).contains(letter))
                        .expect("a letter that is on the keypad"),
                ),
                Key::Delete => {
                    self.field.pop().expect("DELETE on an empty field");
                }
            }
            if self.field.len() == digits {
                self.entered.push(std::mem::take(&mut self.field));
            }
        }
    }

    /// Follows tap screens until something else comes up.
    fn follow(setup: &mut Setup, phone: &mut Phone, digits: usize) -> Screen {
        loop {
            match setup.screen() {
                Screen::Tap { key, .. } => {
                    phone.tap(key, digits);
                    setup.advance(Action::Next).unwrap();
                }
                other => return other,
            }
        }
    }

    #[test]
    fn a_full_setup_puts_the_sealed_code_on_the_phone() {
        for digits in [3, 4, 6, 12, 16] {
            let (_root, store) = store();
            let mut setup = Setup::begin(store.clone(), &options(digits)).unwrap();
            // sealed before the first tap
            assert_eq!(store.pending_files(), [setup.pending_path().to_path_buf()]);
            assert_eq!(open(setup.pending_path()), setup.code());
            assert_eq!(setup.screen(), Screen::Memory);
            assert_ne!(setup.memory_number(), setup.code());

            let mut phone = Phone::default();
            setup.advance(Action::Next).unwrap();
            assert!(matches!(
                setup.screen(),
                Screen::Tap {
                    round: Round::Set,
                    ..
                }
            ));
            assert_eq!(follow(&mut setup, &mut phone, digits), Screen::ReEnter);
            assert_eq!(
                phone.entered.len(),
                1,
                "the field filled once, on the last tap"
            );

            setup.advance(Action::Next).unwrap();
            assert!(matches!(
                setup.screen(),
                Screen::Tap {
                    round: Round::Confirm,
                    ..
                }
            ));
            let Screen::Finished { path, pending } = follow(&mut setup, &mut phone, digits) else {
                panic!("the setup should have finished");
            };
            assert!(!pending);
            assert_eq!(phone.entered.len(), 2);
            assert_eq!(
                phone.entered[0], phone.entered[1],
                "both rounds enter the same code"
            );
            assert_eq!(
                open(&path),
                phone.entered[0],
                "the file opens to what the phone got"
            );
            assert_eq!(store.all_recoveries(), [path]);
            assert!(store.pending_files().is_empty());
            assert!(setup.is_over());
        }
    }

    #[test]
    fn a_mistyped_tap_restarts_the_round_once_the_field_is_clear() {
        let (_root, store) = store();
        let mut setup = Setup::begin(store, &options(4)).unwrap();
        let mut phone = Phone::default();
        setup.advance(Action::Next).unwrap();
        // a few taps, then the person reports a slip
        for _ in 0..3 {
            let Screen::Tap { key, .. } = setup.screen() else {
                unreachable!()
            };
            phone.tap(key, 4);
            setup.advance(Action::Next).unwrap();
        }
        assert_eq!(
            setup.advance(Action::Mistyped).unwrap(),
            Screen::ClearField { round: Round::Set }
        );
        // pressing x again or asking again changes nothing
        assert_eq!(
            setup.advance(Action::Mistyped).unwrap(),
            Screen::ClearField { round: Round::Set }
        );
        phone.field.clear();
        setup.advance(Action::Next).unwrap();
        assert!(matches!(
            setup.screen(),
            Screen::Tap {
                round: Round::Set,
                ..
            }
        ));

        assert_eq!(follow(&mut setup, &mut phone, 4), Screen::ReEnter);
        setup.advance(Action::Next).unwrap();
        // a slip in round 2 restarts round 2 only
        let Screen::Tap { key, .. } = setup.screen() else {
            unreachable!()
        };
        phone.tap(key, 4);
        setup.advance(Action::Next).unwrap();
        assert_eq!(
            setup.advance(Action::Mistyped).unwrap(),
            Screen::ClearField {
                round: Round::Confirm
            }
        );
        phone.field.clear();
        setup.advance(Action::Next).unwrap();
        let Screen::Finished { path, .. } = follow(&mut setup, &mut phone, 4) else {
            panic!("the setup should have finished");
        };
        assert_eq!(phone.entered.len(), 2);
        assert_eq!(phone.entered[0], phone.entered[1]);
        assert_eq!(open(&path), phone.entered[0]);
    }

    #[test]
    fn quitting_before_round_two_deletes_the_sealed_code() {
        // quit at the memory number, mid round 1, after a mistype, at re-enter
        let quit_after: [&[Action]; 4] = [
            &[],
            &[Action::Next, Action::Next, Action::Next],
            &[Action::Next, Action::Next, Action::Mistyped],
            &[],
        ];
        for (case, before) in quit_after.iter().enumerate() {
            let (_root, store) = store();
            let mut setup = Setup::begin(store.clone(), &options(4)).unwrap();
            for action in *before {
                setup.advance(*action).unwrap();
            }
            let halfway = case == 3;
            if halfway {
                let mut phone = Phone::default();
                setup.advance(Action::Next).unwrap();
                assert_eq!(follow(&mut setup, &mut phone, 4), Screen::ReEnter);
            }
            let screen = setup.advance(Action::Quit).unwrap();
            assert_eq!(
                screen,
                Screen::Stopped {
                    halfway,
                    kept_file: false
                },
                "case {case}"
            );
            assert!(store.pending_files().is_empty(), "case {case}");
            assert!(store.all_recoveries().is_empty(), "case {case}");
            assert!(setup.is_over());
        }
    }

    #[test]
    fn quitting_in_round_two_keeps_the_sealed_code() {
        for mistype_first in [false, true] {
            let (_root, store) = store();
            let mut setup = Setup::begin(store.clone(), &options(4)).unwrap();
            let mut phone = Phone::default();
            setup.advance(Action::Next).unwrap();
            follow(&mut setup, &mut phone, 4);
            setup.advance(Action::Next).unwrap();
            setup.advance(Action::Next).unwrap();
            if mistype_first {
                setup.advance(Action::Mistyped).unwrap();
            }
            assert_eq!(
                setup.advance(Action::Quit).unwrap(),
                Screen::Stopped {
                    halfway: true,
                    kept_file: true
                }
            );
            // the person may have done the last tap before quitting
            assert_eq!(store.pending_files(), [setup.pending_path().to_path_buf()]);
            assert_eq!(open(setup.pending_path()), setup.code());
        }
    }

    #[test]
    fn nothing_moves_once_the_setup_is_over() {
        let (_root, store) = store();
        let mut setup = Setup::begin(store, &options(4)).unwrap();
        let stopped = setup.advance(Action::Quit).unwrap();
        for action in [Action::Next, Action::Mistyped, Action::Quit] {
            assert_eq!(setup.advance(action).unwrap(), stopped);
        }
    }

    #[test]
    fn mistyped_means_nothing_before_the_taps_start() {
        let (_root, store) = store();
        let mut setup = Setup::begin(store, &options(4)).unwrap();
        assert_eq!(setup.advance(Action::Mistyped).unwrap(), Screen::Memory);
        let mut phone = Phone::default();
        setup.advance(Action::Next).unwrap();
        follow(&mut setup, &mut phone, 4);
        assert_eq!(setup.advance(Action::Mistyped).unwrap(), Screen::ReEnter);
    }

    #[test]
    fn a_taken_name_leaves_the_finished_code_pending_not_lost() {
        let (_root, store) = store();
        let mut setup = Setup::begin(store.clone(), &options(4)).unwrap();
        let pending = setup.pending_path().to_path_buf();
        let name = pending.file_name().unwrap().to_str().unwrap();
        fs::write(
            pending.with_file_name(name.trim_end_matches(".pending")),
            "taken",
        )
        .unwrap();

        let mut phone = Phone::default();
        setup.advance(Action::Next).unwrap();
        follow(&mut setup, &mut phone, 4);
        setup.advance(Action::Next).unwrap();
        let screen = follow(&mut setup, &mut phone, 4);
        assert_eq!(
            screen,
            Screen::Finished {
                path: pending.clone(),
                pending: true
            }
        );
        assert_eq!(open(&pending), phone.entered[0]);
    }

    #[test]
    fn bad_options_are_refused_before_anything_is_written() {
        let (_root, store) = store();
        for (digits, delay) in [(2, 60.0), (17, 60.0), (4, 0.0), (4, -1.0), (4, f64::NAN)] {
            let options = Options {
                delay,
                ..options(digits)
            };
            assert!(
                Setup::begin(store.clone(), &options).is_err(),
                "{digits} {delay}"
            );
        }
        for memory_digits in [0, 2, 10] {
            let options = Options {
                memory_digits,
                ..options(4)
            };
            assert!(
                Setup::begin(store.clone(), &options).is_err(),
                "memory number of {memory_digits}"
            );
        }
        assert!(!store.main.exists());
    }

    #[test]
    fn the_memory_number_has_the_length_asked_for() {
        for memory_digits in [3, 4, 5, 9] {
            let (_root, store) = store();
            let options = Options {
                memory_digits,
                ..options(4)
            };
            let setup = Setup::begin(store, &options).unwrap();
            assert_eq!(setup.memory_number().len(), memory_digits);
            assert_ne!(setup.memory_number(), setup.code());
        }
    }

    #[test]
    fn letter_mode_names_keys_by_letter_and_still_sets_the_code() {
        for digits in [3, 4, 6] {
            let (_root, store) = store();
            let options = Options {
                letters: true,
                ..options(digits)
            };
            let mut setup = Setup::begin(store.clone(), &options).unwrap();
            assert!(
                setup
                    .code()
                    .chars()
                    .all(|digit| DigitSet::Lettered.contains(digit))
            );
            let mut phone = Phone::default();
            setup.advance(Action::Next).unwrap();
            let mut letters_seen = 0;
            loop {
                match setup.screen() {
                    Screen::Tap { key, .. } => {
                        assert!(!matches!(key, Key::Digit(_)), "letter mode showed a digit");
                        letters_seen += matches!(key, Key::Letter(_)) as usize;
                        // asking again shows the same letter
                        assert_eq!(
                            setup.screen(),
                            Screen::Tap {
                                round: Round::Set,
                                key
                            }
                        );
                        phone.tap(key, digits);
                        setup.advance(Action::Next).unwrap();
                    }
                    Screen::ReEnter => break,
                    other => panic!("unexpected {other:?}"),
                }
            }
            assert!(letters_seen >= digits);
            setup.advance(Action::Next).unwrap();
            let Screen::Finished { path, .. } = follow(&mut setup, &mut phone, digits) else {
                panic!("the setup should have finished");
            };
            assert_eq!(phone.entered[0], phone.entered[1]);
            assert_eq!(open(&path), phone.entered[0]);
        }
    }

    #[test]
    fn the_memory_check_ignores_surrounding_spaces() {
        let (_root, store) = store();
        let setup = Setup::begin(store, &options(6)).unwrap();
        let number = setup.memory_number().to_string();
        assert_eq!(number.len(), DEFAULT_MEMORY_DIGITS);
        assert!(setup.memory_matches(&format!(" {number} ")));
        assert!(!setup.memory_matches("x"));
    }
}
