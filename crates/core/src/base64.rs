//! Base64 with the exact decoding rules of Python's `base64.b64decode`.
//!
//! Those rules are lenient: stray characters are skipped and a full padding
//! sequence ends the input. The original Locker opened files on those terms,
//! so a stricter decoder could lock someone out of a file that used to open.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let group = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(group >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn value_of(byte: u8) -> Option<u32> {
    match byte {
        b'A'..=b'Z' => Some((byte - b'A') as u32),
        b'a'..=b'z' => Some((byte - b'a') as u32 + 26),
        b'0'..=b'9' => Some((byte - b'0') as u32 + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decodes `text`, or returns None wherever Python would raise an error.
pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut group, mut filled, mut pads) = (0u32, 0, 0);
    for byte in text.bytes() {
        if byte == b'=' {
            if filled >= 2 {
                pads += 1;
                if filled + pads >= 4 {
                    return Some(out);
                }
            }
            continue;
        }
        let Some(value) = value_of(byte) else {
            continue;
        };
        pads = 0;
        group = group << 6 | value;
        filled += 1;
        match filled {
            2 => out.push((group >> 4) as u8),
            3 => out.push((group >> 2) as u8),
            4 => {
                out.push(group as u8);
                (group, filled) = (0, 0);
            }
            _ => {}
        }
    }
    (filled == 0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn vectors(name: &str) -> Vec<Value> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/vectors.json"
        );
        let all: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        all[name].as_array().unwrap().clone()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn decoding_matches_python() {
        for case in vectors("base64_decode") {
            let input = case[0].as_str().unwrap();
            let want = case[1].as_str().map(str::to_string);
            assert_eq!(decode(input).map(|b| hex(&b)), want, "input {input:?}");
        }
    }

    #[test]
    fn encoding_matches_python() {
        for case in vectors("base64_encode") {
            let input: Vec<u8> = (0..case[0].as_str().unwrap().len() / 2)
                .map(|i| {
                    u8::from_str_radix(&case[0].as_str().unwrap()[2 * i..2 * i + 2], 16).unwrap()
                })
                .collect();
            assert_eq!(encode(&input), case[1].as_str().unwrap());
        }
    }

    #[test]
    fn non_ascii_input_is_an_error() {
        assert_eq!(decode("abcd€efg"), None);
    }
}
