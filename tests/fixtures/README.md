# Test files from the original Locker

The original Locker is the Python program this project began as, at its version
1.0.0. Every saved code depends on this program opening what that one wrote. These
files are that promise as plain data, so this repo needs no Python to test it.

**Never add a real recovery file here.** Every code in this folder is a throwaway
test value.

| File | What it holds |
| --- | --- |
| `recovery-*.txt` | Recovery files, keeping their exact bytes through `.gitattributes` |
| `manifest.json` | What each file should give: its code, label and time-lock details |
| `vectors.json` | Inputs and outputs of the original's text functions, such as slugs and delays |

`manifest.json` names the exact `locker.py` by SHA-256. `crates/core/tests/fixtures.rs`
opens every file, and the writer must reproduce the ordinary files byte for byte.

Most files came from the original's own `save_recovery`, with the speed test replaced
so that `t` stays small. One came from a real `locker set` run, and its code was found
by following the printed tap instructions on a model of the keypad. The variants
change those files' bytes on purpose, and the original confirmed how it reads each one.

The legacy files are hand-built, because the original reads that format but no longer
writes it. They contain what its reader looks for, which may be less than old files
held.

## Checking the other direction

The original Locker must also open files written here. That check needs the old
program, so it runs outside this repo:

1. Point `HOME` and `USERPROFILE` at an empty temp folder, set `LOCKER_BACKUP_DIR`
   inside it, and clear `OneDrive`, `OneDriveConsumer` and `OneDriveCommercial`. Then
   the old program can't see any real recovery files.
2. Write a test file into that temp folder:
   ```
   cargo run -p locker-core --example write_test_file -- <folder> <code> <squarings> <label>
   ```
3. Run `locker reveal <file>` with the old program, type `reveal`, and check that it
   prints the code. Answer `n` when it offers to delete the file.
