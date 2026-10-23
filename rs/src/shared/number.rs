//! How a number is written as text: by its lexeme when it has a JSON
//! one, else the shortest text that reads back, laid out as the
//! renderers lay it out. The language's `scalar-text` and a native that
//! names a column by a number (`transduce-records`) write it the same
//! way, so it lives here, where transduce reaches it.

use crate::shared::error::{Code, Fail};

/// Whether `text` is exactly a JSON number:
/// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`. Written out rather
/// than as a regular expression so the crate takes no regex dependency
/// for one pattern.
pub fn is_json_number(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(c) if c.is_ascii_digit() => {
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let start = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == b.len()
}

/// The magnitudes written positionally, as the renderers write them:
/// from `1e-6` up to, not including, `1e21`.
const POSITIONAL_MIN: f64 = 1e-6;
const POSITIONAL_MAX: f64 = 1e21;

/// The shortest text that reads back as `value`, laid out as the render
/// crate lays it out (`number::write_value` there): positional within the
/// JavaScript range, exponent form outside it. The two must agree byte for
/// byte, which the differential test checks on every number without a
/// lexeme.
pub fn shortest_number(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude == 0.0 || (POSITIONAL_MIN..POSITIONAL_MAX).contains(&magnitude) {
        format!("{value}")
    } else {
        format!("{value:e}")
    }
}

/// The text of a number as a renderer writes it, with the renderer's
/// checks: a lexeme that is not a JSON number is `INVALID_NUMBER`, a
/// non-finite value `TARGET_VALUE_UNREPRESENTABLE`.
pub fn number_text(value: f64, lexeme: Option<&str>) -> Result<String, Fail> {
    if let Some(l) = lexeme {
        if !is_json_number(l) {
            return Err(Fail::new(
                Code::InvalidNumber,
                format!("{l:?} is not a JSON number"),
            ));
        }
    }
    if !value.is_finite() {
        let message = match lexeme {
            Some(l) => format!("{l:?} is {value} as a number, which has no representation"),
            None => format!("{value} has no representation as a number"),
        };
        return Err(Fail::new(Code::TargetValueUnrepresentable, message));
    }
    Ok(match lexeme {
        Some(l) => l.to_string(),
        None => shortest_number(value),
    })
}
