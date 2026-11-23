//! Writes a recovery file for checking it in the original Locker. See
//! `tests/fixtures/README.md`.
//!
//!     cargo run -p locker-core --example write_test_file -- <dir> <code> <squarings> <label>
//!
//! A squarings value of 0 seals for real, with a 0.5-second delay measured on
//! this machine. The folder is always the one given here, never `~/.locker`.

use std::path::PathBuf;
use std::process::ExitCode;

use locker_core::storage::Store;
use locker_core::timelock::seal_with_squarings;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, code, squarings, label] = args.as_slice() else {
        eprintln!("usage: write_test_file <dir> <code> <squarings> <label>");
        return ExitCode::FAILURE;
    };
    let squarings: u64 = squarings.parse().expect("squarings must be a whole number");
    let dir = PathBuf::from(dir);
    let store = Store {
        main: dir.clone(),
        legacy: dir,
        backup: None,
    };

    let written = if squarings == 0 {
        store.save_recovery(code, 0.5, label)
    } else {
        let puzzle = seal_with_squarings(code.as_bytes(), squarings, 60);
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        store.write_sealed(&puzzle, &stamp, 60.0, label)
    };
    match written {
        Ok(path) => {
            println!("{}", path.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
