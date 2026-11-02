//! Incremental update: the original file is kept byte for byte and new or replaced objects are
//! appended after it, followed by a cross-reference section that chains to the original one.

use super::lexer::dict_entries;

/// What the original file's last trailer says.
pub struct TrailerInfo {
    pub size: u32,
    pub root: Vec<u8>,
    pub info: Option<Vec<u8>>,
    pub id: Option<Vec<u8>>,
    pub encrypted: bool,
    pub prev_xref: usize,
    pub uses_xref_stream: bool,
}

/// Read the trailer the reader starts from (`startxref` at the end of the file).
pub fn read_trailer(pdf: &[u8]) -> Option<TrailerInfo> {
    let tail_start = pdf.len().saturating_sub(4096);
    let tail = &pdf[tail_start..];
    let at = tail.windows(9).rposition(|w| w == b"startxref")? + tail_start + 9;
    let digits: String = pdf[at..]
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take_while(|b| b.is_ascii_digit())
        .map(|&b| b as char)
        .collect();
    let prev_xref: usize = digits.parse().ok()?;
    let section = pdf.get(prev_xref..)?;
    let trimmed = section.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(0);
    let uses_xref_stream = !section[trimmed..].starts_with(b"xref");
    let dict_start = if uses_xref_stream {
        prev_xref + section.windows(2).position(|w| w == b"<<")?
    } else {
        let t = section.windows(7).position(|w| w == b"trailer")?;
        prev_xref + t + 7 + section[t + 7..].windows(2).position(|w| w == b"<<")?
    };
    let dict_bytes = &pdf[dict_start..];
    let entries = dict_entries(dict_bytes)?;
    let get = |key: &[u8]| {
        entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, r)| dict_bytes[r.clone()].to_vec())
    };
    let size = get(b"Size")
        .and_then(|v| String::from_utf8(v).ok())
        .and_then(|s| s.trim().parse().ok())?;
    Some(TrailerInfo {
        size,
        root: get(b"Root")?,
        info: get(b"Info"),
        id: get(b"ID"),
        encrypted: get(b"Encrypt").is_some(),
        prev_xref,
        uses_xref_stream,
    })
}

pub struct IncrementalUpdate<'a> {
    original: &'a [u8],
    trailer: TrailerInfo,
    next_number: u32,
    objects: Vec<(u32, u16, Vec<u8>)>,
}

impl<'a> IncrementalUpdate<'a> {
    pub fn new(original: &'a [u8], trailer: TrailerInfo) -> Self {
        Self {
            original,
            next_number: trailer.size.max(1),
            trailer,
            objects: Vec::new(),
        }
    }

    /// Reserve the number of a new object.
    pub fn allocate(&mut self) -> u32 {
        let n = self.next_number;
        self.next_number += 1;
        n
    }

    /// Write `body` (everything between `obj` and `endobj`) as object `number generation`.
    pub fn set_object(&mut self, number: u32, generation: u16, body: Vec<u8>) {
        self.objects.retain(|(n, _, _)| *n != number);
        self.objects.push((number, generation, body));
    }

    pub fn add_object(&mut self, body: Vec<u8>) -> u32 {
        let n = self.allocate();
        self.set_object(n, 0, body);
        n
    }

    pub fn finish(mut self) -> Vec<u8> {
        let mut out = self.original.to_vec();
        if !out.ends_with(b"\n") {
            out.push(b'\n');
        }
        self.objects.sort_by_key(|(n, _, _)| *n);
        let mut offsets: Vec<(u32, u16, usize)> = Vec::with_capacity(self.objects.len());
        for (num, generation, body) in &self.objects {
            offsets.push((*num, *generation, out.len()));
            out.extend_from_slice(format!("{num} {generation} obj\n").as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let size = self.next_number.max(self.trailer.size);
        let mut trailer_entries = format!("/Size {size} /Root {}", String::from_utf8_lossy(&self.trailer.root));
        if let Some(info) = &self.trailer.info {
            trailer_entries.push_str(&format!(" /Info {}", String::from_utf8_lossy(info)));
        }
        if let Some(id) = &self.trailer.id {
            trailer_entries.push_str(&format!(" /ID {}", String::from_utf8_lossy(id)));
        }
        trailer_entries.push_str(&format!(" /Prev {}", self.trailer.prev_xref));

        if self.trailer.uses_xref_stream {
            // The section itself is an object: include its own entry.
            let xref_num = size;
            let xref_offset = out.len();
            offsets.push((xref_num, 0, xref_offset));
            let mut index = String::new();
            let mut rows = Vec::new();
            for group in subsections(&offsets) {
                index.push_str(&format!("{} {} ", group[0].0, group.len()));
                for (_, generation, off) in group {
                    rows.push(1u8);
                    rows.extend_from_slice(&(*off as u32).to_be_bytes());
                    rows.extend_from_slice(&generation.to_be_bytes());
                }
            }
            out.extend_from_slice(
                format!(
                    "{xref_num} 0 obj\n<< /Type /XRef {} /Size {} /Index [{}] /W [1 4 2] /Length {} >>\nstream\n",
                    trailer_entries.replacen(&format!("/Size {size}"), "", 1),
                    xref_num + 1,
                    index.trim_end(),
                    rows.len()
                )
                .as_bytes(),
            );
            out.extend_from_slice(&rows);
            out.extend_from_slice(b"\nendstream\nendobj\n");
            out.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF\n").as_bytes());
        } else {
            let xref_offset = out.len();
            out.extend_from_slice(b"xref\n");
            for group in subsections(&offsets) {
                out.extend_from_slice(format!("{} {}\n", group[0].0, group.len()).as_bytes());
                for (_, generation, off) in group {
                    out.extend_from_slice(format!("{off:010} {generation:05} n \n").as_bytes());
                }
            }
            out.extend_from_slice(
                format!("trailer\n<< {trailer_entries} >>\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes(),
            );
        }
        out
    }
}

/// Group sorted `(number, generation, offset)` entries into runs of consecutive object numbers.
fn subsections(entries: &[(u32, u16, usize)]) -> Vec<&[(u32, u16, usize)]> {
    let mut groups = Vec::new();
    let mut start = 0;
    for i in 1..=entries.len() {
        if i == entries.len() || entries[i].0 != entries[i - 1].0 + 1 {
            groups.push(&entries[start..i]);
            start = i;
        }
    }
    groups
}

/// Rebuild a dictionary from its raw bytes, replacing (or adding) some entries and dropping others.
/// Untouched values are copied verbatim, so references keep pointing at the original objects.
pub fn patch_dict(raw: &[u8], replace: &[(&[u8], Vec<u8>)], drop: &[&[u8]]) -> Option<Vec<u8>> {
    let entries = dict_entries(raw)?;
    let mut out = b"<<".to_vec();
    for (key, range) in &entries {
        if drop.contains(&key.as_slice()) || replace.iter().any(|(k, _)| *k == key.as_slice()) {
            continue;
        }
        out.push(b'/');
        out.extend_from_slice(key);
        out.push(b' ');
        out.extend_from_slice(&raw[range.clone()]);
        out.push(b' ');
    }
    for (key, value) in replace {
        out.push(b'/');
        out.extend_from_slice(key);
        out.push(b' ');
        out.extend_from_slice(value);
        out.push(b' ');
    }
    out.extend_from_slice(b">>");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_patch_dict_keeps_other_entries() {
        let raw = b"<< /Type /Page /Parent 3 0 R /Contents 4 0 R /MediaBox [0 0 10 10] >>";
        let out = patch_dict(raw, &[(b"Contents", b"[4 0 R 9 0 R]".to_vec())], &[]).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("/Parent 3 0 R"));
        assert!(s.contains("/MediaBox [0 0 10 10]"));
        assert!(s.contains("/Contents [4 0 R 9 0 R]"));
        assert!(!s.contains("/Contents 4 0 R"));
    }

    #[test]
    fn test_subsections() {
        let e = [(3, 0, 10), (4, 0, 20), (9, 0, 30)];
        let g = subsections(&e);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].len(), 2);
    }
}
