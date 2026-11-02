//! Minimal PDF tokenizer that keeps byte ranges, so unchanged operators and dictionary entries can
//! be copied verbatim while only the edited ones are re-serialized.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Number(f64),
    /// Name without its leading slash, `#xx` escapes still in place.
    Name(Vec<u8>),
    /// Decoded string bytes; `hex` tells which syntax it was written in.
    Str {
        bytes: Vec<u8>,
        hex: bool,
    },
    Array(Vec<Value>),
    Dict(Vec<(Vec<u8>, Value)>),
    /// Indirect reference `number generation R`.
    Ref(u32, u16),
    /// `true`, `false`, `null` or any other bare keyword used as an operand.
    Keyword(Vec<u8>),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
}

/// One content-stream operation: operands, operator and the byte range covering both.
#[derive(Debug, Clone)]
pub struct Op {
    pub operator: Vec<u8>,
    pub operands: Vec<Value>,
    pub range: Range<usize>,
}

pub struct Lexer<'a> {
    data: &'a [u8],
    pub pos: usize,
}

fn is_white(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'\x0c' | b'\0')
}

fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Value(Value),
    ArrayOpen,
    ArrayClose,
    DictOpen,
    DictClose,
    Keyword(Vec<u8>),
}

impl<'a> Lexer<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn skip_white(&mut self) {
        while self.pos < self.data.len() {
            let b = self.data[self.pos];
            if is_white(b) {
                self.pos += 1;
            } else if b == b'%' {
                while self.pos < self.data.len() && !matches!(self.data[self.pos], b'\r' | b'\n') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn token(&mut self) -> Option<(Tok, usize)> {
        self.skip_white();
        let start = self.pos;
        let b = *self.data.get(self.pos)?;
        let tok = match b {
            b'[' => {
                self.pos += 1;
                Tok::ArrayOpen
            }
            b']' => {
                self.pos += 1;
                Tok::ArrayClose
            }
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                Tok::DictOpen
            }
            b'>' if self.data.get(self.pos + 1) == Some(&b'>') => {
                self.pos += 2;
                Tok::DictClose
            }
            b'<' => Tok::Value(self.hex_string()),
            b'(' => Tok::Value(self.literal_string()),
            b'/' => {
                self.pos += 1;
                let s = self.pos;
                while self.pos < self.data.len() && !is_white(self.data[self.pos]) && !is_delim(self.data[self.pos]) {
                    self.pos += 1;
                }
                Tok::Value(Value::Name(self.data[s..self.pos].to_vec()))
            }
            b'{' | b'}' | b')' | b'>' => {
                self.pos += 1;
                Tok::Keyword(vec![b])
            }
            _ => {
                while self.pos < self.data.len() && !is_white(self.data[self.pos]) && !is_delim(self.data[self.pos]) {
                    self.pos += 1;
                }
                let word = &self.data[start..self.pos];
                match std::str::from_utf8(word).ok().and_then(|s| s.parse::<f64>().ok()) {
                    Some(n) if word.iter().all(|c| matches!(c, b'0'..=b'9' | b'.' | b'-' | b'+')) => {
                        Tok::Value(Value::Number(n))
                    }
                    _ => Tok::Keyword(word.to_vec()),
                }
            }
        };
        Some((tok, start))
    }

    fn hex_string(&mut self) -> Value {
        self.pos += 1;
        let mut nibbles = Vec::new();
        while self.pos < self.data.len() && self.data[self.pos] != b'>' {
            let c = self.data[self.pos];
            if let Some(v) = (c as char).to_digit(16) {
                nibbles.push(v as u8);
            }
            self.pos += 1;
        }
        self.pos += 1;
        if nibbles.len() % 2 == 1 {
            nibbles.push(0);
        }
        Value::Str {
            bytes: nibbles.chunks(2).map(|p| (p[0] << 4) | p[1]).collect(),
            hex: true,
        }
    }

    fn literal_string(&mut self) -> Value {
        self.pos += 1;
        let mut out = Vec::new();
        let mut depth = 1;
        while self.pos < self.data.len() {
            let c = self.data[self.pos];
            self.pos += 1;
            match c {
                b'\\' => {
                    let Some(&e) = self.data.get(self.pos) else { break };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match self.data.get(self.pos) {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        b'\r' => {
                            if self.data.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                _ => out.push(c),
            }
        }
        Value::Str { bytes: out, hex: false }
    }

    /// Read one complete value (nested arrays and dictionaries included), or a keyword.
    fn value(&mut self) -> Option<(Result<Value, Vec<u8>>, usize)> {
        let (tok, start) = self.token()?;
        let v = match tok {
            Tok::Value(Value::Number(n)) => {
                // `n g R` is one value: an indirect reference.
                let save = self.pos;
                match (self.token(), self.token()) {
                    (Some((Tok::Value(Value::Number(g)), _)), Some((Tok::Keyword(k), _))) if k == b"R" => {
                        Ok(Value::Ref(n as u32, g as u16))
                    }
                    _ => {
                        self.pos = save;
                        Ok(Value::Number(n))
                    }
                }
            }
            Tok::Value(v) => Ok(v),
            Tok::Keyword(k) => Err(k),
            Tok::ArrayOpen => {
                let mut items = Vec::new();
                loop {
                    self.skip_white();
                    if self.data.get(self.pos) == Some(&b']') {
                        self.pos += 1;
                        break;
                    }
                    match self.value()? {
                        (Ok(v), _) => items.push(v),
                        (Err(k), _) => items.push(Value::Keyword(k)),
                    }
                }
                Ok(Value::Array(items))
            }
            Tok::DictOpen => {
                let mut entries = Vec::new();
                loop {
                    self.skip_white();
                    if self.data.get(self.pos) == Some(&b'>') && self.data.get(self.pos + 1) == Some(&b'>') {
                        self.pos += 2;
                        break;
                    }
                    let key = match self.value()? {
                        (Ok(Value::Name(n)), _) => n,
                        _ => return None,
                    };
                    let val = match self.value()? {
                        (Ok(v), _) => v,
                        (Err(k), _) => Value::Keyword(k),
                    };
                    entries.push((key, val));
                }
                Ok(Value::Dict(entries))
            }
            Tok::ArrayClose | Tok::DictClose => return None,
        };
        Some((v, start))
    }

    /// Next content-stream operation, with inline images (`BI … ID … EI`) kept as one operation.
    pub fn next_op(&mut self) -> Option<Op> {
        let mut operands = Vec::new();
        let mut start: Option<usize> = None;
        loop {
            let (v, s) = self.value()?;
            start.get_or_insert(s);
            match v {
                Ok(val) => operands.push(val),
                Err(op) => {
                    if op == b"BI" {
                        self.skip_inline_image();
                    }
                    return Some(Op {
                        operator: op,
                        operands,
                        range: start.unwrap_or(s)..self.pos,
                    });
                }
            }
        }
    }

    fn skip_inline_image(&mut self) {
        // Parameters up to `ID`, then binary data up to a whitespace-delimited `EI`.
        while let Some((tok, _)) = self.token() {
            if tok == Tok::Keyword(b"ID".to_vec()) {
                break;
            }
        }
        self.pos += 1;
        while self.pos + 2 <= self.data.len() {
            if self.data[self.pos] == b'E'
                && self.data.get(self.pos + 1) == Some(&b'I')
                && self.pos > 0
                && is_white(self.data[self.pos - 1])
                && self.data.get(self.pos + 2).is_none_or(|&b| is_white(b) || is_delim(b))
            {
                self.pos += 2;
                return;
            }
            self.pos += 1;
        }
        self.pos = self.data.len();
    }
}

/// Top-level entries of a dictionary (`<< … >>`): key (without slash) and the byte range of its
/// value inside `data`, an indirect reference `n g R` counting as one value.
pub fn dict_entries(data: &[u8]) -> Option<Vec<(Vec<u8>, Range<usize>)>> {
    let mut lx = Lexer::new(data);
    match lx.token()? {
        (Tok::DictOpen, _) => {}
        _ => return None,
    }
    let mut out = Vec::new();
    loop {
        lx.skip_white();
        if data.get(lx.pos) == Some(&b'>') && data.get(lx.pos + 1) == Some(&b'>') {
            return Some(out);
        }
        let key = match lx.token()? {
            (Tok::Value(Value::Name(n)), _) => n,
            _ => return None,
        };
        let (_, start) = lx.value()?;
        out.push((key, start..lx.pos));
    }
}

/// Parse an indirect reference value (`12 0 R`).
pub fn parse_ref(raw: &[u8]) -> Option<(u32, u16)> {
    let s = std::str::from_utf8(raw).ok()?;
    let mut it = s.split_whitespace();
    let num = it.next()?.parse().ok()?;
    let generation = it.next()?.parse().ok()?;
    (it.next()? == "R").then_some((num, generation))
}

/// Serialize a string operand as a hex string.
pub fn write_hex_string(bytes: &[u8], out: &mut Vec<u8>) {
    out.push(b'<');
    for b in bytes {
        out.extend_from_slice(format!("{b:02X}").as_bytes());
    }
    out.push(b'>');
}

/// Serialize a number the way PDF expects (no exponent, trimmed).
pub fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e12 {
        format!("{}", n as i64)
    } else {
        let s = format!("{n:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ops_and_ranges() {
        let data = b"q 1 0 0 1 10 20 cm BT /F1 12 Tf (Hi\\) there) Tj [(A) -120 <4243>] TJ ET Q";
        let mut lx = Lexer::new(data);
        let mut ops = Vec::new();
        while let Some(op) = lx.next_op() {
            ops.push(op);
        }
        let names: Vec<&[u8]> = ops.iter().map(|o| o.operator.as_slice()).collect();
        assert_eq!(names, vec![&b"q"[..], b"cm", b"BT", b"Tf", b"Tj", b"TJ", b"ET", b"Q"]);
        assert_eq!(&data[ops[4].range.clone()], &b"(Hi\\) there) Tj"[..]);
        match &ops[4].operands[0] {
            Value::Str { bytes, .. } => assert_eq!(bytes, b"Hi) there"),
            other => panic!("unexpected {other:?}"),
        }
        match &ops[5].operands[0] {
            Value::Array(items) => {
                assert_eq!(items.len(), 3);
                assert_eq!(
                    items[2],
                    Value::Str {
                        bytes: b"BC".to_vec(),
                        hex: true
                    }
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn test_inline_image_is_one_op() {
        let data = b"q BI /W 2 /H 1 /BPC 8 /CS /G ID \x00EI\xff EI Q";
        let mut lx = Lexer::new(data);
        let ops: Vec<Vec<u8>> = std::iter::from_fn(|| lx.next_op()).map(|o| o.operator).collect();
        assert_eq!(ops, vec![b"q".to_vec(), b"BI".to_vec(), b"Q".to_vec()]);
    }

    #[test]
    fn test_dict_entries_with_refs() {
        let data =
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Rotate 0 >>";
        let entries = dict_entries(data).unwrap();
        let keys: Vec<&[u8]> = entries.iter().map(|(k, _)| k.as_slice()).collect();
        assert_eq!(
            keys,
            vec![&b"Type"[..], b"Parent", b"MediaBox", b"Resources", b"Rotate"]
        );
        assert_eq!(&data[entries[1].1.clone()], b"2 0 R");
        assert_eq!(parse_ref(&data[entries[1].1.clone()]), Some((2, 0)));
        assert_eq!(&data[entries[4].1.clone()], b"0");
    }
}
