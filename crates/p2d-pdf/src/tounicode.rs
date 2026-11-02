//! Reading a font's `ToUnicode` CMap: which character each character code draws.

use std::collections::HashMap;

use hayro_interpret::hayro_syntax::object::dict::keys::TO_UNICODE;
use hayro_interpret::hayro_syntax::object::{Dict, Stream};

use crate::edit::lexer::{Lexer, Value};

/// Code → character for every code of `font`'s `ToUnicode` map that stands for one character
/// (codes standing for ligatures or nothing are left out).
pub fn to_unicode_chars(font: &Dict<'_>) -> HashMap<u32, char> {
    let mut map = HashMap::new();
    let Some(data) = font.get::<Stream<'_>>(TO_UNICODE).and_then(|s| s.decoded().ok()) else {
        return map;
    };
    let code_of = |b: &[u8]| b.iter().fold(0u32, |acc, &x| (acc << 8) | x as u32);
    let single_char = |b: &[u8]| {
        let units: Vec<u16> = b
            .chunks(2)
            .map(|p| u16::from_be_bytes([p[0], *p.get(1).unwrap_or(&0)]))
            .collect();
        let s = String::from_utf16(&units).ok()?;
        let mut chars = s.chars();
        let c = chars.next()?;
        chars.next().is_none().then_some(c)
    };
    let mut lx = Lexer::new(&data);
    while let Some(op) = lx.next_op() {
        match op.operator.as_slice() {
            b"endbfchar" => {
                for pair in op.operands.chunks(2) {
                    if let [Value::Str { bytes: code, .. }, Value::Str { bytes: uni, .. }] = pair
                        && let Some(c) = single_char(uni)
                    {
                        map.insert(code_of(code), c);
                    }
                }
            }
            b"endbfrange" => {
                for triple in op.operands.chunks(3) {
                    let [Value::Str { bytes: lo, .. }, Value::Str { bytes: hi, .. }, dst] = triple else {
                        continue;
                    };
                    let (lo, hi) = (code_of(lo), code_of(hi));
                    for (k, code) in (lo..=hi.min(lo + 0xFFFF)).enumerate() {
                        let c = match dst {
                            Value::Str { bytes, .. } => {
                                let base = single_char(bytes).map(|c| c as u32);
                                base.and_then(|b| char::from_u32(b + k as u32))
                            }
                            Value::Array(items) => match items.get(k) {
                                Some(Value::Str { bytes, .. }) => single_char(bytes),
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some(c) = c {
                            map.insert(code, c);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    map
}
