use std::cmp::Ordering;

use serde_json::{Map, Number, Value};

use crate::foundation::foundation_compare::{folding, CompareOptions};

/// json helpers, just a namespace
pub enum JSONSerialization {}

/// json writing flags
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JSONWritingOptions {
    pub sorted_keys: bool,
    pub pretty_printed: bool,
    pub without_escaping_slashes: bool,
    pub fragments_allowed: bool,
}

impl JSONSerialization {
    /// parses json bytes, top level must be an object or array
    /// utf-8, utf-16 and utf-32 are detected from the bom or the zero byte pattern of the first chars
    pub fn json_object(data: &[u8]) -> Option<Value> {
        let text = decode_json_text(data)?;
        let value: Value = serde_json::from_str(&text).ok()?;
        Self::is_valid_json_object(&value).then_some(value)
    }

    /// serializes json, none where foundation would throw
    /// eg a top level fragment without fragmentsAllowed
    pub fn data_with_json_object(value: &Value, options: JSONWritingOptions) -> Option<Vec<u8>> {
        if !options.fragments_allowed && !Self::is_valid_json_object(value) {
            return None;
        }
        let mut out = String::new();
        write_value(&mut out, value, options, 0);
        Some(out.into_bytes())
    }

    /// true if top level is an object or array, nested values are already valid json
    pub fn is_valid_json_object(value: &Value) -> bool {
        matches!(value, Value::Object(_) | Value::Array(_))
    }
}

fn decode_json_text(data: &[u8]) -> Option<String> {
    match data {
        [0x00, 0x00, 0xFE, 0xFF, rest @ ..] => decode_utf32(rest, u32::from_be_bytes),
        [0xFF, 0xFE, 0x00, 0x00, rest @ ..] => decode_utf32(rest, u32::from_le_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] => std::str::from_utf8(rest).ok().map(str::to_owned),
        [0xFE, 0xFF, rest @ ..] => decode_utf16(rest, u16::from_be_bytes),
        [0xFF, 0xFE, rest @ ..] => decode_utf16(rest, u16::from_le_bytes),
        [0x00, 0x00, 0x00, _, ..] => decode_utf32(data, u32::from_be_bytes),
        [_, 0x00, 0x00, 0x00, ..] => decode_utf32(data, u32::from_le_bytes),
        [0x00, _, ..] => decode_utf16(data, u16::from_be_bytes),
        [_, 0x00, ..] => decode_utf16(data, u16::from_le_bytes),
        _ => std::str::from_utf8(data).ok().map(str::to_owned),
    }
}

fn decode_utf16(bytes: &[u8], unit: fn([u8; 2]) -> u16) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units = bytes.chunks_exact(2).map(|pair| unit([pair[0], pair[1]]));
    char::decode_utf16(units).collect::<Result<String, _>>().ok()
}

fn decode_utf32(bytes: &[u8], scalar: fn([u8; 4]) -> u32) -> Option<String> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    bytes
        .chunks_exact(4)
        .map(|quad| char::from_u32(scalar([quad[0], quad[1], quad[2], quad[3]])))
        .collect()
}

const INDENT: &str = "  ";

fn write_indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str(INDENT);
    }
}

fn write_value(out: &mut String, value: &Value, options: JSONWritingOptions, level: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => write_number(out, number),
        Value::String(string) => write_string(out, string, options),
        Value::Array(items) => write_array(out, items, options, level),
        Value::Object(map) => write_object(out, map, options, level),
    }
}

fn write_array(out: &mut String, items: &[Value], options: JSONWritingOptions, level: usize) {
    out.push('[');
    if options.pretty_printed {
        out.push('\n');
        if items.is_empty() {
            out.push('\n');
        }
    }
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push(',');
            if options.pretty_printed {
                out.push('\n');
            }
        }
        if options.pretty_printed {
            write_indent(out, level + 1);
        }
        write_value(out, item, options, level + 1);
    }
    if options.pretty_printed {
        if !items.is_empty() {
            out.push('\n');
        }
        write_indent(out, level);
    }
    out.push(']');
}

fn write_object(out: &mut String, map: &Map<String, Value>, options: JSONWritingOptions, level: usize) {
    let mut entries: Vec<(&String, &Value)> = map.iter().collect();
    if options.sorted_keys {
        entries.sort_by(|(a, _), (b, _)| sorted_keys_order(a, b));
    }
    out.push('{');
    if options.pretty_printed {
        out.push('\n');
        if entries.is_empty() {
            out.push('\n');
        }
    }
    for (index, (key, item)) in entries.iter().enumerate() {
        if index > 0 {
            out.push(',');
            if options.pretty_printed {
                out.push('\n');
            }
        }
        if options.pretty_printed {
            write_indent(out, level + 1);
        }
        write_string(out, key, options);
        out.push_str(if options.pretty_printed { " : " } else { ":" });
        write_value(out, item, options, level + 1);
    }
    if options.pretty_printed {
        if !entries.is_empty() {
            out.push('\n');
        }
        write_indent(out, level);
    }
    out.push('}');
}

fn write_string(out: &mut String, string: &str, options: JSONWritingOptions) {
    out.push('"');
    for c in string.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' if !options.without_escaping_slashes => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_number(out: &mut String, number: &Number) {
    if let Some(unsigned) = number.as_u64() {
        out.push_str(&unsigned.to_string());
    } else if let Some(signed) = number.as_i64() {
        out.push_str(&signed.to_string());
    } else if let Some(double) = number.as_f64() {
        out.push_str(&format_g17(double));
    }
}

fn format_g17(value: f64) -> String {
    const PRECISION: i32 = 17;
    let scientific = format!("{:.*e}", (PRECISION - 1) as usize, value);
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        return scientific;
    };
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let mut out = String::from(sign);
    if (-4..PRECISION).contains(&exponent) {
        if exponent >= 0 {
            let split = (exponent + 1) as usize;
            out.push_str(&digits[..split]);
            push_fraction(&mut out, &digits[split..]);
        } else {
            out.push('0');
            let mut fraction = "0".repeat((-exponent - 1) as usize);
            fraction.push_str(&digits);
            push_fraction(&mut out, &fraction);
        }
    } else {
        out.push_str(&digits[..1]);
        push_fraction(&mut out, &digits[1..]);
        out.push('e');
        out.push(if exponent < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exponent.unsigned_abs()));
    }
    out
}

fn push_fraction(out: &mut String, fraction: &str) {
    let kept = fraction.trim_end_matches('0');
    if !kept.is_empty() {
        out.push('.');
        out.push_str(kept);
    }
}

fn sorted_keys_order(a: &str, b: &str) -> Ordering {
    let fold = CompareOptions::CASE_INSENSITIVE.union(CompareOptions::WIDTH_INSENSITIVE);
    let left: Vec<char> = folding(a, fold).chars().collect();
    let right: Vec<char> = folding(b, fold).chars().collect();
    numeric_order(&left, &right).then_with(|| forced_order(a, b))
}

fn numeric_order(left: &[char], right: &[char]) -> Ordering {
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        if left[i].is_ascii_digit() && right[j].is_ascii_digit() {
            let left_end = digit_run_end(left, i);
            let right_end = digit_run_end(right, j);
            let order = compare_digit_runs(&left[i..left_end], &right[j..right_end]);
            if order != Ordering::Equal {
                return order;
            }
            i = left_end;
            j = right_end;
        } else {
            let order = utf16_order(left[i], right[j]);
            if order != Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }
    (left.len() - i).cmp(&(right.len() - j))
}

fn digit_run_end(chars: &[char], start: usize) -> usize {
    chars[start..].iter().position(|c| !c.is_ascii_digit()).map_or(chars.len(), |offset| start + offset)
}

fn compare_digit_runs(left_run: &[char], right_run: &[char]) -> Ordering {
    let significant = |run: &[char]| -> usize { run.iter().position(|c| *c != '0').unwrap_or(run.len()) };
    let left = &left_run[significant(left_run)..];
    let right = &right_run[significant(right_run)..];
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

fn utf16_order(left: char, right: char) -> Ordering {
    let mut left_units = [0u16; 2];
    let mut right_units = [0u16; 2];
    left.encode_utf16(&mut left_units).iter().cmp(right.encode_utf16(&mut right_units).iter())
}

fn forced_order(a: &str, b: &str) -> Ordering {
    let swapped = |s: &str| -> Vec<u16> {
        s.chars()
            .flat_map(|c| {
                let flipped: String = if c.is_lowercase() {
                    c.to_uppercase().collect()
                } else if c.is_uppercase() {
                    c.to_lowercase().collect()
                } else {
                    c.to_string()
                };
                flipped.encode_utf16().collect::<Vec<u16>>()
            })
            .collect()
    };
    swapped(a).cmp(&swapped(b))
}
