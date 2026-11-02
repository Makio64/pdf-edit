//! Choosing the installed font that looks most like a PDF font the Mac does not have, by drawing
//! the glyphs the PDF uses in both and comparing their ink: same letter shapes, weight, x-height
//! and widths score high, whatever the fonts are called.

use p2d_ir::{FontIr, PathOp};

/// Installed families a missing font may be replaced with: the common Mac faces, covering
/// grotesque, geometric, humanist, rounded and condensed sans, old-style, transitional, modern
/// and slab serifs, and monospace.
pub const CANDIDATES: &[&str] = &[
    "Helvetica Neue",
    "Helvetica",
    "Arial",
    "Avenir Next",
    "Avenir",
    "Futura",
    "Gill Sans",
    "Optima",
    "Verdana",
    "Tahoma",
    "Trebuchet MS",
    "Lucida Grande",
    "PT Sans",
    "Skia",
    "Arial Rounded MT Bold",
    "Arial Narrow",
    "Avenir Next Condensed",
    "DIN Alternate",
    "DIN Condensed",
    "Georgia",
    "Times New Roman",
    "Times",
    "Palatino",
    "Baskerville",
    "Hoefler Text",
    "Didot",
    "Bodoni 72",
    "Big Caslon",
    "Charter",
    "Cochin",
    "PT Serif",
    "Rockwell",
    "American Typewriter",
    "Courier New",
    "Menlo",
    "PT Mono",
];

/// Candidates with serifs (slab serifs included).
pub const SERIF: &[&str] = &[
    "Georgia",
    "Times New Roman",
    "Times",
    "Palatino",
    "Baskerville",
    "Hoefler Text",
    "Didot",
    "Bodoni 72",
    "Big Caslon",
    "Charter",
    "Cochin",
    "PT Serif",
    "Rockwell",
    "American Typewriter",
    "Courier New",
];

/// What a candidate loses for being on the other side of the serif / sans divide than the PDF
/// font says it is: the outlines alone can miss small serifs.
pub const CLASS_PENALTY: f32 = 0.05;

const COLS: usize = 80;
const ROWS: usize = 88;
/// Grid cell in em units (1000 per em), over x ∈ [-100, 1100) and y ∈ [-300, 1020).
const CELL: f32 = 15.0;
const X0: f32 = -100.0;
const Y0: f32 = -300.0;

/// Where an outline (em units, y up) puts ink on a coarse grid over the em box.
pub fn ink(path: &[PathOp]) -> Vec<bool> {
    let mut segs: Vec<[f32; 4]> = Vec::new();
    let (mut start, mut cur) = ([0.0f32; 2], [0.0f32; 2]);
    let line = |segs: &mut Vec<[f32; 4]>, a: [f32; 2], b: [f32; 2]| {
        if a != b {
            segs.push([a[0], a[1], b[0], b[1]]);
        }
    };
    for op in path {
        match *op {
            PathOp::MoveTo(p) => {
                line(&mut segs, cur, start);
                start = p;
                cur = p;
            }
            PathOp::LineTo(p) => {
                line(&mut segs, cur, p);
                cur = p;
            }
            PathOp::QuadTo([cx, cy, x, y]) => {
                let mut prev = cur;
                for k in 1..=8 {
                    let t = k as f32 / 8.0;
                    let u = 1.0 - t;
                    let p = [
                        u * u * cur[0] + 2.0 * u * t * cx + t * t * x,
                        u * u * cur[1] + 2.0 * u * t * cy + t * t * y,
                    ];
                    line(&mut segs, prev, p);
                    prev = p;
                }
                cur = [x, y];
            }
            PathOp::CurveTo([ax, ay, bx, by, x, y]) => {
                let mut prev = cur;
                for k in 1..=12 {
                    let t = k as f32 / 12.0;
                    let u = 1.0 - t;
                    let p = [
                        u * u * u * cur[0] + 3.0 * u * u * t * ax + 3.0 * u * t * t * bx + t * t * t * x,
                        u * u * u * cur[1] + 3.0 * u * u * t * ay + 3.0 * u * t * t * by + t * t * t * y,
                    ];
                    line(&mut segs, prev, p);
                    prev = p;
                }
                cur = [x, y];
            }
            PathOp::Close => {
                line(&mut segs, cur, start);
                cur = start;
            }
        }
    }
    line(&mut segs, cur, start);

    // Non-zero winding, sampled at cell centres.
    let mut out = vec![false; COLS * ROWS];
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for row in 0..ROWS {
        let y = Y0 + (row as f32 + 0.5) * CELL;
        crossings.clear();
        for &[x0, y0, x1, y1] in &segs {
            if (y0 <= y) != (y1 <= y) {
                crossings.push((x0 + (y - y0) * (x1 - x0) / (y1 - y0), if y1 > y0 { 1 } else { -1 }));
            }
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut winding = 0;
        let mut k = 0;
        for col in 0..COLS {
            let x = X0 + (col as f32 + 0.5) * CELL;
            while k < crossings.len() && crossings[k].0 <= x {
                winding += crossings[k].1;
                k += 1;
            }
            out[row * COLS + col] = winding != 0;
        }
    }
    out
}

const ROW_MASK: u128 = (1u128 << COLS) - 1;

fn pack_rows(g: &[bool]) -> [u128; ROWS] {
    let mut rows = [0u128; ROWS];
    for (r, row_slice) in g.chunks_exact(COLS).take(ROWS).enumerate() {
        let mut bits = 0u128;
        for (c, &cell) in row_slice.iter().enumerate() {
            if cell {
                bits |= 1u128 << c;
            }
        }
        rows[r] = bits;
    }
    rows
}

/// The grid with every inked cell grown by one cell in each direction.
fn grown(a: &[u128; ROWS]) -> [u128; ROWS] {
    let mut horiz = [0u128; ROWS];
    for r in 0..ROWS {
        let b = a[r];
        horiz[r] = (b | (b << 1) | (b >> 1)) & ROW_MASK;
    }
    let mut out = [0u128; ROWS];
    for r in 0..ROWS {
        let mut m = horiz[r];
        if r > 0 {
            m |= horiz[r - 1];
        }
        if r + 1 < ROWS {
            m |= horiz[r + 1];
        }
        out[r] = m;
    }
    out
}

/// Width and height of the inked cells' bounding box, in cells.
fn ink_extent(g: &[u128; ROWS]) -> (usize, usize) {
    let mut all_cols = 0u128;
    let mut r0 = ROWS;
    let mut r1 = 0;
    for (r, &bits) in g.iter().enumerate() {
        if bits != 0 {
            all_cols |= bits;
            r0 = r0.min(r);
            r1 = r + 1;
        }
    }
    if all_cols == 0 {
        return (0, 0);
    }
    let c0 = all_cols.trailing_zeros() as usize;
    let c1 = 128 - all_cols.leading_zeros() as usize;
    (c1.saturating_sub(c0), r1.saturating_sub(r0))
}

/// How alike two glyphs' ink is, from 0 to 1: their outlines agree within one cell (each one's
/// ink lies near the other's, as an F-score), they have the same proportions (inked width and
/// height) and are as heavy (ink area ratio).
pub fn overlap(a: &[bool], b: &[bool]) -> f32 {
    let pa = pack_rows(a);
    let pb = pack_rows(b);
    let count = |g: &[u128; ROWS]| g.iter().map(|r| r.count_ones() as usize).sum::<usize>();
    let (na, nb) = (count(&pa), count(&pb));
    if na == 0 || nb == 0 {
        return if na == nb { 1.0 } else { 0.0 };
    }
    let near = |x: &[u128; ROWS], grown_y: &[u128; ROWS]| {
        x.iter()
            .zip(grown_y)
            .map(|(p, q)| (p & q).count_ones() as usize)
            .sum::<usize>() as f32
    };
    let precision = near(&pa, &grown(&pb)) / na as f32;
    let recall = near(&pb, &grown(&pa)) / nb as f32;
    let shape = if precision + recall > 0.0 { 2.0 * precision * recall / (precision + recall) } else { 0.0 };
    let ratio = |x: usize, y: usize| x.min(y) as f32 / x.max(y).max(1) as f32;
    let ((wa, ha), (wb, hb)) = (ink_extent(&pa), ink_extent(&pb));
    0.5 * shape + 0.2 * ratio(wa, wb) + 0.15 * ratio(ha, hb) + 0.15 * ratio(na, nb)
}

/// The glyphs of `font` worth comparing: letters and digits the document draws, with their
/// outline and advance (em units).
pub fn samples(font: &FontIr) -> Vec<(char, f32, Vec<bool>)> {
    let mut out: Vec<(char, f32, Vec<bool>)> = font
        .shapes
        .values()
        .filter(|s| s.unicode.is_alphanumeric() && !s.path.is_empty() && s.advance > 0.0)
        .map(|s| (s.unicode, s.advance, ink(&s.path)))
        .collect();
    out.sort_by_key(|(c, ..)| *c);
    out.dedup_by_key(|(c, ..)| *c);
    // Spread over the alphabet rather than the first few letters.
    let step = out.len().div_ceil(32).max(1);
    out.into_iter().step_by(step).collect()
}

/// Score of a candidate font for `samples`: mean ink overlap, less the mean relative difference of
/// advances (widths decide where words end). `draw` gives the candidate's outline and advance for
/// a character; a character it lacks scores nothing.
pub fn score(samples: &[(char, f32, Vec<bool>)], mut draw: impl FnMut(char) -> Option<(Vec<PathOp>, f32)>) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mut total = 0.0;
    for (c, advance, pdf_ink) in samples {
        if let Some((path, cand_advance)) = draw(*c) {
            let width_error = ((cand_advance - advance) / advance.max(1.0)).abs().min(1.0);
            total += overlap(pdf_ink, &ink(&path)) - 0.5 * width_error;
        }
    }
    total / samples.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<PathOp> {
        vec![
            PathOp::MoveTo([x0, y0]),
            PathOp::LineTo([x1, y0]),
            PathOp::LineTo([x1, y1]),
            PathOp::LineTo([x0, y1]),
            PathOp::Close,
        ]
    }

    #[test]
    fn test_ink_and_overlap() {
        let a = ink(&rect(100.0, 0.0, 500.0, 700.0));
        assert_eq!(overlap(&a, &a), 1.0);
        let filled = a.iter().filter(|x| **x).count();
        // 400 x 700 em units on 15-unit cells.
        assert!((filled as i32 - (400 * 700 / 225)).abs() < 60, "{filled}");
        let half = overlap(&a, &ink(&rect(100.0, 0.0, 500.0, 350.0)));
        let shifted = overlap(&a, &ink(&rect(110.0, 0.0, 510.0, 700.0)));
        let apart = overlap(&a, &ink(&rect(600.0, 0.0, 900.0, 700.0)));
        assert!(shifted > 0.95 && half < 0.8 && apart < half, "{shifted} {half} {apart}");
    }

    #[test]
    fn test_score_prefers_the_same_shape_and_width() {
        let samples = vec![('l', 300.0, ink(&rect(100.0, 0.0, 200.0, 700.0)))];
        let same = score(&samples, |_| Some((rect(100.0, 0.0, 200.0, 700.0), 300.0)));
        let wider = score(&samples, |_| Some((rect(100.0, 0.0, 200.0, 700.0), 450.0)));
        let bolder = score(&samples, |_| Some((rect(80.0, 0.0, 260.0, 700.0), 300.0)));
        let missing = score(&samples, |_| None);
        assert!(same > wider && same > bolder && bolder > missing, "{same} {wider} {bolder} {missing}");
    }
}
