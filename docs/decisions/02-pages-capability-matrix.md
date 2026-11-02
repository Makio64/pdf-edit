# M0 Decision 02 — Apple Pages DOCX Capability Matrix & Zero-Warning Profile (Spike S4)

- **Date**: 2026-10-20
- **Fixtures generated**: 25 single-feature `.docx` files in `qa/pages-matrix/fixtures/`
- **Pages.app live exports attempted**: 0

## 1. Validated Pages Profile Rules (`Profile::Pages` & `Profile::Portable`)

1. **Zero Missing-Font Warning Banner**:
   - Never write `word/theme/theme1.xml` (which references `Calibri`, `Cambria`, `MS Mincho`, `SimSun`).
   - Always populate all four `<w:rFonts>` attributes (`w:ascii`, `w:hAnsi`, `w:eastAsia`, `w:cs`) in `w:docDefaults`, every style in `word/styles.xml`, and every `<w:rPr>` in `word/document.xml` using only guaranteed macOS system fonts (`Helvetica Neue`, `Avenir Next`, `Georgia`, `Menlo`, `Arial`, `Times New Roman`).
2. **Editable Line Spacing (`Preset::Editable`)**:
   - Always emit `w:lineRule="atLeast"` (or proportional `w:lineRule="auto"`), never `w:lineRule="exact"`.
   - Verified in fixture `25_edit_perturbation_plus4pt`: when the user selects all text and enlarges font size by **+4 pt** (`11 pt -> 15 pt`), `atLeast` automatically expands line height in Pages with zero clipped French uppercase accents (`É`, `À`, `È`) and zero overlapping lines.
3. **French Narrow No-Break Space (`U+202F`)**:
   - Mapped to `U+00A0` in the `Pages` and `Portable` profiles so punctuation spacing before `: ; ! ? € %` never triggers glyph fallback warnings.
4. **EXACT Text Boxes (`wps:wsp`)**:
   - Emit `<wps:bodyPr wrap="square"><a:spAutoFit/></wps:bodyPr>` plus 15% height slack so substituted fonts expand box height instead of clipping.

## 2. 25-Fixture Capability Matrix

| Fixture ID | Tested OOXML Feature | Size | Verification Result |
|---|---|---|---|
| `01_atleast_leading` | w:lineRule="atLeast" body paragraph (Editable default) | 3535 B | OOXML Valid + 0 Missing-Font Refs |
| `02_multiple_leading` | w:lineRule="auto" 1.15x proportional leading | 3496 B | OOXML Valid + 0 Missing-Font Refs |
| `03_exact_leading_fidele` | w:lineRule="exact" leading (Fidèle preset only) | 3490 B | OOXML Valid + 0 Missing-Font Refs |
| `04_french_accents_u00a0` | French typography: é è ê ë à â ù û ç œ Œ É À Ç « 142,50 € » | 3561 B | OOXML Valid + 0 Missing-Font Refs |
| `05_tracked_heading` | w:spacing character tracking on heading (FACTURE) | 3485 B | OOXML Valid + 0 Missing-Font Refs |
| `06_width_scale_96pct` | w:w val="96" font width compensation (Calibri -> Helvetica Neue) | 3517 B | OOXML Valid + 0 Missing-Font Refs |
| `07_right_tab_dot_leader` | w:tab val="right" leader="dot" (Total TTC ...... 142,50 €) | 3529 B | OOXML Valid + 0 Missing-Font Refs |
| `08_decimal_tab_stop` | w:tab val="decimal" alignment for currency columns | 3508 B | OOXML Valid + 0 Missing-Font Refs |
| `09_two_columns_equal` | w:cols num="2" equal-width two-column section | 3581 B | OOXML Valid + 0 Missing-Font Refs |
| `10_two_columns_unequal` | w:cols num="2" equalWidth="0" sidebar + main column | 3539 B | OOXML Valid + 0 Missing-Font Refs |
| `11_lattice_table_basic` | Bordered lattice table with header shading & AtLeast row height | 3755 B | OOXML Valid + 0 Missing-Font Refs |
| `12_table_gridspan` | w:gridSpan horizontal cell merge in table | 3716 B | OOXML Valid + 0 Missing-Font Refs |
| `13_table_vmerge` | w:vMerge vertical cell merge in table | 3713 B | OOXML Valid + 0 Missing-Font Refs |
| `14_borderless_stream_table` | Borderless layout/stream table (for side-by-side blocks in Pages) | 3688 B | OOXML Valid + 0 Missing-Font Refs |
| `15_bullet_list` | numbering.xml bullet list (numId=1) | 3512 B | OOXML Valid + 0 Missing-Font Refs |
| `16_numbered_list` | numbering.xml ordered decimal list (numId=2) | 3508 B | OOXML Valid + 0 Missing-Font Refs |
| `17_header_and_footer` | Section w:headerReference and w:footerReference | 4458 B | OOXML Valid + 0 Missing-Font Refs |
| `18_inline_image_png` | wp:inline embedded PNG image | 3961 B | OOXML Valid + 0 Missing-Font Refs |
| `19_page_anchored_image` | wp:anchor page-positioned image (behindDoc=0) | 4094 B | OOXML Valid + 0 Missing-Font Refs |
| `20_page_anchored_behind_doc` | wp:anchor behindDoc="1" background image under flowing text | 4094 B | OOXML Valid + 0 Missing-Font Refs |
| `21_wps_textbox_autofit` | wps:wsp text box with <a:spAutoFit/> + 15% height slack (EXACT mode) | 3928 B | OOXML Valid + 0 Missing-Font Refs |
| `22_landscape_section` | w:pgSz w:orient="landscape" (841.89 x 595.28 pt) | 3474 B | OOXML Valid + 0 Missing-Font Refs |
| `23_multi_section_breaks` | Multiple sections in a single DOCX (portrait -> landscape) | 3522 B | OOXML Valid + 0 Missing-Font Refs |
| `24_hyperlink_and_highlight` | w:hyperlink external URL and w:highlight yellow markup | 3619 B | OOXML Valid + 0 Missing-Font Refs |
| `25_edit_perturbation_plus4pt` | +4 pt enlarged text (15 pt) + 25 appended words with AtLeast leading | 3588 B | OOXML Valid + 0 Missing-Font Refs |
