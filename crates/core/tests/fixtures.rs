//! Rust must open everything the original Locker wrote. `tests/fixtures` holds
//! those files, and `manifest.json` records what each one should give.

use std::collections::BTreeSet;
use std::fs;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use locker_core::recovery::{self, file_name, render_file, slug};
use locker_core::storage::Store;
use locker_core::timelock::{Puzzle, SolveError, solve};
use serde_json::Value;

fn folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn manifest() -> Value {
    let text = fs::read_to_string(folder().join("manifest.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn entries(kind: &str) -> Vec<Value> {
    let found: Vec<Value> = manifest()["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == kind)
        .cloned()
        .collect();
    assert!(!found.is_empty(), "no {kind} files");
    found
}

fn file_of(entry: &Value) -> PathBuf {
    folder().join(entry["file"].as_str().unwrap())
}

fn open(puzzle: &Puzzle) -> Result<String, SolveError> {
    solve(puzzle, |_, _| ControlFlow::Continue(()))
}

#[test]
fn the_files_came_from_locker_1_0_0() {
    assert_eq!(manifest()["generator"]["locker_version"], "1.0.0");
}

#[test]
fn every_file_in_the_folder_is_in_the_manifest() {
    let recorded: BTreeSet<String> = manifest()["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["file"].as_str().unwrap().to_string())
        .collect();
    let on_disk: BTreeSet<String> = fs::read_dir(folder())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("recovery-"))
        .collect();
    assert_eq!(recorded, on_disk);
}

#[test]
fn time_locked_files_open_to_the_recorded_code() {
    for entry in entries("timelocked") {
        let name = entry["file"].as_str().unwrap();
        let data = recovery::load(&file_of(&entry)).unwrap_or_else(|| panic!("{name} won't load"));
        assert_eq!(data.pin(), None, "{name}");
        let puzzle = data
            .puzzle()
            .unwrap_or_else(|| panic!("{name} has no puzzle"));
        assert_eq!(puzzle.t, entry["t"].as_u64().unwrap(), "{name}");
        assert_eq!(puzzle.n.bits(), entry["n_bits"].as_u64().unwrap(), "{name}");
        assert_eq!(
            open(&puzzle).as_deref(),
            Ok(entry["code"].as_str().unwrap()),
            "{name}"
        );

        let label = entry["label"].as_str().unwrap();
        let want = (!label.is_empty()).then(|| label.to_string());
        assert_eq!(data.label(), want, "{name}");
        let created = recovery::fmt_created(entry["created"].as_str().unwrap());
        assert_eq!(data.created(), created, "{name}");
    }
}

#[test]
fn the_tricky_cases_are_really_covered() {
    let files = entries("timelocked");
    let bits: BTreeSet<u64> = files
        .iter()
        .map(|e| e["n_bits"].as_u64().unwrap())
        .collect();
    assert_eq!(bits, BTreeSet::from([2047, 2048]));
    let lengths: BTreeSet<usize> = files
        .iter()
        .map(|e| e["code"].as_str().unwrap().len())
        .collect();
    assert_eq!(lengths, BTreeSet::from([3, 4, 6, 12]));
    assert!(
        files
            .iter()
            .any(|e| !e["label"].as_str().unwrap().is_ascii())
    );
    assert!(
        files.iter().any(|e| e["t"].as_u64().unwrap() > 100_000),
        "a real speed test"
    );

    // the key has to be padded for this file, and a plain to_bytes_be would miss it
    let entry = files
        .iter()
        .find(|e| e["label"] == "x has a leading zero byte")
        .unwrap();
    let puzzle = recovery::load(&file_of(entry)).unwrap().puzzle().unwrap();
    let mut x = puzzle.a.clone();
    for _ in 0..puzzle.t {
        x = &x * &x % &puzzle.n;
    }
    assert!(x.to_bytes_be().len() < puzzle.n.bits().div_ceil(8) as usize);
}

#[test]
fn the_writer_matches_python_byte_for_byte() {
    let mut checked = 0;
    for entry in entries("timelocked") {
        if entry["byte_exact"] != true {
            continue;
        }
        let name = entry["file"].as_str().unwrap();
        let raw = fs::read(file_of(&entry)).unwrap();
        let puzzle = recovery::load(&file_of(&entry)).unwrap().puzzle().unwrap();
        let (stamp, label) = (
            entry["created"].as_str().unwrap(),
            entry["label"].as_str().unwrap(),
        );
        let text = render_file(&puzzle, stamp, label, entry["delay"].as_f64().unwrap());
        // these files were written on Windows
        let text = text.replace('\n', "\r\n");
        assert_eq!(String::from_utf8(raw).unwrap(), text, "{name}");
        assert_eq!(slug(label), entry["slug"].as_str().unwrap(), "{name}");
        assert_eq!(file_name(stamp, label), name, "{name}");
        checked += 1;
    }
    assert!(checked >= 30, "only {checked} files were compared");
}

#[test]
fn damaged_files_are_reported_as_damaged() {
    for entry in entries("damaged") {
        let name = entry["file"].as_str().unwrap();
        let puzzle = recovery::load(&file_of(&entry)).unwrap().puzzle().unwrap();
        assert_eq!(open(&puzzle), Err(SolveError::Damaged), "{name}");
    }
}

#[test]
fn legacy_files_give_up_their_pin() {
    for entry in entries("legacy") {
        let name = entry["file"].as_str().unwrap();
        let data = recovery::load(&file_of(&entry)).unwrap();
        assert_eq!(
            data.pin().as_deref(),
            Some(entry["code"].as_str().unwrap()),
            "{name}"
        );
        assert_eq!(data.label().as_deref(), entry["label"].as_str(), "{name}");
        match entry["created"].as_str() {
            Some(stamp) => assert_eq!(data.created(), recovery::fmt_created(stamp), "{name}"),
            None => assert_eq!(data.created(), "unknown", "{name}"),
        }
    }
}

#[test]
fn unreadable_files_do_not_load() {
    for entry in entries("unreadable") {
        let name = entry["file"].as_str().unwrap();
        assert!(recovery::load(&file_of(&entry)).is_none(), "{name}");
    }
}

#[test]
fn listing_sees_every_file_in_order() {
    let store = Store {
        main: folder(),
        legacy: folder().join("none"),
        backup: None,
    };
    let names: Vec<String> = store
        .all_recoveries()
        .iter()
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert_eq!(names.len(), manifest()["files"].as_array().unwrap().len());
}

#[test]
fn lookups_find_non_english_labels_ignoring_case() {
    let store = Store {
        main: folder(),
        legacy: folder().join("none"),
        backup: None,
    };
    let found = store.find_by_label("телефон сЫНА").unwrap();
    assert_eq!(
        recovery::load(&found).unwrap().label().as_deref(),
        Some("Телефон сына")
    );
}
