//! The recovery file format, version 3, and the small text rules around it.
//!
//! A recovery file is plain text whose last line is base64-encoded JSON. These
//! functions match the original Locker byte for byte, so
//! `tests/fixtures/vectors.json` holds what Python returned for each of them.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use num_bigint::BigUint;
use serde_json::{Map, Value};
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory};

use crate::base64;
use crate::timelock::Puzzle;

const SLUG_CHARS: usize = 24;

/// Python's `str.strip()` whitespace: Rust's plus the separators U+001C-001F.
fn is_python_space(c: char) -> bool {
    c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
}

/// Python's `str.splitlines()` boundaries.
fn is_python_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '\x0b'
            | '\x0c'
            | '\x1c'
            | '\x1d'
            | '\x1e'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// Python's `str.isalnum()`, by general category. Rust's `is_alphanumeric` is
/// wider: it also takes vowel signs in many scripts and circled letters.
fn is_python_alnum(c: char) -> bool {
    matches!(
        c.general_category_group(),
        GeneralCategoryGroup::Letter | GeneralCategoryGroup::Number
    )
}

/// The label's form in file names: lowercase, with runs of anything else
/// turned into single dashes, cut at 24 characters (a dash may end up last).
pub fn slug(label: &str) -> String {
    let mapped: String = label
        .to_lowercase()
        .chars()
        .map(|c| if is_python_alnum(c) { c } else { '-' })
        .collect();
    let parts: Vec<&str> = mapped.split('-').filter(|part| !part.is_empty()).collect();
    parts.join("-").chars().take(SLUG_CHARS).collect()
}

pub fn file_name(stamp: &str, label: &str) -> String {
    match slug(label).as_str() {
        "" => format!("recovery-{stamp}.txt"),
        slug => format!("recovery-{stamp}-{slug}.txt"),
    }
}

pub fn fmt_duration(seconds: f64) -> String {
    if seconds < 90.0 {
        let n = seconds.round_ties_even().max(1.0) as u64;
        format!("{n} second{}", if n == 1 { "" } else { "s" })
    } else if seconds < 5400.0 {
        format!("{} minutes", (seconds / 60.0).round_ties_even() as u64)
    } else {
        format!("{:.1} hours", seconds / 3600.0)
    }
}

/// Parses "30s", "5m", "2h" or a bare number of minutes into seconds. Empty
/// input means the 1-minute default; None means unparseable.
///
/// Unlike the original Locker this refuses infinity and NaN. Python accepted
/// them here and then crashed at sealing, after the code was already on the
/// phone. It also refuses Python's other `float()` extras, `1_000` and
/// non-ASCII digits, which only ever leads to being asked again.
pub fn parse_delay(text: &str) -> Option<f64> {
    let text = text.trim_matches(is_python_space).to_lowercase();
    if text.is_empty() {
        return Some(60.0);
    }
    let (number, unit) = match text.chars().last() {
        Some('s') => (&text[..text.len() - 1], 1.0),
        Some('m') => (&text[..text.len() - 1], 60.0),
        Some('h') => (&text[..text.len() - 1], 3600.0),
        _ => (&text[..], 60.0),
    };
    let value: f64 = number.trim_matches(is_python_space).parse().ok()?;
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let seconds = value * unit;
    seconds.is_finite().then_some(seconds)
}

/// Shows a `YYYYMMDD-HHMMSS` stamp as `YYYY-MM-DD HH:MM`. Anything that isn't
/// 15 characters long is shown as it is.
pub fn fmt_created(stamp: &str) -> String {
    let c: Vec<char> = stamp.chars().collect();
    if c.len() != 15 {
        return stamp.to_string();
    }
    let part = |from: usize, to: usize| c[from..to].iter().collect::<String>();
    format!(
        "{}-{}-{} {}:{}",
        part(0, 4),
        part(4, 6),
        part(6, 8),
        part(9, 11),
        part(11, 13)
    )
}

/// Normalizes a label the way lookups compare them.
pub(crate) fn normalize_label(label: &str) -> String {
    label.trim_matches(is_python_space).to_lowercase()
}

/// A JSON string as Python's `json.dumps` writes it: ASCII only.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    write!(out, "\\u{unit:04x}").expect("writing to a String can't fail");
                }
            }
        }
    }
    out.push('"');
    out
}

/// The JSON of a version 3 file. `n` stays a bare integer, which is what
/// the original Locker reads.
fn payload_json(puzzle: &Puzzle, stamp: &str, label: &str) -> String {
    format!(
        "{{\"version\": 3, \"n\": {}, \"a\": {}, \"t\": {}, \"ct\": {}, \"tag\": {}, \
         \"delay_seconds\": {}, \"created\": {}, \"label\": {}}}",
        puzzle.n,
        puzzle.a,
        puzzle.t,
        json_string(&base64::encode(&puzzle.ct)),
        json_string(&puzzle.tag),
        puzzle.delay_seconds,
        json_string(stamp),
        json_string(label),
    )
}

/// A whole recovery file with `\n` line endings. `delay` is the delay as it was
/// chosen, because `puzzle.delay_seconds` has lost any fraction of a second.
pub fn render_file(puzzle: &Puzzle, stamp: &str, label: &str, delay: f64) -> String {
    let payload = base64::encode(payload_json(puzzle, stamp, label).as_bytes());
    format!(
        "LOCKER RECOVERY FILE\n\
         Label: {label}\n\
         Created: {stamp}\n\
         The passcode is sealed in a computational time-lock. Revealing\n\
         it takes about {} of nonstop computation.\n\
         To recover it, run:  locker reveal {}\n\n\
         {payload}\n",
        fmt_duration(delay),
        slug(label),
    )
}

/// `load_recovery`'s line choice: the last line of the stripped text.
fn last_line(text: &str) -> Option<&str> {
    let text = text.trim_matches(is_python_space);
    if text.is_empty() {
        return None;
    }
    text.rsplit(is_python_line_break).next()
}

/// A recovery file's decoded contents.
///
/// This stays as loose as the Python dictionary it replaces. Only a reveal
/// needs the time-lock fields, and `list` must still show a file that lacks
/// them. `version` is never checked, because the original Locker didn't
/// either.
#[derive(Clone, Debug)]
pub struct Recovery {
    fields: Map<String, Value>,
}

fn is_truthy_number(value: &Value) -> bool {
    value.as_f64().is_some_and(|number| number != 0.0)
}

fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

impl Recovery {
    /// Decodes a file's text, or returns None if it can't be read.
    pub fn parse(text: &str) -> Option<Self> {
        let payload = base64::decode(last_line(text)?)?;
        match serde_json::from_slice(&payload) {
            Ok(Value::Object(fields)) => Some(Self { fields }),
            _ => None,
        }
    }

    /// The label to show, or None for a file with no usable label.
    pub fn label(&self) -> Option<String> {
        self.fields
            .get("label")
            .filter(|value| match value {
                Value::String(text) => !text.is_empty(),
                Value::Number(_) => is_truthy_number(value),
                _ => false,
            })
            .and_then(value_text)
    }

    /// The label as lookups see it: empty when there is none.
    pub(crate) fn lookup_label(&self) -> String {
        self.fields
            .get("label")
            .and_then(value_text)
            .unwrap_or_default()
    }

    /// The creation stamp, ready to show.
    pub fn created(&self) -> String {
        match self.fields.get("created").and_then(value_text) {
            Some(stamp) => fmt_created(&stamp),
            None => "unknown".to_string(),
        }
    }

    /// The code in plain text, on a legacy file. If this is Some, it wins over
    /// any time-lock fields.
    pub fn pin(&self) -> Option<String> {
        self.fields
            .get("pin")
            .map(|pin| value_text(pin).unwrap_or_else(|| pin.to_string()))
    }

    /// The delay the file claims, or None if it has none. This reflects the
    /// program that sealed it, so a reveal may well go faster.
    pub fn delay_seconds(&self) -> Option<f64> {
        self.fields
            .get("delay_seconds")
            .filter(|value| is_truthy_number(value))
            .and_then(Value::as_f64)
    }

    /// The time-lock, or None if its fields are missing or malformed.
    pub fn puzzle(&self) -> Option<Puzzle> {
        let big = |name: &str| -> Option<BigUint> {
            // arbitrary_precision keeps a number's digits as raw text
            match self.fields.get(name)? {
                Value::Number(number) => number.to_string().parse().ok(),
                _ => None,
            }
        };
        Some(Puzzle {
            n: big("n")?,
            a: big("a")?,
            t: self.fields.get("t")?.as_u64()?,
            ct: base64::decode(self.fields.get("ct")?.as_str()?)?,
            tag: self.fields.get("tag")?.as_str()?.to_string(),
            delay_seconds: self
                .fields
                .get("delay_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        })
    }
}

/// Reads a recovery file, or returns None if it can't be read.
pub fn load(path: &Path) -> Option<Recovery> {
    Recovery::parse(&fs::read_to_string(path).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vectors(name: &str) -> Vec<Value> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/vectors.json"
        );
        let all: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        all[name].as_array().unwrap().clone()
    }

    fn pair(case: &Value) -> (&str, &Value) {
        (case[0].as_str().unwrap(), &case[1])
    }

    #[test]
    fn slug_cases_from_the_python_tests() {
        for (label, want) in [
            ("iphone", "iphone"),
            ("kids iPad (upstairs)", "kids-ipad-upstairs"),
            ("???", ""),
            (&"a".repeat(40), &"a".repeat(24)),
        ] {
            assert_eq!(slug(label), want);
        }
    }

    #[test]
    fn slugs_match_python() {
        for case in vectors("slug") {
            let (label, want) = pair(&case);
            assert_eq!(slug(label), want.as_str().unwrap(), "label {label:?}");
        }
    }

    #[test]
    fn slugs_truncate_at_characters_not_bytes() {
        let label = "息".repeat(40);
        assert_eq!(slug(&label).chars().count(), 24);
        assert_eq!(slug(&label).len(), 72);
    }

    #[test]
    fn file_names_drop_an_empty_slug() {
        assert_eq!(
            file_name("20261022-142413", "iphone"),
            "recovery-20261022-142413-iphone.txt"
        );
        assert_eq!(
            file_name("20261022-142413", "???"),
            "recovery-20261022-142413.txt"
        );
    }

    #[test]
    fn durations_from_the_python_tests() {
        for (seconds, want) in [
            (1.0, "1 second"),
            (0.2, "1 second"),
            (45.0, "45 seconds"),
            (120.0, "2 minutes"),
            (7200.0, "2.0 hours"),
        ] {
            assert_eq!(fmt_duration(seconds), want);
        }
    }

    #[test]
    fn durations_match_python() {
        for case in vectors("fmt_duration") {
            let seconds = case[0].as_f64().unwrap();
            assert_eq!(
                fmt_duration(seconds),
                case[1].as_str().unwrap(),
                "{seconds} seconds"
            );
        }
    }

    #[test]
    fn delays_from_the_python_tests() {
        for (text, want) in [
            ("", Some(60.0)),
            ("30s", Some(30.0)),
            ("5", Some(300.0)),
            ("5m", Some(300.0)),
            ("2h", Some(7200.0)),
            ("1.5h", Some(5400.0)),
            (" 90S ", Some(90.0)),
            ("abc", None),
            ("0", None),
            ("-5m", None),
        ] {
            assert_eq!(parse_delay(text), want, "{text:?}");
        }
    }

    #[test]
    fn delays_match_python() {
        for case in vectors("parse_delay") {
            let (text, want) = pair(&case);
            assert_eq!(parse_delay(text), want.as_f64(), "{text:?}");
        }
    }

    #[test]
    fn delays_refuse_what_only_python_accepted() {
        for case in vectors("parse_delay_python_only") {
            let text = case.as_str().unwrap();
            assert_eq!(parse_delay(text), None, "{text:?}");
        }
    }

    #[test]
    fn created_stamps_match_python() {
        for case in vectors("fmt_created") {
            let (stamp, want) = pair(&case);
            assert_eq!(fmt_created(stamp), want.as_str().unwrap(), "{stamp:?}");
        }
    }

    #[test]
    fn json_strings_match_python() {
        for case in vectors("json_string") {
            let (text, want) = pair(&case);
            assert_eq!(json_string(text), want.as_str().unwrap(), "{text:?}");
        }
    }

    #[test]
    fn last_lines_match_python() {
        for case in vectors("last_line") {
            let (text, want) = pair(&case);
            assert_eq!(last_line(text), want.as_str(), "{text:?}");
        }
    }

    #[test]
    fn label_normalizing_matches_python() {
        for case in vectors("find_label") {
            let (label, want) = pair(&case);
            assert_eq!(normalize_label(label), want.as_str().unwrap(), "{label:?}");
        }
    }

    #[test]
    fn a_payload_that_isnt_an_object_is_unreadable() {
        for json in [&b"[1, 2]"[..], b"5", b"\"text\"", b"null"] {
            assert!(Recovery::parse(&base64::encode(json)).is_none());
        }
    }

    #[test]
    fn missing_or_malformed_time_lock_fields_give_no_puzzle() {
        for json in [
            r#"{"label": "x"}"#,
            r#"{"n": 15, "a": 2, "t": 1, "ct": "AA=="}"#,
            r#"{"n": "15", "a": 2, "t": 1, "ct": "AA==", "tag": "00"}"#,
            r#"{"n": 1.5, "a": 2, "t": 1, "ct": "AA==", "tag": "00"}"#,
            r#"{"n": -15, "a": 2, "t": 1, "ct": "AA==", "tag": "00"}"#,
            r#"{"n": 15, "a": 2, "t": -1, "ct": "AA==", "tag": "00"}"#,
            r#"{"n": 15, "a": 2, "t": 1, "ct": 5, "tag": "00"}"#,
        ] {
            let recovery = Recovery::parse(&base64::encode(json.as_bytes())).unwrap();
            assert!(recovery.puzzle().is_none(), "{json}");
        }
    }

    #[test]
    fn a_huge_modulus_survives_as_exact_digits() {
        let digits = "9".repeat(617);
        let json = format!(r#"{{"n": {digits}, "a": 2, "t": 1, "ct": "", "tag": ""}}"#);
        let recovery = Recovery::parse(&base64::encode(json.as_bytes())).unwrap();
        assert_eq!(recovery.puzzle().unwrap().n.to_string(), digits);
    }

    #[test]
    fn labels_show_like_python() {
        let parse = |json: &str| Recovery::parse(&base64::encode(json.as_bytes())).unwrap();
        assert_eq!(parse(r#"{"label": "x"}"#).label().as_deref(), Some("x"));
        assert_eq!(parse(r#"{"label": 5}"#).label().as_deref(), Some("5"));
        for json in [
            r#"{"label": ""}"#,
            r#"{"label": null}"#,
            r#"{"label": 0}"#,
            "{}",
        ] {
            assert_eq!(parse(json).label(), None, "{json}");
        }
        assert_eq!(
            parse(r#"{"created": "20261022-142413"}"#).created(),
            "2026-10-22 14:24"
        );
        assert_eq!(parse("{}").created(), "unknown");
        assert_eq!(parse(r#"{"delay_seconds": 0}"#).delay_seconds(), None);
        assert_eq!(
            parse(r#"{"delay_seconds": 60}"#).delay_seconds(),
            Some(60.0)
        );
    }
}
