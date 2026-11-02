use miniz_oxide::inflate::decompress_to_vec;
use p2d_ir::*;

use crate::zip_pkg::DeterministicZipWriter;

/// Pages' own blank document (`Blank/ISO.template`), which this writer fills in. It is Apple's
/// file, so it is read from the Pages installed on this Mac and never shipped with the program.
/// Only the version whose bytes the offsets below were written for is used (Pages 14.5:
/// 69 368 bytes, CRC-32 `40d1e267`); with any other, there is no native `.pages` writer.
const BLANK_TEMPLATE_PATHS: &[&str] = &[
    "/Applications/Pages.app/Contents/SharedSupport/Templates/Blank/ISO.template",
    "/Applications/Pages Creator Studio.app/Contents/SharedSupport/Templates/Blank/ISO.template",
];
const BLANK_TEMPLATE_LEN: usize = 69_368;
const BLANK_TEMPLATE_CRC32: u32 = 0x40d1_e267;

/// The blank Pages 14.5 document from this Mac's Pages, if it has that version.
pub fn blank_template() -> Option<Vec<u8>> {
    BLANK_TEMPLATE_PATHS.iter().find_map(|path| {
        let bytes = std::fs::read(path).ok()?;
        (bytes.len() == BLANK_TEMPLATE_LEN && crc32fast::hash(&bytes) == BLANK_TEMPLATE_CRC32).then_some(bytes)
    })
}

// Stylesheet object varints inside the blank template (`Index/DocumentStylesheet.iwa`):
// Paragraph styles:
const STYLE_PARA_TITLE: &[u8] = &[0x8a, 0xd7, 0x69]; // 1731466 (Title)
const STYLE_PARA_HEADING1: &[u8] = &[0x95, 0xd7, 0x69]; // 1731477 (Heading)
const STYLE_PARA_HEADING2: &[u8] = &[0x96, 0xd7, 0x69]; // 1731478 (Heading 2)
const STYLE_PARA_BODY: &[u8] = &[0x9e, 0xd7, 0x69]; // 1731486 (Body)

// Character styles:
const STYLE_CHAR_BOLD: &[u8] = &[0xbc, 0xd7, 0x69]; // 1731516 (Emphasis / Bold)
const STYLE_CHAR_ITALIC: &[u8] = &[0xbf, 0xd7, 0x69]; // 1731519 (Italic)

// List styles:
const STYLE_LIST_NONE: &[u8] = &[0x80, 0xd7, 0x69]; // 1731456 (None)
const STYLE_LIST_BULLET: &[u8] = &[0x84, 0xd7, 0x69]; // 1731460 (Bullet)

/// Serialize a `FlowDoc` into a native Apple Pages `.pages` archive (iWork '14+ IWA format), or
/// `None` when this Mac has no Pages 14.5 blank template to fill in (see [`blank_template`]).
pub fn write_pages(doc: &FlowDoc) -> Option<Vec<u8>> {
    let template = blank_template()?;
    let entries = read_zip_entries(&template);
    let mut zip = DeterministicZipWriter::new();

    for (name, raw_data) in entries {
        if name == "Index/Document.iwa" {
            let patched = patch_document_iwa(&raw_data, doc).unwrap_or(raw_data);
            zip.add_file(&name, &patched, false);
        } else {
            let deflate = name.ends_with(".plist") || name.ends_with(".xml");
            zip.add_file(&name, &raw_data, deflate);
        }
    }

    Some(zip.finish())
}

#[derive(Clone)]
struct PagesPara {
    runs: Vec<(String, Option<&'static [u8]>)>,
    para_style: &'static [u8],
    list_style: &'static [u8],
}

fn collect_pages_paragraphs(doc: &FlowDoc) -> Vec<PagesPara> {
    let mut out = Vec::new();

    for (sec_idx, sec) in doc.sections.iter().enumerate() {
        if sec_idx > 0 && !out.is_empty() {
            out.push(PagesPara {
                runs: vec![("\u{000C}".to_string(), None)],
                para_style: STYLE_PARA_BODY,
                list_style: STYLE_LIST_NONE,
            });
        }

        for hp in &sec.header_paragraphs {
            push_ir_paragraph(&mut out, hp, false);
        }

        for block in &sec.blocks {
            match block {
                FlowBlock::Paragraph(p) => {
                    if p.page_break_before && !out.is_empty() {
                        out.push(PagesPara {
                            runs: vec![("\u{000C}".to_string(), None)],
                            para_style: STYLE_PARA_BODY,
                            list_style: STYLE_LIST_NONE,
                        });
                    }
                    push_ir_paragraph(&mut out, p, false);
                }
                FlowBlock::Table(t) => {
                    if t.rows.len() == 1 && t.rows[0].cells.len() == 1 {
                        // Single-cell banner or callout block
                        let cell = &t.rows[0].cells[0];
                        let is_dark_banner = cell
                            .shading_color
                            .map(|c| {
                                (0.299 * c.r as f32 + 0.587 * c.g as f32 + 0.114 * c.b as f32)
                                    < 140.0
                            })
                            .unwrap_or(false);
                        for p in &cell.paragraphs {
                            push_ir_paragraph(&mut out, p, is_dark_banner);
                        }
                    } else if t.is_borderless_layout && t.rows.len() == 1 && t.rows[0].cells.len() == 2 {
                        // 2-column header block (Sender left / Recipient right)
                        for (c_idx, cell) in t.rows[0].cells.iter().enumerate() {
                            if c_idx > 0 && !cell.paragraphs.is_empty() {
                                out.push(PagesPara {
                                    runs: Vec::new(),
                                    para_style: STYLE_PARA_BODY,
                                    list_style: STYLE_LIST_NONE,
                                });
                            }
                            for p in &cell.paragraphs {
                                let indent_prefix = if c_idx == 1 {
                                    "\t\t\t\t"
                                } else {
                                    ""
                                };
                                let mut runs = Vec::new();
                                if !indent_prefix.is_empty() {
                                    runs.push((indent_prefix.to_string(), None));
                                }
                                for r in &p.runs {
                                    let cstyle = if r.style.bold {
                                        Some(STYLE_CHAR_BOLD)
                                    } else if r.style.italic {
                                        Some(STYLE_CHAR_ITALIC)
                                    } else {
                                        None
                                    };
                                    runs.push((r.text.replace('\u{202F}', "\u{00A0}"), cstyle));
                                }
                                out.push(PagesPara {
                                    runs,
                                    para_style: STYLE_PARA_BODY,
                                    list_style: STYLE_LIST_NONE,
                                });
                            }
                        }
                    } else {
                        // Structured multi-column data table: render rows with tab stops
                        for (r_idx, row) in t.rows.iter().enumerate() {
                            let is_hdr = row.is_header_row || r_idx == 0;
                            let mut runs: Vec<(String, Option<&'static [u8]>)> = Vec::new();
                            for (c_idx, cell) in row.cells.iter().enumerate() {
                                if c_idx > 0 {
                                    runs.push((" \t ".to_string(), None));
                                }
                                let cell_text: String = cell
                                    .paragraphs
                                    .iter()
                                    .map(|p| {
                                        p.runs
                                            .iter()
                                            .map(|r| r.text.replace('\u{202F}', "\u{00A0}"))
                                            .collect::<String>()
                                    })
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                let cstyle = if is_hdr
                                    || cell
                                        .paragraphs
                                        .iter()
                                        .flat_map(|p| &p.runs)
                                        .any(|r| r.style.bold)
                                {
                                    Some(STYLE_CHAR_BOLD)
                                } else {
                                    None
                                };
                                runs.push((cell_text, cstyle));
                            }
                            out.push(PagesPara {
                                runs,
                                para_style: if is_hdr {
                                    STYLE_PARA_HEADING2
                                } else {
                                    STYLE_PARA_BODY
                                },
                                list_style: STYLE_LIST_NONE,
                            });
                        }
                    }
                }
                FlowBlock::TextBox(tb) => {
                    for p in &tb.paragraphs {
                        push_ir_paragraph(&mut out, p, false);
                    }
                }
                FlowBlock::Image(_) | FlowBlock::Shape(_) => {}
                FlowBlock::PageBreak => {
                    if !out.is_empty() {
                        out.push(PagesPara {
                            runs: vec![("\u{000C}".to_string(), None)],
                            para_style: STYLE_PARA_BODY,
                            list_style: STYLE_LIST_NONE,
                        });
                    }
                }
            }
        }

        for fp in &sec.footer_paragraphs {
            push_ir_paragraph(&mut out, fp, false);
        }
    }

    if out.is_empty() {
        out.push(PagesPara {
            runs: vec![(String::new(), None)],
            para_style: STYLE_PARA_BODY,
            list_style: STYLE_LIST_NONE,
        });
    }

    out
}

fn push_ir_paragraph(out: &mut Vec<PagesPara>, p: &Paragraph, force_title: bool) {
    let para_style = if force_title {
        STYLE_PARA_TITLE
    } else {
        match p.heading_level {
            Some(1) => STYLE_PARA_HEADING1,
            Some(2) | Some(3) => STYLE_PARA_HEADING2,
            _ => STYLE_PARA_BODY,
        }
    };

    let list_style = match p.list_kind {
        Some(ListKind::Bullet) => STYLE_LIST_BULLET,
        _ => STYLE_LIST_NONE,
    };

    let mut runs = Vec::new();
    if p.left_indent_pt >= 180.0 && p.list_kind.is_none() {
        runs.push(("\t\t\t\t".to_string(), None));
    }

    for r in &p.runs {
        if r.text.is_empty() {
            continue;
        }
        let cstyle = if r.style.bold && p.heading_level.is_none() && !force_title {
            Some(STYLE_CHAR_BOLD)
        } else if r.style.italic {
            Some(STYLE_CHAR_ITALIC)
        } else {
            None
        };
        let clean = r
            .text
            .replace('\u{202F}', "\u{00A0}")
            .replace('\n', " ");
        runs.push((clean, cstyle));
    }

    out.push(PagesPara {
        runs,
        para_style,
        list_style,
    });
}

fn patch_document_iwa(raw_iwa: &[u8], doc: &FlowDoc) -> Option<Vec<u8>> {
    let decompressed = snappy_decompress_iwa(raw_iwa)?;
    // Verify expected layout of `Obj 1732514` (`TSWP.StorageArchive`) at offset 11698..11826
    if decompressed.len() < 11826
        || &decompressed[11698..11703] != &[0x1e, 0x08, 0xa2, 0xdf, 0x69]
    {
        return None;
    }

    let orig_payload = &decompressed[11729..11826];
    let paras = collect_pages_paragraphs(doc);

    let mut full_text = String::new();
    let mut utf16_offset: usize = 0;

    let mut para_entries = Vec::new();
    let mut list_entries = Vec::new();
    let mut char_entries = Vec::new();

    let mut last_list_style: Option<&'static [u8]> = None;
    let mut last_char_style: Option<Option<&'static [u8]>> = None;

    for (idx, p) in paras.iter().enumerate() {
        let is_last = idx + 1 == paras.len();

        // Paragraph style entry (every paragraph start is recorded in `table_para_style`)
        para_entries.extend_from_slice(&encode_obj_entry(utf16_offset, Some(p.para_style)));

        // List style transition entry
        if last_list_style != Some(p.list_style) {
            list_entries.extend_from_slice(&encode_obj_entry(utf16_offset, Some(p.list_style)));
            last_list_style = Some(p.list_style);
        }

        if p.runs.is_empty() {
            if last_char_style != Some(None) {
                char_entries.extend_from_slice(&encode_obj_entry(utf16_offset, None));
                last_char_style = Some(None);
            }
        } else {
            for (text, cstyle) in &p.runs {
                if text.is_empty() {
                    continue;
                }
                if last_char_style != Some(*cstyle) {
                    char_entries.extend_from_slice(&encode_obj_entry(utf16_offset, *cstyle));
                    last_char_style = Some(*cstyle);
                }
                full_text.push_str(text);
                utf16_offset += text.encode_utf16().count();
            }
        }

        if !is_last {
            if last_char_style != Some(None) {
                char_entries.extend_from_slice(&encode_obj_entry(utf16_offset, None));
                last_char_style = Some(None);
            }
            full_text.push('\n');
            utf16_offset += 1;
        }
    }

    if list_entries.is_empty() {
        list_entries.extend_from_slice(&encode_obj_entry(0, Some(STYLE_LIST_NONE)));
    }
    if char_entries.is_empty() {
        char_entries.extend_from_slice(&encode_obj_entry(0, None));
    }

    // Build `TSWP.StorageArchive` payload
    let mut new_payload = Vec::new();
    // Field 1 (`kind = 0`) + Field 2 (`style_sheet = 1732588`)
    new_payload.extend_from_slice(&orig_payload[0..8]);

    // Field 3 (`text`, tag `0x1a`)
    if !full_text.is_empty() {
        new_payload.push(0x1a);
        encode_varint(full_text.len(), &mut new_payload);
        new_payload.extend_from_slice(full_text.as_bytes());
    }

    // Field 5 (`table_para_style`, tag `0x2a`)
    new_payload.push(0x2a);
    encode_varint(para_entries.len(), &mut new_payload);
    new_payload.extend_from_slice(&para_entries);

    // Field 6 (`table_para_starts`, tag `0x32` - 10 bytes from template)
    new_payload.extend_from_slice(&orig_payload[20..30]);

    // Field 7 (`table_list_style`, tag `0x3a`)
    new_payload.push(0x3a);
    encode_varint(list_entries.len(), &mut new_payload);
    new_payload.extend_from_slice(&list_entries);

    // Field 8 (`table_char_style`, tag `0x42`)
    new_payload.push(0x42);
    encode_varint(char_entries.len(), &mut new_payload);
    new_payload.extend_from_slice(&char_entries);

    // Fields 10..28 (`in_document`, `table_column_style`, `table_section`, `table_drop_cap`, `table_bidi`, etc.)
    new_payload.extend_from_slice(&orig_payload[42..97]);

    // Build updated `ArchiveInfo` for `Obj 1732514`
    let refs: &[u8] = &[
        0x9e, 0xd7, 0x69, // Body
        0x95, 0xd7, 0x69, // Heading
        0x96, 0xd7, 0x69, // Heading 2
        0x8a, 0xd7, 0x69, // Title
        0xbc, 0xd7, 0x69, // Emphasis (Bold)
        0xbf, 0xd7, 0x69, // Italic
        0x80, 0xd7, 0x69, // List None
        0x84, 0xd7, 0x69, // List Bullet
        0xe6, 0xdf, 0x69, // Column style
        0xaf, 0xdd, 0x69, // Section
    ];
    let mut msg_info = Vec::new();
    msg_info.extend_from_slice(&[0x08, 0xd1, 0x0f, 0x12, 0x03, 0x01, 0x00, 0x05, 0x18]);
    encode_varint(new_payload.len(), &mut msg_info);
    msg_info.push(0x2a);
    encode_varint(refs.len(), &mut msg_info);
    msg_info.extend_from_slice(refs);

    let mut arch_info = Vec::new();
    arch_info.extend_from_slice(&[0x08, 0xa2, 0xdf, 0x69, 0x12]);
    encode_varint(msg_info.len(), &mut arch_info);
    arch_info.extend_from_slice(&msg_info);

    let mut new_uncompressed = Vec::with_capacity(decompressed.len() + new_payload.len());
    new_uncompressed.extend_from_slice(&decompressed[..11698]);
    encode_varint(arch_info.len(), &mut new_uncompressed);
    new_uncompressed.extend_from_slice(&arch_info);
    new_uncompressed.extend_from_slice(&new_payload);
    new_uncompressed.extend_from_slice(&decompressed[11826..]);

    Some(snappy_compress_iwa(&new_uncompressed))
}

fn encode_obj_entry(utf16_offset: usize, style_varint: Option<&[u8]>) -> Vec<u8> {
    let mut inner = Vec::with_capacity(12);
    inner.push(0x08);
    encode_varint(utf16_offset, &mut inner);
    if let Some(sv) = style_varint {
        inner.extend_from_slice(&[0x12, 0x04, 0x08]);
        inner.extend_from_slice(sv);
    }
    let mut entry = Vec::with_capacity(inner.len() + 2);
    entry.push(0x0a);
    encode_varint(inner.len(), &mut entry);
    entry.extend_from_slice(&inner);
    entry
}

fn encode_varint(mut val: usize, out: &mut Vec<u8>) {
    while val >= 0x80 {
        out.push(((val & 0x7f) as u8) | 0x80);
        val >>= 7;
    }
    out.push(val as u8);
}

fn decode_varint(data: &[u8], mut pos: usize) -> Option<(usize, usize)> {
    let mut val = 0usize;
    let mut shift = 0u32;
    loop {
        let b = *data.get(pos)?;
        pos += 1;
        val |= ((b & 0x7f) as usize) << shift;
        if (b & 0x80) == 0 {
            return Some((val, pos));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

fn snappy_decompress_iwa(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 4 <= data.len() {
        let _chunk_type = data[pos];
        let clen = (data[pos + 1] as usize)
            | ((data[pos + 2] as usize) << 8)
            | ((data[pos + 3] as usize) << 16);
        pos += 4;
        let chunk = data.get(pos..pos + clen)?;
        pos += clen;

        let (uncompressed_len, mut cpos) = decode_varint(chunk, 0)?;
        let mut block = Vec::with_capacity(uncompressed_len);
        while cpos < chunk.len() {
            let tag = chunk[cpos];
            cpos += 1;
            match tag & 0x03 {
                0 => {
                    let mut len = ((tag >> 2) as usize) + 1;
                    if len > 60 {
                        let extra_bytes = len - 60;
                        let mut extra = 0usize;
                        for b_idx in 0..extra_bytes {
                            extra |= (*chunk.get(cpos + b_idx)? as usize) << (8 * b_idx);
                        }
                        cpos += extra_bytes;
                        len = extra + 1;
                    }
                    block.extend_from_slice(chunk.get(cpos..cpos + len)?);
                    cpos += len;
                }
                1 => {
                    let len = (((tag >> 2) & 0x07) as usize) + 4;
                    let offset = (((tag & 0xe0) as usize) << 3) | (*chunk.get(cpos)? as usize);
                    cpos += 1;
                    if offset == 0 || offset > block.len() {
                        return None;
                    }
                    for _ in 0..len {
                        let b = block[block.len() - offset];
                        block.push(b);
                    }
                }
                2 => {
                    let len = ((tag >> 2) as usize) + 1;
                    let lo = *chunk.get(cpos)? as usize;
                    let hi = *chunk.get(cpos + 1)? as usize;
                    cpos += 2;
                    let offset = lo | (hi << 8);
                    if offset == 0 || offset > block.len() {
                        return None;
                    }
                    for _ in 0..len {
                        let b = block[block.len() - offset];
                        block.push(b);
                    }
                }
                _ => return None,
            }
        }
        out.extend_from_slice(&block);
    }
    Some(out)
}

fn snappy_compress_iwa(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + 128);
    let mut pos = 0usize;
    while pos < raw.len() {
        let block_end = (pos + 65536).min(raw.len());
        let block = &raw[pos..block_end];
        pos = block_end;

        let mut chunk = Vec::with_capacity(block.len() + (block.len() / 60) + 16);
        encode_varint(block.len(), &mut chunk);

        let mut bpos = 0usize;
        while bpos < block.len() {
            let sub_end = (bpos + 60).min(block.len());
            let sub = &block[bpos..sub_end];
            bpos = sub_end;
            chunk.push((((sub.len() - 1) as u8) << 2) | 0x00);
            chunk.extend_from_slice(sub);
        }

        let clen = chunk.len();
        out.push(0x00);
        out.push((clen & 0xff) as u8);
        out.push(((clen >> 8) & 0xff) as u8);
        out.push(((clen >> 16) & 0xff) as u8);
        out.extend_from_slice(&chunk);
    }
    out
}

fn read_zip_entries(zip_bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut entries = Vec::new();
    // Locate End of Central Directory (0x06054b50) from the end of the buffer
    let mut eocd_pos = None;
    if zip_bytes.len() >= 22 {
        for i in (0..=zip_bytes.len() - 22).rev() {
            if &zip_bytes[i..i + 4] == &[0x50, 0x4b, 0x05, 0x06] {
                eocd_pos = Some(i);
                break;
            }
        }
    }
    let Some(eocd) = eocd_pos else {
        return entries;
    };

    let cd_count = u16::from_le_bytes([zip_bytes[eocd + 10], zip_bytes[eocd + 11]]) as usize;
    let cd_offset = u32::from_le_bytes([
        zip_bytes[eocd + 16],
        zip_bytes[eocd + 17],
        zip_bytes[eocd + 18],
        zip_bytes[eocd + 19],
    ]) as usize;

    let mut pos = cd_offset;
    for _ in 0..cd_count {
        if pos + 46 > zip_bytes.len() || &zip_bytes[pos..pos + 4] != &[0x50, 0x4b, 0x01, 0x02] {
            break;
        }
        let method = u16::from_le_bytes([zip_bytes[pos + 10], zip_bytes[pos + 11]]);
        let comp_size = u32::from_le_bytes([
            zip_bytes[pos + 20],
            zip_bytes[pos + 21],
            zip_bytes[pos + 22],
            zip_bytes[pos + 23],
        ]) as usize;
        let name_len = u16::from_le_bytes([zip_bytes[pos + 28], zip_bytes[pos + 29]]) as usize;
        let extra_len = u16::from_le_bytes([zip_bytes[pos + 30], zip_bytes[pos + 31]]) as usize;
        let comment_len = u16::from_le_bytes([zip_bytes[pos + 32], zip_bytes[pos + 33]]) as usize;
        let local_offset = u32::from_le_bytes([
            zip_bytes[pos + 42],
            zip_bytes[pos + 43],
            zip_bytes[pos + 44],
            zip_bytes[pos + 45],
        ]) as usize;

        let name = String::from_utf8_lossy(&zip_bytes[pos + 46..pos + 46 + name_len]).to_string();
        pos += 46 + name_len + extra_len + comment_len;

        if local_offset + 30 <= zip_bytes.len()
            && &zip_bytes[local_offset..local_offset + 4] == &[0x50, 0x4b, 0x03, 0x04]
        {
            let l_name_len =
                u16::from_le_bytes([zip_bytes[local_offset + 26], zip_bytes[local_offset + 27]])
                    as usize;
            let l_extra_len =
                u16::from_le_bytes([zip_bytes[local_offset + 28], zip_bytes[local_offset + 29]])
                    as usize;
            let data_start = local_offset + 30 + l_name_len + l_extra_len;
            if let Some(comp_slice) = zip_bytes.get(data_start..data_start + comp_size) {
                let raw = if method == 8 {
                    decompress_to_vec(comp_slice).unwrap_or_default()
                } else {
                    comp_slice.to_vec()
                };
                entries.push((name, raw));
            }
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_pages_bundle_generation() {
        let mut sec = Section::default();
        sec.blocks.push(FlowBlock::Paragraph(Paragraph {
            runs: vec![TextRun {
                text: "DIRECTION GÉNÉRALE DES FINANCES PUBLIQUES".to_string(),
                style: RunStyle {
                    bold: true,
                    font_size_pt: 15.0,
                    ..Default::default()
                },
            }],
            heading_level: Some(1),
            ..Default::default()
        }));
        sec.blocks.push(FlowBlock::Paragraph(Paragraph {
            runs: vec![
                TextRun {
                    text: "Madame, Monsieur, votre dégrèvement de ".to_string(),
                    style: RunStyle::default(),
                },
                TextRun {
                    text: "642,00 €".to_string(),
                    style: RunStyle {
                        bold: true,
                        ..Default::default()
                    },
                },
                TextRun {
                    text: " a été validé.".to_string(),
                    style: RunStyle::default(),
                },
            ],
            ..Default::default()
        }));

        let doc = FlowDoc {
            sections: vec![sec],
            ..Default::default()
        };

        let Some(bytes) = write_pages(&doc) else {
            eprintln!("skipped: this Mac has no Pages 14.5 blank template");
            return;
        };
        assert!(bytes.starts_with(b"PK\x03\x04"));
        let entries = read_zip_entries(&bytes);
        let doc_iwa = entries
            .iter()
            .find(|(name, _)| name == "Index/Document.iwa")
            .expect("Must contain Index/Document.iwa");
        let decompressed =
            snappy_decompress_iwa(&doc_iwa.1).expect("Must decompress patched Document.iwa");
        let text_lossy = String::from_utf8_lossy(&decompressed);
        assert!(text_lossy.contains("DIRECTION GÉNÉRALE DES FINANCES PUBLIQUES"));
        assert!(text_lossy.contains("642,00"));
    }
}
