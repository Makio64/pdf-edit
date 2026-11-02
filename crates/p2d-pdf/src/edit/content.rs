//! Walks a page content stream with just enough graphics and text state to know where every text
//! operator draws its first glyph and how many glyphs it draws, in the same order as the
//! extraction pass, and rewrites the operators holding glyphs that must disappear so those glyphs
//! are drawn in the invisible text mode. Rendering modes do not move glyphs, so every following
//! glyph keeps its exact position without knowing any glyph width. Text inside a form XObject drawn
//! once (also inside nested forms) is rewritten the same way, in a copy of the form for this page.

use std::collections::{BTreeMap, BTreeSet};

use hayro_interpret::hayro_syntax::object::dict::keys::{ENCODING, MATRIX, RESOURCES, SUBTYPE};
use hayro_interpret::hayro_syntax::object::{Dict, Name, Stream};
use hayro_interpret::hayro_syntax::page::Resources;
use kurbo::{Affine, Point};
use p2d_ir::GlyphIr;

use super::lexer::{Lexer, Value, fmt_num, write_hex_string};

/// How a font splits a string into character codes.
#[derive(Clone, Debug)]
enum CodeLen {
    One,
    Two,
    /// Codespace ranges of an embedded CMap: `(low, high)` byte strings.
    Ranges(Vec<(Vec<u8>, Vec<u8>)>),
}

impl CodeLen {
    fn of_font(font: &Dict<'_>) -> Option<CodeLen> {
        let subtype = font.get::<Name<'_>>(SUBTYPE)?;
        if subtype.as_ref() != b"Type0" {
            return Some(CodeLen::One);
        }
        if let Some(name) = font.get::<Name<'_>>(ENCODING) {
            let n = name.as_ref();
            let two =
                n.starts_with(b"Identity") || n.windows(4).any(|w| w == b"UCS2") || n.windows(5).any(|w| w == b"UTF16");
            return two.then_some(CodeLen::Two);
        }
        let cmap = font.get::<Stream<'_>>(ENCODING)?;
        let data = cmap.decoded().ok()?;
        let mut ranges = Vec::new();
        let mut lx = Lexer::new(&data);
        let mut in_space = false;
        let mut pending: Vec<Vec<u8>> = Vec::new();
        while let Some(op) = lx.next_op() {
            match op.operator.as_slice() {
                b"begincodespacerange" => {
                    in_space = true;
                    pending.clear();
                }
                b"endcodespacerange" => {
                    for v in &op.operands {
                        if let Value::Str { bytes, .. } = v {
                            pending.push(bytes.clone());
                        }
                    }
                    for pair in pending.chunks(2) {
                        if let [lo, hi] = pair {
                            ranges.push((lo.clone(), hi.clone()));
                        }
                    }
                    in_space = false;
                }
                _ if in_space => {}
                _ => {}
            }
        }
        (!ranges.is_empty()).then_some(CodeLen::Ranges(ranges))
    }

    /// Byte length of each code in `s`.
    fn split(&self, s: &[u8]) -> Vec<usize> {
        match self {
            CodeLen::One => vec![1; s.len()],
            CodeLen::Two => {
                let mut v = vec![2; s.len() / 2];
                if s.len() % 2 == 1 {
                    v.push(1);
                }
                v
            }
            CodeLen::Ranges(ranges) => {
                let mut out = Vec::new();
                let mut i = 0;
                while i < s.len() {
                    let len = (1..=4)
                        .find(|&n| {
                            i + n <= s.len()
                                && ranges.iter().any(|(lo, hi)| {
                                    lo.len() == n && (0..n).all(|k| lo[k] <= s[i + k] && s[i + k] <= hi[k])
                                })
                        })
                        .unwrap_or(1);
                    out.push(len);
                    i += len;
                }
                out
            }
        }
    }
}

#[derive(Clone)]
struct GState {
    ctm: Affine,
    font: Option<Vec<u8>>,
    font_size: f64,
    tr: i64,
    tc: f64,
    tw: f64,
    th: f64,
    tl: f64,
    ts: f64,
}

/// Result of rewriting one content stream.
pub struct Rewrite {
    pub content: Vec<u8>,
    /// Requested glyphs that are now drawn invisibly.
    pub hidden: BTreeSet<usize>,
    /// Font each hidden glyph was drawn with: the `/XObject` keys leading from the page to the
    /// form it is drawn in (empty: the page itself), and its `/Font` key there.
    pub fonts: BTreeMap<usize, (Vec<Vec<u8>>, Vec<u8>)>,
    /// Form XObjects whose text now hides glyphs: the `/XObject` keys leading to each from the
    /// page, and its new (decoded) content.
    pub forms: Vec<(Vec<Vec<u8>>, Vec<u8>)>,
    /// How each hidden glyph is set: its font size in visual page points (`Tf` times the text and
    /// graphics matrices) and the direction of its text space in visual page coordinates (the
    /// linear part of those matrices at unit size: `[1, 0, 0, -1]` for upright text).
    pub setting: BTreeMap<usize, (f64, [f64; 4])>,
    /// The character code (bytes) each hidden glyph was drawn with.
    pub codes: BTreeMap<usize, Vec<u8>>,
}

/// Names of the XObjects `content` draws exactly once.
fn drawn_once(content: &[u8]) -> BTreeSet<Vec<u8>> {
    let mut uses: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
    let mut lx = Lexer::new(content);
    while let Some(op) = lx.next_op() {
        if let (b"Do", Some(Value::Name(name))) = (op.operator.as_slice(), op.operands.first()) {
            *uses.entry(name.clone()).or_default() += 1;
        }
    }
    uses.into_iter().filter(|(_, n)| *n == 1).map(|(k, _)| k).collect()
}

pub struct Walker<'g> {
    glyphs: &'g [GlyphIr],
    hide: &'g BTreeSet<usize>,
    page_tf: Affine,
    next_glyph: usize,
    aligned: bool,
    hidden: BTreeSet<usize>,
    fonts: BTreeMap<usize, (Vec<Vec<u8>>, Vec<u8>)>,
    forms: Vec<(Vec<Vec<u8>>, Vec<u8>)>,
    setting: BTreeMap<usize, (f64, [f64; 4])>,
    codes: BTreeMap<usize, Vec<u8>>,
    /// `/XObject` keys from the page to the form being walked.
    form_path: Vec<Vec<u8>>,
}

const MAX_FORM_DEPTH: usize = 6;
/// How far (pt) an operator's first glyph may sit from the extracted one before the walk is
/// considered out of step with the extraction.
const POSITION_TOLERANCE_PT: f64 = 0.75;

impl<'g> Walker<'g> {
    pub fn new(glyphs: &'g [GlyphIr], hide: &'g BTreeSet<usize>, page_tf: Affine) -> Self {
        Self {
            glyphs,
            hide,
            page_tf,
            next_glyph: 0,
            aligned: true,
            hidden: BTreeSet::new(),
            fonts: BTreeMap::new(),
            forms: Vec::new(),
            setting: BTreeMap::new(),
            codes: BTreeMap::new(),
            form_path: Vec::new(),
        }
    }

    /// Rewrite the page content so the requested glyphs become invisible.
    pub fn rewrite_page(mut self, content: &[u8], resources: &Resources<'_>) -> Rewrite {
        let mut out = Vec::with_capacity(content.len() + 256);
        let gs = GState {
            ctm: Affine::IDENTITY,
            font: None,
            font_size: 0.0,
            tr: 0,
            tc: 0.0,
            tw: 0.0,
            th: 1.0,
            tl: 0.0,
            ts: 0.0,
        };
        self.walk(content, resources, gs, 0, Some(&mut out));
        Rewrite {
            content: out,
            hidden: self.hidden,
            fonts: self.fonts,
            forms: self.forms,
            setting: self.setting,
            codes: self.codes,
        }
    }

    fn walk(
        &mut self,
        content: &[u8],
        resources: &Resources<'_>,
        start: GState,
        depth: usize,
        mut out: Option<&mut Vec<u8>>,
    ) {
        let mut stack: Vec<GState> = Vec::new();
        let mut gs = start;
        // Only a form drawn once here can get a rewritten copy of its own.
        let once = if out.is_some() { drawn_once(content) } else { BTreeSet::new() };
        let mut tm = Affine::IDENTITY;
        let mut tlm = Affine::IDENTITY;
        let mut copied_to = 0usize;
        let mut lx = Lexer::new(content);

        while let Some(op) = lx.next_op() {
            let num = |i: usize| op.operands.get(i).and_then(Value::as_f64).unwrap_or(0.0);
            let mut replacement: Option<Vec<u8>> = None;
            match op.operator.as_slice() {
                b"q" => stack.push(gs.clone()),
                b"Q" => {
                    if let Some(g) = stack.pop() {
                        gs = g;
                    }
                }
                b"cm" => gs.ctm *= Affine::new([num(0), num(1), num(2), num(3), num(4), num(5)]),
                b"BT" => {
                    tm = Affine::IDENTITY;
                    tlm = Affine::IDENTITY;
                }
                b"Tf" => {
                    if let Some(Value::Name(n)) = op.operands.first() {
                        gs.font = Some(n.clone());
                    }
                    gs.font_size = num(1);
                }
                b"Tr" => gs.tr = num(0) as i64,
                b"Tc" => gs.tc = num(0),
                b"Tw" => gs.tw = num(0),
                b"Tz" => gs.th = num(0) / 100.0,
                b"TL" => gs.tl = num(0),
                b"Ts" => gs.ts = num(0),
                b"Td" => {
                    tlm *= Affine::translate((num(0), num(1)));
                    tm = tlm;
                }
                b"TD" => {
                    gs.tl = -num(1);
                    tlm *= Affine::translate((num(0), num(1)));
                    tm = tlm;
                }
                b"Tm" => {
                    tlm = Affine::new([num(0), num(1), num(2), num(3), num(4), num(5)]);
                    tm = tlm;
                }
                b"T*" => {
                    tlm *= Affine::translate((0.0, -gs.tl));
                    tm = tlm;
                }
                b"Tj" | b"TJ" | b"'" | b"\"" => {
                    let mut prefix = Vec::new();
                    if op.operator == b"\"" {
                        gs.tw = num(0);
                        gs.tc = num(1);
                        prefix.extend_from_slice(format!("{} Tw {} Tc ", fmt_num(num(0)), fmt_num(num(1))).as_bytes());
                    }
                    if op.operator == b"'" || op.operator == b"\"" {
                        tlm *= Affine::translate((0.0, -gs.tl));
                        tm = tlm;
                        prefix.extend_from_slice(b"T* ");
                    }
                    let items: Vec<Value> = match (op.operator.as_slice(), op.operands.last()) {
                        (b"TJ", Some(Value::Array(a))) => a.clone(),
                        (_, Some(s @ Value::Str { .. })) => vec![s.clone()],
                        _ => Vec::new(),
                    };
                    let m = (self.page_tf * gs.ctm * tlm).as_coeffs();
                    // Each axis normalised on its own: the direction keeps rotation, flip and slant,
                    // while a horizontal stretch is set again by `Tz`.
                    let (sx, sy) = (m[0].hypot(m[1]), m[2].hypot(m[3]));
                    let unit = if sx > 0.0 && sy > 0.0 {
                        [m[0] / sx, m[1] / sx, m[2] / sy, m[3] / sy]
                    } else {
                        [1.0, 0.0, 0.0, -1.0]
                    };
                    let setting = (gs.font_size.abs() * sy, unit);
                    let rewritten = self.show_text(&items, &gs, tm, setting, resources, out.is_some());
                    // The glyph advances are unknown here, but `tm` only matters for the first glyph
                    // of the next operator, which always repositions explicitly or continues from
                    // here; continuing without a position change is caught by the position check.
                    if let Some(body) = rewritten {
                        let mut r = prefix;
                        r.extend_from_slice(&body);
                        replacement = Some(r);
                    }
                    // Text continues after this run; mark that the next operator's first glyph
                    // position is unknown unless it repositions.
                    tm = Affine::new([f64::NAN; 6]);
                }
                b"Do" => {
                    if let Some(Value::Name(name)) = op.operands.first() {
                        self.walk_form(name, resources, &gs, depth, once.contains(name));
                    }
                }
                _ => {}
            }
            if let (Some(buf), Some(rep)) = (out.as_deref_mut(), replacement) {
                buf.extend_from_slice(&content[copied_to..op.range.start]);
                buf.extend_from_slice(&rep);
                copied_to = op.range.end;
            }
        }
        if let Some(buf) = out {
            buf.extend_from_slice(&content[copied_to..]);
        }
    }

    /// Walk a form XObject; with `rewrite`, keep a copy of its content hiding the requested glyphs.
    fn walk_form(&mut self, name: &[u8], resources: &Resources<'_>, gs: &GState, depth: usize, rewrite: bool) {
        if depth >= MAX_FORM_DEPTH {
            return;
        }
        let Some(stream) = resources.get_x_object(&Name::new_unescaped(name)) else {
            return;
        };
        let dict = stream.dict();
        if dict.get::<Name<'_>>(SUBTYPE).is_none_or(|s| s.as_ref() != b"Form") {
            return;
        }
        let Ok(data) = stream.decoded() else {
            return;
        };
        let matrix = dict
            .get::<hayro_interpret::hayro_syntax::object::Array<'_>>(MATRIX)
            .map(|a| a.iter::<f64>().collect::<Vec<f64>>())
            .filter(|m| m.len() == 6)
            .map(|m| Affine::new([m[0], m[1], m[2], m[3], m[4], m[5]]))
            .unwrap_or(Affine::IDENTITY);
        let form_resources = dict
            .get::<Dict<'_>>(RESOURCES)
            .map(Resources::new)
            .unwrap_or_else(|| resources.clone());
        let mut inner = gs.clone();
        inner.ctm *= matrix;
        self.form_path.push(name.to_vec());
        if rewrite {
            let hidden_before = self.hidden.len();
            let mut copy = Vec::with_capacity(data.len() + 64);
            self.walk(&data, &form_resources, inner, depth + 1, Some(&mut copy));
            if self.hidden.len() > hidden_before {
                self.forms.push((self.form_path.clone(), copy));
            }
        } else {
            self.walk(&data, &form_resources, inner, depth + 1, None);
        }
        self.form_path.pop();
    }

    /// Count (and check) the glyphs one text operator draws; return its rewrite when some of them
    /// must be hidden and the operator can be rewritten.
    fn show_text(
        &mut self,
        items: &[Value],
        gs: &GState,
        tm: Affine,
        setting: (f64, [f64; 4]),
        resources: &Resources<'_>,
        rewritable: bool,
    ) -> Option<Vec<u8>> {
        let code_len = gs
            .font
            .as_ref()
            .and_then(|n| resources.get_font(&Name::new_unescaped(n)))
            .and_then(|f| CodeLen::of_font(&f));
        let Some(code_len) = code_len else {
            // Unknown encoding: glyph counts cannot be trusted from here on.
            self.aligned = false;
            return None;
        };

        let codes: Vec<Vec<usize>> = items
            .iter()
            .map(|v| match v {
                Value::Str { bytes, .. } => code_len.split(bytes),
                _ => Vec::new(),
            })
            .collect();
        let n_codes: usize = codes.iter().map(Vec::len).sum();
        let copies = match gs.tr {
            7 => 0,
            2 | 6 => 2,
            _ => 1,
        };
        if n_codes == 0 || copies == 0 {
            return None;
        }

        let first = self.next_glyph;
        self.next_glyph += n_codes * copies;
        if !self.aligned {
            return None;
        }
        // The first glyph lands at the text origin shifted by the rise, unless a previous text
        // operator left the position unknown (then only the count is trusted).
        let origin = self.page_tf * gs.ctm * tm * Point::new(0.0, gs.ts);
        if let Some(g) = self.glyphs.get(first) {
            let known = origin.x.is_finite() && origin.y.is_finite();
            if known
                && ((origin.x - g.bbox.x0 as f64).abs() > POSITION_TOLERANCE_PT
                    || (origin.y - g.baseline_y as f64).abs() > POSITION_TOLERANCE_PT)
            {
                self.aligned = false;
                return None;
            }
        } else {
            self.aligned = false;
            return None;
        }

        let hide_code = |i: usize| (0..copies).any(|c| self.hide.contains(&(first + c * n_codes + i)));
        if !(0..n_codes).any(hide_code) || !rewritable {
            return None;
        }

        // Split every string at the boundaries between hidden and visible codes, emitting the
        // hidden chunks between `Tr` switches. Spacing numbers stay with the chunk they follow.
        let hidden_tr = if gs.tr >= 4 { 7 } else { 3 };
        let mut out = Vec::new();
        let mut chunk: Vec<u8> = Vec::new();
        let mut chunk_hidden = false;
        let mut chunk_has_glyph = false;
        let flush = |out: &mut Vec<u8>, chunk: &mut Vec<u8>, hidden: bool, has_glyph: bool| {
            if chunk.is_empty() {
                return;
            }
            if hidden && has_glyph {
                out.extend_from_slice(format!("{hidden_tr} Tr [").as_bytes());
                out.extend_from_slice(chunk);
                out.extend_from_slice(format!("] TJ {} Tr ", gs.tr).as_bytes());
            } else {
                out.push(b'[');
                out.extend_from_slice(chunk);
                out.extend_from_slice(b"] TJ ");
            }
            chunk.clear();
        };
        let mut code_idx = 0usize;
        for (item, lens) in items.iter().zip(&codes) {
            match item {
                Value::Str { bytes, .. } => {
                    let mut pos = 0;
                    let mut run: Vec<u8> = Vec::new();
                    for &len in lens {
                        let hidden = hide_code(code_idx);
                        if hidden != chunk_hidden && (chunk_has_glyph || !run.is_empty()) {
                            if !run.is_empty() {
                                write_hex_string(&run, &mut chunk);
                                run.clear();
                            }
                            flush(&mut out, &mut chunk, chunk_hidden, true);
                        }
                        chunk_hidden = hidden;
                        run.extend_from_slice(&bytes[pos..(pos + len).min(bytes.len())]);
                        chunk_has_glyph = true;
                        pos += len;
                        code_idx += 1;
                    }
                    if !run.is_empty() {
                        write_hex_string(&run, &mut chunk);
                        chunk.push(b' ');
                    }
                }
                Value::Number(n) => {
                    chunk.extend_from_slice(fmt_num(*n).as_bytes());
                    chunk.push(b' ');
                }
                _ => {}
            }
        }
        flush(&mut out, &mut chunk, chunk_hidden, chunk_has_glyph);
        let code_bytes: Vec<&[u8]> = items
            .iter()
            .zip(&codes)
            .flat_map(|(item, lens)| {
                let bytes: &[u8] = match item {
                    Value::Str { bytes, .. } => bytes,
                    _ => &[],
                };
                lens.iter().scan(0usize, move |pos, &len| {
                    let code = bytes.get(*pos..(*pos + len).min(bytes.len())).unwrap_or(&[]);
                    *pos += len;
                    Some(code)
                })
            })
            .collect();
        for i in 0..n_codes {
            for c in 0..copies {
                let idx = first + c * n_codes + i;
                if self.hide.contains(&idx) {
                    self.hidden.insert(idx);
                    self.setting.insert(idx, setting);
                    if let Some(code) = code_bytes.get(i) {
                        self.codes.insert(idx, code.to_vec());
                    }
                    if let Some(font) = &gs.font {
                        self.fonts.insert(idx, (self.form_path.clone(), font.clone()));
                    }
                }
            }
        }
        Some(out)
    }
}
