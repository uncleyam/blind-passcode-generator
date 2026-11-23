//! Where recovery files live.
//!
//! Everything goes through a [`Store`] that names its folders, so tests and
//! tools can never reach someone's real files by accident. Only
//! [`Store::for_current_user`] looks at the real home folder.

use std::collections::{BTreeMap, HashSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{NaiveDateTime, TimeDelta};

use crate::recovery::{self, normalize_label};
use crate::timelock::{self, Puzzle};

const NAME_PREFIX: &str = "recovery-";
const NAME_SUFFIX: &str = ".txt";
/// Marks a sealed code whose setup hasn't finished. The original Locker
/// ignores these files, because their names don't end in `.txt`.
const PENDING_SUFFIX: &str = ".pending";

const STAMP_FORMAT: &str = "%Y%m%d-%H%M%S";

/// A deleted code waits in the bin this many days before it's gone for good.
pub const BIN_DAYS: i64 = 30;
/// The bin: a folder inside the folder a file was deleted from.
const BIN_FOLDER: &str = "deleted";
/// In the bin a file's name gains this and the stamp of when it was deleted.
/// The original Locker and the listings here skip such a name, as they do a
/// pending one.
const DELETED_MARK: &str = ".deleted-";
/// Beside a deleted file in the main bin: its code in plain text, kept only
/// when the code had already been revealed.
const CODE_SUFFIX: &str = ".code";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    /// Where new files go.
    pub main: PathBuf,
    /// An older location that is read but never written.
    pub legacy: PathBuf,
    /// A synced folder to offer backups in, if there is one.
    pub backup: Option<PathBuf>,
}

/// Finds a synced folder to offer backups in.
///
/// `LOCKER_BACKUP_DIR` always wins and is used verbatim. Otherwise this looks
/// for OneDrive (Windows env vars), then the conventional cloud-sync folders
/// on macOS and Linux, and uses an Apps/Locker subfolder there (the app-folder
/// convention OneDrive and Dropbox use themselves).
pub fn find_backup_dir(home: &Path, env_var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |name: &str| env_var(name).filter(|value| !value.is_empty());
    if let Some(dir) = set("LOCKER_BACKUP_DIR") {
        return Some(expand_home(&dir, home));
    }
    for name in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
        if let Some(root) = set(name).map(PathBuf::from).filter(|root| root.is_dir()) {
            return Some(root.join("Apps").join("Locker"));
        }
    }
    let mut candidates = Vec::new();
    // macOS providers
    if let Ok(entries) = fs::read_dir(home.join("Library").join("CloudStorage")) {
        let mut providers: Vec<PathBuf> = entries
            .filter_map(|entry| Some(entry.ok()?.path()))
            .collect();
        providers.sort();
        candidates.extend(providers);
    }
    candidates.extend([
        home.join("Library")
            .join("Mobile Documents")
            .join("com~apple~CloudDocs"),
        home.join("Dropbox"),
        home.join("Nextcloud"),
        home.join("OneDrive"), // the common Linux OneDrive client's default
        home.join("Sync"),     // Syncthing's default
    ]);
    candidates
        .into_iter()
        .find(|candidate| candidate.is_dir())
        .map(|candidate| candidate.join("Apps").join("Locker"))
}

/// Expands a leading `~` or `~/`. `~user` forms are left alone.
fn expand_home(path: &OsString, home: &Path) -> PathBuf {
    let path = PathBuf::from(path);
    match path.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => path,
    }
}

fn has_affixes(name: &str, suffix: &str) -> bool {
    // the old glob ignored case on Windows, and only there
    let matches = |name: &str| name.starts_with(NAME_PREFIX) && name.ends_with(suffix);
    if cfg!(windows) {
        matches(&name.to_ascii_lowercase())
    } else {
        matches(name)
    }
}

fn is_recovery_name(name: &str) -> bool {
    has_affixes(name, NAME_SUFFIX)
}

fn is_pending_name(name: &str) -> bool {
    has_affixes(name, &format!("{NAME_SUFFIX}{PENDING_SUFFIX}"))
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// The time on this computer's clock, which is what stamps are written in.
pub fn local_now() -> NaiveDateTime {
    chrono::Local::now().naive_local()
}

/// The stamp recovery files carry: local time, `YYYYMMDD-HHMMSS`.
pub(crate) fn now_stamp() -> String {
    local_now().format(STAMP_FORMAT).to_string()
}

/// Splits a bin name into the file's old name and when it was deleted. None
/// for anything that isn't a deleted recovery file.
fn parse_bin_name(name: &str) -> Option<(&str, NaiveDateTime)> {
    let (original, stamp) = name.rsplit_once(DELETED_MARK)?;
    if !is_recovery_name(original) || stamp.len() != 15 {
        return None;
    }
    let at = NaiveDateTime::parse_from_str(stamp, STAMP_FORMAT).ok()?;
    Some((original, at))
}

/// A deleted code waiting in the bin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deleted {
    /// The bin's name for it: the file's old name, then when it was deleted.
    pub name: String,
    /// Every copy, each in the bin of the folder it was deleted from.
    pub copies: Vec<PathBuf>,
    /// When it was deleted, in local time.
    pub at: NaiveDateTime,
    /// The code in plain text, if it had been revealed before the delete.
    pub code: Option<String>,
}

impl Deleted {
    /// Days until it's removed for good, counting a part of a day as one.
    /// Zero or less means its time is up.
    pub fn days_left(&self, now: NaiveDateTime) -> i64 {
        let seconds = (self.at + TimeDelta::days(BIN_DAYS) - now).num_seconds();
        // a clock set back mustn't promise more than the full wait
        (seconds + 86_399).div_euclid(86_400).min(BIN_DAYS)
    }
}

/// The folder stays private on macOS and Linux; Windows has no such mode.
fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Writes a new file so it either appears complete or not at all, and never
/// replaces an existing one, which may be holding a live code.
fn write_new_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", path.display()),
        ));
    }
    // a temporary name that the recovery-*.txt pattern doesn't match
    let temporary = path.with_file_name(format!(".{}.tmp", name_of(path)));
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()
    })();
    let result = written.and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// Moves a file to a new name, and never onto an existing file.
fn move_new(from: &Path, to: &Path) -> io::Result<()> {
    if to.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", to.display()),
        ));
    }
    fs::rename(from, to)
}

/// Removes a file. One that's already gone counts as removed.
fn remove_if_there(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

impl Store {
    /// The folders the original Locker used, for the person running this
    /// program.
    pub fn for_current_user() -> Option<Self> {
        let home = env::home_dir()?;
        Some(Self {
            main: home.join(".locker"),
            legacy: home.join(".password-locker"),
            backup: find_backup_dir(&home, |name| env::var_os(name)),
        })
    }

    fn folders(&self) -> impl Iterator<Item = &PathBuf> {
        [&self.main, &self.legacy]
            .into_iter()
            .chain(self.backup.as_ref())
    }

    /// Recovery files, oldest first (file names sort by timestamp).
    ///
    /// Scans the main folder, the legacy folder and the backup folder. A backup
    /// is only listed when its original is gone (same file name).
    pub fn all_recoveries(&self) -> Vec<PathBuf> {
        let mut seen = HashSet::new();
        let mut files = Vec::new();
        for folder in self.folders() {
            let Ok(entries) = fs::read_dir(folder) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if is_recovery_name(&name) && seen.insert(name.clone()) {
                    files.push((name, entry.path()));
                }
            }
        }
        files.sort();
        files.into_iter().map(|(_, path)| path).collect()
    }

    /// The newest recovery file whose label matches, ignoring case.
    pub fn find_by_label(&self, label: &str) -> Option<PathBuf> {
        let wanted = normalize_label(label);
        self.all_recoveries().into_iter().rev().find(|path| {
            let found = recovery::load(path)
                .map(|r| r.lookup_label())
                .unwrap_or_default();
            found.to_lowercase() == wanted
        })
    }

    /// Every existing copy of a recovery file: the original and any backups.
    pub fn copies_of(&self, path: &Path) -> Vec<PathBuf> {
        let name = path.file_name().unwrap_or_default();
        let mut seen = HashSet::new();
        let mut copies = Vec::new();
        let elsewhere = self.folders().map(|folder| folder.join(name));
        for copy in std::iter::once(path.to_path_buf()).chain(elsewhere) {
            if copy.is_file() {
                let key = fs::canonicalize(&copy)
                    .unwrap_or_else(|_| copy.clone())
                    .to_string_lossy()
                    .to_lowercase();
                if seen.insert(key) {
                    copies.push(copy);
                }
            }
        }
        copies
    }

    /// Seals `code` behind a time-lock measured on this machine and saves it.
    pub fn save_recovery(&self, code: &str, delay: f64, label: &str) -> io::Result<PathBuf> {
        let stamp = now_stamp();
        let puzzle = timelock::seal(code.as_bytes(), delay);
        self.write_sealed(&puzzle, &stamp, delay, label)
    }

    /// Saves an already sealed code in the main folder, with the line endings
    /// the original Locker would use on this platform.
    pub fn write_sealed(
        &self,
        puzzle: &Puzzle,
        stamp: &str,
        delay: f64,
        label: &str,
    ) -> io::Result<PathBuf> {
        create_private_dir(&self.main)?;
        let path = self.main.join(recovery::file_name(stamp, label));
        write_new_file(&path, &rendered(puzzle, stamp, delay, label))?;
        Ok(path)
    }

    /// Saves a sealed code as pending, before its setup puts it on the phone.
    /// [`Store::finish_pending`] later gives it its ordinary name.
    pub fn write_pending(
        &self,
        puzzle: &Puzzle,
        stamp: &str,
        delay: f64,
        label: &str,
    ) -> io::Result<PathBuf> {
        create_private_dir(&self.main)?;
        let name = recovery::file_name(stamp, label);
        let finished = self.main.join(&name);
        if finished.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} already exists", finished.display()),
            ));
        }
        let path = self.main.join(format!("{name}{PENDING_SUFFIX}"));
        write_new_file(&path, &rendered(puzzle, stamp, delay, label))?;
        Ok(path)
    }

    /// Pending files in the main folder, oldest first. Each one is a setup
    /// that never finished, so its code may be on a phone.
    pub fn pending_files(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(&self.main) else {
            return Vec::new();
        };
        let mut files: Vec<(String, PathBuf)> = entries
            .flatten()
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                )
            })
            .filter(|(name, _)| is_pending_name(name))
            .collect();
        files.sort();
        files.into_iter().map(|(_, path)| path).collect()
    }

    /// Gives a pending file its ordinary name, which makes it a saved code.
    /// An existing file of that name is never replaced.
    pub fn finish_pending(&self, pending: &Path) -> io::Result<PathBuf> {
        let name = name_of(pending);
        if !is_pending_name(&name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a pending recovery file", pending.display()),
            ));
        }
        let finished = pending.with_file_name(&name[..name.len() - PENDING_SUFFIX.len()]);
        if finished.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} already exists", finished.display()),
            ));
        }
        fs::rename(pending, &finished)?;
        Ok(finished)
    }

    /// Copies a recovery file into the backup folder, if there is one.
    pub fn copy_to_backup(&self, path: &Path) -> io::Result<Option<PathBuf>> {
        let Some(backup) = &self.backup else {
            return Ok(None);
        };
        fs::create_dir_all(backup)?;
        let copy = backup.join(path.file_name().unwrap_or_default());
        write_new_file(&copy, &fs::read(path)?)?;
        Ok(Some(copy))
    }

    /// Where a folder's deleted files wait. The legacy folder is never
    /// written, so its files wait in the main folder's bin.
    fn bin_for(&self, folder: &Path) -> PathBuf {
        if folder == self.legacy {
            self.main.join(BIN_FOLDER)
        } else {
            folder.join(BIN_FOLDER)
        }
    }

    fn bins(&self) -> Vec<PathBuf> {
        let mut bins = vec![self.main.join(BIN_FOLDER)];
        if let Some(backup) = &self.backup {
            let bin = backup.join(BIN_FOLDER);
            if !bins.contains(&bin) {
                bins.push(bin);
            }
        }
        bins
    }

    fn code_path(&self, bin_name: &str) -> PathBuf {
        self.main
            .join(BIN_FOLDER)
            .join(format!("{bin_name}{CODE_SUFFIX}"))
    }

    /// Moves every copy of a recovery file into the bin, where it waits
    /// [`BIN_DAYS`] days, and reports how each move went. One failure doesn't
    /// stop the others.
    ///
    /// `revealed` is the file's code, if it has already been shown. The bin
    /// then keeps it in plain text too, in the main folder only, so it never
    /// reaches a synced folder.
    pub fn move_to_bin(
        &self,
        path: &Path,
        revealed: Option<&str>,
    ) -> Vec<(PathBuf, io::Result<()>)> {
        let name = format!("{}{DELETED_MARK}{}", name_of(path), now_stamp());
        let outcomes: Vec<(PathBuf, io::Result<()>)> = self
            .copies_of(path)
            .into_iter()
            .map(|copy| {
                let bin = self.bin_for(copy.parent().unwrap_or(&self.main));
                let moved =
                    create_private_dir(&bin).and_then(|()| move_new(&copy, &bin.join(&name)));
                (copy, moved)
            })
            .collect();
        if let Some(code) = revealed
            && outcomes.iter().any(|(_, moved)| moved.is_ok())
        {
            // without it the file can still be restored and revealed again
            let _ = create_private_dir(&self.main.join(BIN_FOLDER)).and_then(|()| {
                write_new_file(&self.code_path(&name), format!("{code}\n").as_bytes())
            });
        }
        outcomes
    }

    /// What's in the bin, most recently deleted first.
    pub fn deleted(&self) -> Vec<Deleted> {
        let mut found: BTreeMap<(NaiveDateTime, String), Vec<PathBuf>> = BTreeMap::new();
        for bin in self.bins() {
            let Ok(entries) = fs::read_dir(&bin) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Some((_, at)) = parse_bin_name(&name) {
                    found.entry((at, name)).or_default().push(entry.path());
                }
            }
        }
        found
            .into_iter()
            .rev()
            .map(|((at, name), copies)| Deleted {
                code: self.read_code(&name),
                name,
                copies,
                at,
            })
            .collect()
    }

    fn read_code(&self, bin_name: &str) -> Option<String> {
        let text = fs::read_to_string(self.code_path(bin_name)).ok()?;
        let code = text.trim();
        // one line of anything: locked text may hold spaces and any letters
        let plausible = !code.is_empty() && code.len() <= 64 && !code.chars().any(char::is_control);
        plausible.then(|| code.to_string())
    }

    /// Puts a deleted code's copies back in the folders they came from, and
    /// reports where each went. A copy whose old name has been taken stays in
    /// the bin. A copy from the legacy folder comes back in the main folder.
    pub fn restore(&self, deleted: &Deleted) -> Vec<(PathBuf, io::Result<PathBuf>)> {
        let Some((original, _)) = parse_bin_name(&deleted.name) else {
            return Vec::new();
        };
        let outcomes: Vec<(PathBuf, io::Result<PathBuf>)> = deleted
            .copies
            .iter()
            .map(|copy| {
                let folder = copy.parent().and_then(Path::parent).unwrap_or(&self.main);
                let back = folder.join(original);
                (copy.clone(), move_new(copy, &back).map(|()| back))
            })
            .collect();
        if outcomes.iter().all(|(_, moved)| moved.is_ok()) {
            let _ = remove_if_there(&self.code_path(&deleted.name));
        }
        outcomes
    }

    /// Removes a deleted code for good: every copy, and its code in plain
    /// text if the bin kept one. Reports how each removal went.
    pub fn purge(&self, deleted: &Deleted) -> Vec<(PathBuf, io::Result<()>)> {
        let mut outcomes = delete_copies(&deleted.copies);
        let code = self.code_path(&deleted.name);
        if let Err(error) = remove_if_there(&code) {
            outcomes.push((code, Err(error)));
        }
        outcomes
    }

    /// Removes for good whatever has waited in the bin its [`BIN_DAYS`] days.
    pub fn purge_expired(&self, now: NaiveDateTime) {
        for deleted in self.deleted() {
            if deleted.days_left(now) <= 0 {
                let _ = self.purge(&deleted);
            }
        }
        // a code in plain text goes at the same age, even if its file has
        // gone some other way
        let Ok(entries) = fs::read_dir(self.main.join(BIN_FOLDER)) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let expired = name
                .strip_suffix(CODE_SUFFIX)
                .and_then(parse_bin_name)
                .is_some_and(|(_, at)| at + TimeDelta::days(BIN_DAYS) <= now);
            if expired {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// A recovery file's bytes, with the line endings the original Locker would
/// use here.
fn rendered(puzzle: &Puzzle, stamp: &str, delay: f64, label: &str) -> Vec<u8> {
    let text = recovery::render_file(puzzle, stamp, label, delay);
    let text = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text
    };
    text.into_bytes()
}

/// Deletes each file and reports how each attempt went. One failure doesn't
/// stop the others.
pub fn delete_copies(copies: &[PathBuf]) -> Vec<(PathBuf, io::Result<()>)> {
    copies
        .iter()
        .map(|path| (path.clone(), fs::remove_file(path)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timelock::{seal_with_squarings, solve};
    use std::ops::ControlFlow;
    use tempfile::TempDir;

    struct Folders {
        _root: TempDir,
        store: Store,
    }

    fn folders() -> Folders {
        let root = TempDir::new().unwrap();
        let store = Store {
            main: root.path().join("main"),
            legacy: root.path().join("legacy"),
            backup: Some(root.path().join("backup")),
        };
        Folders { _root: root, store }
    }

    fn touch(folder: &Path, name: &str, label: &str) -> PathBuf {
        fs::create_dir_all(folder).unwrap();
        let path = folder.join(name);
        let json = format!(r#"{{"label": "{label}"}}"#);
        let line = crate::base64::encode(json.as_bytes());
        fs::write(&path, format!("LOCKER RECOVERY FILE\n\n{line}\n")).unwrap();
        path
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths.iter().map(|path| name_of(path)).collect()
    }

    fn no_env(_: &str) -> Option<OsString> {
        None
    }

    #[test]
    fn listing_is_sorted_oldest_first_and_prefers_originals() {
        let f = folders();
        let backup = f.store.backup.clone().unwrap();
        let original = touch(&f.store.main, "recovery-20260102-000000-b.txt", "b");
        touch(&f.store.main, "recovery-20260101-000000-a.txt", "a");
        touch(&f.store.legacy, "recovery-20250101-000000-old.txt", "old");
        touch(&backup, "recovery-20260102-000000-b.txt", "b");
        touch(&backup, "recovery-20260103-000000-only-backup.txt", "c");

        let files = f.store.all_recoveries();
        assert_eq!(
            names(&files),
            [
                "recovery-20250101-000000-old.txt",
                "recovery-20260101-000000-a.txt",
                "recovery-20260102-000000-b.txt",
                "recovery-20260103-000000-only-backup.txt",
            ]
        );
        assert!(
            files.contains(&original),
            "the original wins over its backup"
        );
    }

    #[test]
    fn listing_ignores_other_files_and_missing_folders() {
        let f = folders();
        assert!(f.store.all_recoveries().is_empty());
        for name in [
            "notes.txt",
            "recovery.txt",
            "recovery-1.txt.pending",
            ".recovery-1.txt.tmp",
            "x-recovery-1.txt",
        ] {
            touch(&f.store.main, name, "x");
        }
        assert!(f.store.all_recoveries().is_empty());
        touch(&f.store.main, "recovery-.txt", "x");
        assert_eq!(names(&f.store.all_recoveries()), ["recovery-.txt"]);
    }

    #[test]
    fn name_matching_ignores_case_only_on_windows() {
        assert_eq!(is_recovery_name("RECOVERY-1.TXT"), cfg!(windows));
        assert!(is_recovery_name("recovery-1.txt"));
    }

    #[test]
    fn labels_match_ignoring_case_and_the_newest_wins() {
        let f = folders();
        touch(&f.store.main, "recovery-20260101-000000-a.txt", "iPhone");
        let newest = touch(&f.store.main, "recovery-20260102-000000-a.txt", "IPHONE");
        touch(&f.store.main, "recovery-20260103-000000-b.txt", "kids");
        assert_eq!(f.store.find_by_label("  iphone "), Some(newest));
        assert_eq!(f.store.find_by_label("nope"), None);
    }

    #[test]
    fn copies_include_backups_but_not_the_same_file_twice() {
        let f = folders();
        let backup = f.store.backup.clone().unwrap();
        let original = touch(&f.store.main, "recovery-1.txt", "a");
        let copy = touch(&backup, "recovery-1.txt", "a");
        touch(&backup, "recovery-2.txt", "b");
        assert_eq!(
            f.store.copies_of(&original),
            [original.clone(), copy.clone()]
        );
        assert_eq!(f.store.copies_of(&copy), [copy.clone(), original.clone()]);

        let results = delete_copies(&f.store.copies_of(&original));
        assert!(results.iter().all(|(_, result)| result.is_ok()));
        assert!(!original.exists() && !copy.exists());
        assert!(backup.join("recovery-2.txt").exists(), "other files stay");
    }

    #[test]
    fn deleting_keeps_going_after_a_failure() {
        let f = folders();
        let real = touch(&f.store.main, "recovery-1.txt", "a");
        let missing = f.store.main.join("recovery-2.txt");
        let results = delete_copies(&[missing, real.clone()]);
        assert!(results[0].1.is_err());
        assert!(results[1].1.is_ok());
        assert!(!real.exists());
    }

    #[test]
    fn a_saved_file_opens_and_is_backed_up_byte_for_byte() {
        let f = folders();
        let path = f.store.save_recovery("4821", 0.0001, "kids iPad").unwrap();
        assert_eq!(path.parent().unwrap(), f.store.main);
        assert!(name_of(&path).ends_with("-kids-ipad.txt"));

        let data = recovery::load(&path).unwrap();
        assert_eq!(data.label().as_deref(), Some("kids iPad"));
        let code = solve(&data.puzzle().unwrap(), |_, _| ControlFlow::Continue(()));
        assert_eq!(code.as_deref(), Ok("4821"));

        let raw = fs::read(&path).unwrap();
        assert_eq!(raw.windows(2).any(|pair| pair == b"\r\n"), cfg!(windows));

        let copy = f.store.copy_to_backup(&path).unwrap().unwrap();
        assert_eq!(fs::read(copy).unwrap(), raw);
    }

    #[test]
    fn nothing_is_written_to_the_legacy_folder() {
        let f = folders();
        f.store.save_recovery("4821", 0.0001, "x").unwrap();
        assert!(!f.store.legacy.exists());
    }

    #[test]
    fn an_existing_file_is_never_replaced() {
        let f = folders();
        let puzzle = seal_with_squarings(b"1111", 1000, 0);
        let first = f
            .store
            .write_sealed(&puzzle, "20260101-000000", 1.0, "same")
            .unwrap();
        let before = fs::read(&first).unwrap();

        let other = seal_with_squarings(b"2222", 1000, 0);
        let error = f
            .store
            .write_sealed(&other, "20260101-000000", 1.0, "same")
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&first).unwrap(), before);
    }

    #[test]
    fn writing_leaves_no_temporary_file() {
        let f = folders();
        let puzzle = seal_with_squarings(b"1111", 1000, 0);
        f.store
            .write_sealed(&puzzle, "20260101-000000", 1.0, "x")
            .unwrap();
        let entries: Vec<_> = fs::read_dir(&f.store.main).unwrap().flatten().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn no_backup_folder_means_no_backup() {
        let mut f = folders();
        f.store.backup = None;
        let path = touch(&f.store.main, "recovery-1.txt", "a");
        assert_eq!(f.store.copy_to_backup(&path).unwrap(), None);
    }

    #[test]
    fn backup_variable_wins_and_expands_the_home_folder() {
        let home = TempDir::new().unwrap();
        fs::create_dir(home.path().join("Dropbox")).unwrap();
        let env_var = |value: &'static str| {
            move |name: &str| (name == "LOCKER_BACKUP_DIR").then(|| OsString::from(value))
        };
        assert_eq!(
            find_backup_dir(home.path(), env_var("somewhere/else")),
            Some(PathBuf::from("somewhere/else"))
        );
        assert_eq!(
            find_backup_dir(home.path(), env_var("~/mine")),
            Some(home.path().join("mine"))
        );
        // an empty value counts as unset
        assert_eq!(
            find_backup_dir(home.path(), env_var("")),
            Some(home.path().join("Dropbox").join("Apps").join("Locker"))
        );
    }

    #[test]
    fn onedrive_variables_need_a_real_folder() {
        let home = TempDir::new().unwrap();
        let cloud = TempDir::new().unwrap();
        let cloud_path = cloud.path().as_os_str().to_owned();
        let missing = cloud.path().join("missing").into_os_string();
        let found = find_backup_dir(home.path(), |name| match name {
            "OneDrive" => Some(missing.clone()),
            "OneDriveCommercial" => Some(cloud_path.clone()),
            _ => None,
        });
        assert_eq!(found, Some(cloud.path().join("Apps").join("Locker")));
    }

    #[test]
    fn sync_folders_are_tried_in_order() {
        let home = TempDir::new().unwrap();
        assert_eq!(find_backup_dir(home.path(), no_env), None);
        let expect = |folder: PathBuf| Some(folder.join("Apps").join("Locker"));

        fs::create_dir(home.path().join("Sync")).unwrap();
        assert_eq!(
            find_backup_dir(home.path(), no_env),
            expect(home.path().join("Sync"))
        );
        for name in ["OneDrive", "Nextcloud", "Dropbox"] {
            fs::create_dir(home.path().join(name)).unwrap();
            assert_eq!(
                find_backup_dir(home.path(), no_env),
                expect(home.path().join(name))
            );
        }
        let icloud = home
            .path()
            .join("Library/Mobile Documents/com~apple~CloudDocs");
        fs::create_dir_all(&icloud).unwrap();
        assert_eq!(find_backup_dir(home.path(), no_env), expect(icloud));

        // macOS providers come first, sorted, and only folders count
        let providers = home.path().join("Library/CloudStorage");
        fs::create_dir_all(providers.join("OneDrive-Personal")).unwrap();
        fs::create_dir_all(providers.join("Dropbox")).unwrap();
        fs::write(providers.join("A-file"), "").unwrap();
        assert_eq!(
            find_backup_dir(home.path(), no_env),
            expect(providers.join("Dropbox"))
        );
    }

    #[test]
    fn pending_files_are_invisible_to_listing_until_finished() {
        let f = folders();
        let puzzle = seal_with_squarings(b"4821", 1000, 0);
        let pending = f
            .store
            .write_pending(&puzzle, "20260101-000000", 60.0, "kids iPad")
            .unwrap();
        assert_eq!(
            name_of(&pending),
            "recovery-20260101-000000-kids-ipad.txt.pending"
        );
        assert!(
            f.store.all_recoveries().is_empty(),
            "the original Locker's pattern skips it"
        );
        assert_eq!(f.store.pending_files(), std::slice::from_ref(&pending));

        // the same bytes as a finished file, so it opens the same way
        let data = recovery::load(&pending).unwrap();
        let code = solve(&data.puzzle().unwrap(), |_, _| ControlFlow::Continue(()));
        assert_eq!(code.as_deref(), Ok("4821"));

        let finished = f.store.finish_pending(&pending).unwrap();
        assert_eq!(name_of(&finished), "recovery-20260101-000000-kids-ipad.txt");
        assert!(!pending.exists());
        assert_eq!(f.store.all_recoveries(), [finished]);
        assert!(f.store.pending_files().is_empty());
    }

    #[test]
    fn pending_files_never_replace_a_saved_code() {
        let f = folders();
        let puzzle = seal_with_squarings(b"1111", 1000, 0);
        let saved = f
            .store
            .write_sealed(&puzzle, "20260101-000000", 1.0, "x")
            .unwrap();
        let before = fs::read(&saved).unwrap();

        let other = seal_with_squarings(b"2222", 1000, 0);
        let refused = f.store.write_pending(&other, "20260101-000000", 1.0, "x");
        assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::AlreadyExists);

        // a pending file whose finished name got taken in the meantime stays put
        let pending = f
            .store
            .write_pending(&other, "20260101-000001", 1.0, "x")
            .unwrap();
        fs::write(f.store.main.join("recovery-20260101-000001-x.txt"), "taken").unwrap();
        let error = f.store.finish_pending(&pending).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(pending.exists());
        assert_eq!(fs::read(&saved).unwrap(), before);
    }

    #[test]
    fn only_pending_names_can_be_finished_or_listed_as_pending() {
        let f = folders();
        let saved = touch(&f.store.main, "recovery-1.txt", "a");
        touch(&f.store.main, "notes.txt.pending", "a");
        touch(&f.store.main, "recovery-2.pending", "a");
        assert!(f.store.pending_files().is_empty());
        let error = f.store.finish_pending(&saved).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(saved.exists());
    }

    fn at(stamp: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(stamp, STAMP_FORMAT).unwrap()
    }

    fn count(folder: &Path) -> usize {
        fs::read_dir(folder).map_or(0, |entries| entries.count())
    }

    #[test]
    fn deleting_moves_every_copy_into_its_own_folders_bin() {
        let f = folders();
        let backup = f.store.backup.clone().unwrap();
        let original = touch(&f.store.main, "recovery-20260101-000000-a.txt", "a");
        let copy = touch(&backup, "recovery-20260101-000000-a.txt", "a");
        let other = touch(&f.store.main, "recovery-20260102-000000-b.txt", "b");
        let bytes = fs::read(&original).unwrap();

        let outcomes = f.store.move_to_bin(&original, None);
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|(_, moved)| moved.is_ok()));
        assert!(!original.exists() && !copy.exists());
        assert_eq!(f.store.all_recoveries(), [other]);

        let bin = f.store.deleted();
        assert_eq!(bin.len(), 1);
        assert!(
            bin[0]
                .name
                .starts_with("recovery-20260101-000000-a.txt.deleted-")
        );
        assert_eq!(bin[0].code, None);
        let in_bins: Vec<&Path> = bin[0].copies.iter().flat_map(|c| c.parent()).collect();
        assert_eq!(
            in_bins,
            [f.store.main.join("deleted"), backup.join("deleted")]
        );
        assert_eq!(
            fs::read(&bin[0].copies[0]).unwrap(),
            bytes,
            "moved, not rewritten"
        );
    }

    #[test]
    fn restoring_puts_every_copy_back_as_it_was() {
        let f = folders();
        let backup = f.store.backup.clone().unwrap();
        let original = touch(&f.store.main, "recovery-20260101-000000-a.txt", "a");
        let copy = touch(&backup, "recovery-20260101-000000-a.txt", "a");
        let bytes = fs::read(&original).unwrap();
        f.store.move_to_bin(&original, Some("4821"));

        let bin = f.store.deleted();
        let outcomes = f.store.restore(&bin[0]);
        let back: Vec<PathBuf> = outcomes
            .into_iter()
            .map(|(_, moved)| moved.unwrap())
            .collect();
        assert_eq!(back, [original.clone(), copy.clone()]);
        assert_eq!(fs::read(&original).unwrap(), bytes);
        assert_eq!(fs::read(&copy).unwrap(), bytes);
        assert_eq!(f.store.all_recoveries(), [original]);
        assert!(f.store.deleted().is_empty());
        assert_eq!(
            count(&f.store.main.join("deleted")),
            0,
            "the plain code goes too"
        );
    }

    #[test]
    fn a_restore_never_replaces_a_file() {
        let f = folders();
        let backup = f.store.backup.clone().unwrap();
        let original = touch(&f.store.main, "recovery-20260101-000000-a.txt", "a");
        let copy = touch(&backup, "recovery-20260101-000000-a.txt", "a");
        f.store.move_to_bin(&original, Some("4821"));
        fs::write(&original, "taken").unwrap();

        let bin = f.store.deleted();
        let outcomes = f.store.restore(&bin[0]);
        assert_eq!(
            outcomes[0].1.as_ref().unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(outcomes[1].1.as_ref().unwrap(), &copy);
        assert_eq!(fs::read(&original).unwrap(), b"taken");

        // what couldn't go back is still in the bin, with its code
        let left = f.store.deleted();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].copies, [bin[0].copies[0].clone()]);
        assert_eq!(left[0].code.as_deref(), Some("4821"));
    }

    #[test]
    fn a_revealed_code_waits_in_plain_text_in_the_main_bin_only() {
        let f = folders();
        let backup = f.store.backup.clone().unwrap();
        let only_backup = touch(&backup, "recovery-20260101-000000-a.txt", "a");
        f.store.move_to_bin(&only_backup, Some("4821"));

        let bin = f.store.deleted();
        assert_eq!(bin[0].code.as_deref(), Some("4821"));
        assert_eq!(count(&backup.join("deleted")), 1, "just the file");
        let code_file = f
            .store
            .main
            .join("deleted")
            .join(format!("{}.code", bin[0].name));
        assert_eq!(fs::read_to_string(code_file).unwrap(), "4821\n");

        // without a reveal there is no such file
        let unseen = touch(&f.store.main, "recovery-20260102-000000-b.txt", "b");
        f.store.move_to_bin(&unseen, None);
        assert_eq!(count(&f.store.main.join("deleted")), 2);
        assert_eq!(f.store.deleted()[0].code, None);
    }

    #[test]
    fn revealed_text_waits_in_the_bin_as_it_is() {
        let f = folders();
        let text = "correct horse ✓ staple";
        let file = touch(&f.store.main, "recovery-20260101-000000-a.txt", "a");
        f.store.move_to_bin(&file, Some(text));
        assert_eq!(f.store.deleted()[0].code.as_deref(), Some(text));

        // anything that isn't one line of text is not shown
        let code_file = f.store.code_path(&f.store.deleted()[0].name);
        for bad in ["", "\n", "two\nlines\n", "tab\there\n", &"x".repeat(65)] {
            fs::write(&code_file, bad).unwrap();
            assert_eq!(f.store.deleted()[0].code, None, "{bad:?}");
        }
    }

    #[test]
    fn purging_removes_every_copy_and_the_plain_code() {
        let f = folders();
        let backup = f.store.backup.clone().unwrap();
        let original = touch(&f.store.main, "recovery-20260101-000000-a.txt", "a");
        touch(&backup, "recovery-20260101-000000-a.txt", "a");
        let kept = touch(&f.store.main, "recovery-20260102-000000-b.txt", "b");
        f.store.move_to_bin(&original, Some("4821"));

        let outcomes = f.store.purge(&f.store.deleted()[0]);
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|(_, removed)| removed.is_ok()));
        assert!(f.store.deleted().is_empty());
        assert_eq!(count(&f.store.main.join("deleted")), 0);
        assert_eq!(count(&backup.join("deleted")), 0);
        assert_eq!(f.store.all_recoveries(), [kept]);
    }

    #[test]
    fn the_bin_empties_itself_after_thirty_days() {
        let f = folders();
        let backup = f.store.backup.clone().unwrap();
        let name = "recovery-20251201-000000-a.txt.deleted-20260101-120000";
        touch(&f.store.main.join("deleted"), name, "a");
        touch(&backup.join("deleted"), name, "a");
        fs::write(f.store.code_path(name), "4821\n").unwrap();
        let newer = "recovery-20251202-000000-b.txt.deleted-20260115-120000";
        touch(&f.store.main.join("deleted"), newer, "b");

        let old = &f.store.deleted()[1];
        assert_eq!(old.name, name);
        for (now, days) in [
            ("20260101-120000", 30),
            ("20260101-120001", 30),
            ("20260102-120000", 29),
            ("20260131-115959", 1),
            ("20260131-120000", 0),
            ("20260210-000000", -9),
            // a clock that has been set back
            ("20251225-000000", 30),
        ] {
            assert_eq!(old.days_left(at(now)), days, "{now}");
        }

        f.store.purge_expired(at("20260131-115959"));
        assert_eq!(f.store.deleted().len(), 2, "a second early is too early");

        f.store.purge_expired(at("20260131-120000"));
        let left = f.store.deleted();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].name, newer);
        assert_eq!(count(&f.store.main.join("deleted")), 1);
        assert_eq!(count(&backup.join("deleted")), 0);
    }

    #[test]
    fn a_stray_plain_code_goes_at_the_same_age() {
        let f = folders();
        let bin = f.store.main.join("deleted");
        fs::create_dir_all(&bin).unwrap();
        let stray = f.store.code_path("recovery-1.txt.deleted-20260101-120000");
        fs::write(&stray, "4821\n").unwrap();
        fs::write(bin.join("notes.code"), "not ours").unwrap();

        f.store.purge_expired(at("20260131-115959"));
        assert!(stray.exists());
        f.store.purge_expired(at("20260131-120000"));
        assert!(!stray.exists());
        assert!(bin.join("notes.code").exists(), "other files stay");
    }

    #[test]
    fn a_legacy_file_waits_in_the_main_bin_and_comes_back_in_the_main_folder() {
        let f = folders();
        let old = touch(&f.store.legacy, "recovery-20250101-000000-old.txt", "old");
        let outcomes = f.store.move_to_bin(&old, None);
        assert!(outcomes[0].1.is_ok());
        assert_eq!(
            count(&f.store.legacy),
            0,
            "nothing is written to the legacy folder"
        );

        let bin = f.store.deleted();
        assert_eq!(
            bin[0].copies[0].parent().unwrap(),
            f.store.main.join("deleted")
        );
        let back = f.store.restore(&bin[0]).remove(0).1.unwrap();
        assert_eq!(back, f.store.main.join("recovery-20250101-000000-old.txt"));
    }

    #[test]
    fn the_listings_and_the_bin_keep_to_their_own_names() {
        let f = folders();
        // a bin name isn't a saved or pending code, wherever it turns up
        touch(&f.store.main, "recovery-1.txt.deleted-20260101-120000", "a");
        assert!(f.store.all_recoveries().is_empty());
        assert!(f.store.pending_files().is_empty());

        // and only whole bin names count inside the bin
        let bin = f.store.main.join("deleted");
        for name in [
            "notes.txt",
            "recovery-1.txt",
            "recovery-1.txt.deleted-2026",
            "recovery-1.txt.deleted-20261340-120000",
            "recovery-1.txt.pending.deleted-20260101-120000",
            "x.deleted-20260101-120000",
            "recovery-1.txt.deleted-20260101-120000.code",
        ] {
            touch(&bin, name, "x");
        }
        assert!(f.store.deleted().is_empty());
    }

    #[test]
    fn the_newest_delete_is_listed_first() {
        let f = folders();
        let bin = f.store.main.join("deleted");
        touch(&bin, "recovery-3.txt.deleted-20260102-000000", "a");
        touch(&bin, "recovery-1.txt.deleted-20260103-000000", "b");
        touch(&bin, "recovery-2.txt.deleted-20260101-000000", "c");
        let names: Vec<String> = f.store.deleted().into_iter().map(|d| d.name).collect();
        assert_eq!(
            names,
            [
                "recovery-1.txt.deleted-20260103-000000",
                "recovery-3.txt.deleted-20260102-000000",
                "recovery-2.txt.deleted-20260101-000000",
            ]
        );
    }
}
