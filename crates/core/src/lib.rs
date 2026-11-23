//! The logic behind Locker: the tap sequence, the time-lock, the recovery
//! file format and where those files live. The command line and the app are
//! thin front ends over this crate.
//!
//! The original Locker, a Python program (`locker.py`, its version 1.0.0), is
//! the reference for all of it: files it wrote must keep opening here, and
//! files written here must open in it. See `tests/fixtures/README.md` for how
//! that is checked. This crate's own version numbers have nothing to do with
//! that one.

pub mod recovery;
pub mod sequence;
pub mod setup;
pub mod storage;
pub mod text;
pub mod timelock;

mod base64;
mod random;
