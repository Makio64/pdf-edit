# Decision 03 — Pixel-Identical Pages (`Preset::Exact`) and Direct PDF Editing

- **Date**: 2026-10-21
- **Question**: can the converted `.docx` / Pages document look 1:1 like the PDF, and where that stops,
  what is the alternative? Answer: `Preset::Exact` gets within a pixel for every document whose fonts
  the Mac has; for the rest, and for "correct and send back" use, edit the PDF itself.

## 1. How fidelity is measured

`p2d-cli fidelity [--preset exact|fidele|editable] [--min-f1 0.95] <pdf>…` converts each PDF, opens
the DOCX in **Apple Pages**, exports it back to PDF, renders both PDFs at 100 dpi and scores the
**ink F1**. Ink pixels are those darker than mid-grey. A pixel counts as matched when the other page
has ink within 1.5 px (and within 3 px), and 1.000 means every mark is where it was. A different page
count is flagged.

## 2. What Pages does with a DOCX

Measured with calibration documents exported by Pages 15.3:

| Behaviour | Measured |
|---|---|
| `Exact` line spacing `L` | first baseline at **top + 0.8 × L**, then exactly every `L`, for every font tested |
| Text box with zero insets | text starts **0.5 pt** right of the box edge |
| Flow paragraphs | baseline = top margin + 0.8 L; `before` is added exactly; `after`+`before` do **not** add up; `before` is **dropped at the top of a page** |
| Character spacing `w:spacing` | honoured, **added** after every character of the run |
| Width scale `w:w` | **ignored** |
| Pair kerning | **always applied** (from the font's `kern` table), also across runs, before any character spacing |
| Justified line ending with `w:br` | **not** stretched |

## 3. Results (26 synthetic + 8 real documents, 51 pages)

| Preset | Synthetic F1 @1.5 / @3 px | Real-world F1 @1.5 / @3 px | Page count kept |
|---|---|---|---|
| `Editable` (flow, default before) | 0.405 / 0.590 | 0.228 / 0.302 | 6 documents overflow |
| `Fidele` (flow, exact leading) | 0.400 / 0.573 | 0.242 / 0.322 | 7 documents overflow |
| **`Exact` (new)** | **1.000 / 1.000** | **0.978 / 0.996** | **all** |

With the look-alike fonts of §4 (2026-10-21, same set): InDesign invoice 0.905 → **0.963**, Canva
courses 0.968 → 0.970 and 0.977 → 0.980, Nunito invoice 0.978 → 0.963 (its bold headings now get a
face of the right weight, slightly narrower); at 3 px every document is ≥ 0.994. The DOCX of the
synthetic corpus is byte-for-byte unchanged.

The real-world set is private: a 7-page Canva course (×2), a Facturation.pro quote, a wkhtmltopdf invoice
in Nunito, an Excel-like invoice, a Pages-made certificate, a government form with logos and an InDesign
invoice in Perpetua / Trade Gothic. Every real document is ≥ 0.989 at 3 px; the lowest at 1.5 px
(0.905) is the InDesign invoice, whose fonts are not on the Mac.

`p2d-cli qa` now also gates `Exact` on every corpus document: 100 % word recall, 0 glued words, every
block positioned.

## 4. How `Preset::Exact` works

- **Graphics.** A page whose drawings are only axis-aligned rectangles and rules (most letters,
  invoices and forms) gets them back as **native DOCX shapes**: sharp at any zoom and editable. Any other page
  gets one background image: the page rendered without its horizontal text, from a vendored copy of the
  `hayro` renderer with a `skip_text` switch (`vendor/hayro`). It is PNG, or JPEG when that is much smaller.
  Rotated text stays in that image.
- **Text.** Glyphs → lines → column segments → blocks (same size, pitch and alignment edge). Each block is
  one borderless **text box anchored to the page** at `x − 0.5 pt`, `top = first baseline − 0.8 × L`,
  with `Exact` spacing equal to the block's pitch, real paragraphs and a soft break at each source line end.
- **Fonts.** The PDF font's own family when the Mac has it (`TimesNewRomanPSMT` → Times New Roman, base-14
  Helvetica → Helvetica, …), measured from the installed font files (`SystemFontMetrics`, with `kern` pairs).
  Otherwise the installed family that **looks most like it** (`p2d_layout::lookalike`): the glyphs the
  PDF draws (outlines kept by the extraction, whatever the font type) are rasterised next to the same
  characters in ~40 common Mac families, scored on outline agreement within one cell, ink weight and
  advance widths, with a small penalty across the serif / sans divide. Montserrat → Avenir Next, Oswald →
  Avenir Next Condensed / Arial Narrow, Nunito → Avenir, Trade Gothic → PT Sans. The name table
  (`map_pdf_font_to_mac`) is only the last resort (WASM, too few glyphs).
- **Positions inside a line.** Every glyph set in an installed font, the PDF's own family or a look-alike,
  lands on its source x: per-character spacing = source distance − (advance + kerning Pages adds). Words
  whose font cannot be measured are stretched as a whole. Spaces absorb the rest, which also reproduces
  justified text exactly.

## 5. Where 1:1 stops with DOCX / Pages

1. **Fonts the Mac does not have** (Canva's Montserrat / Oswald, Google fonts, commercial faces such as Trade
   Gothic): positions stay exact, glyph shapes differ. Pages ignores fonts embedded in a DOCX. The only fix
   inside Pages is installing the font. Many PDFs embed the font program, but usually as a subset that
   only has the glyphs the document used.
2. **Editing moves text, not graphics**: growing a paragraph does not move the shapes or background
   under it. Boxes keep everything in place, which is right for corrections and filling in, and
   wrong for rewriting a whole page.
3. Complex graphics (curves, gradients, photos) live in the background image, so they cannot be edited.
4. Scanned pages keep their image and get no live text. The native `.pages` writer is still flow-only;
   `.pages` files with the exact layout have to come from Pages itself (open the DOCX, save).

## 6. The alternative: edit the PDF itself

`p2d_pdf::edit` (CLI `p2d-cli edit-lines` / `p2d-cli edit`; app: **« ✏️ Modifier le PDF directement »**).
The person clicks a line on the real page image, retypes it, presses Enter, and saves.
`… (modifié).pdf` is written next to the original, which is never touched.

- **Nothing else changes**: the result is an **incremental update**. The original bytes stay as they are, and
  only the page's new content stream, page dictionary and fonts are appended, with a cross-reference
  table or stream chained via `/Prev`. Untouched pages render byte-for-byte the same (measured F1 1.000,
  SSIM 1.000).
- **Removing the old words without moving any other glyph**: the page content is re-walked with its
  graphics and text state. Text operators are matched to the extracted glyphs: counts follow how the
  renderer emits them, and each operator's first-glyph position is checked. The operators holding the
  edited glyphs are split, and those glyphs are drawn in invisible mode (`3 Tr`). Render mode never
  moves text, so no glyph width is needed. Text inside form XObjects drawn once (nested ones too, e.g.
  Prawn's `Stamp` headers) is rewritten in a copy of each form on the path, and the page points at the
  copies. When none of this can be done safely (an unknown encoding, a form drawn twice, a stream out of
  step with the extraction), the line is covered with the sampled background colour instead.
- **Same typeface, same typography**: see §8.
- Measured on the 8 real documents: every test edit is removed in-stream (the Prawn invoice headers,
  covered before, now too), and the re-extracted text of an edited line is clean.

**Limits / next steps**: the old words remain in the file (invisible), so copy/search still finds them.
Encrypted PDFs are refused. A retyped line does not wrap or push the next lines. Characters missing
from a subset come from the closest installed look-alike, so they show a slightly different design
when the Mac lacks the PDF's font. The extension (WASM) does not offer direct editing yet.

## 7. Recommendation

- **Correct and send back** (forms, letters, amounts, names, dates): **edit the PDF directly**. It is
  identical by construction and needs no Pages round trip.
- **Reuse the content in Pages / Word with the same look**: `Exact` (the « 🎯 Identique au PDF » button,
  now the default).
- **Rewrite at length**: `Editable` flow, accepting that the layout is approximate.

## 8. Editing looks like the PDF

Goal: clicking a line must not change how it looks, and what is shown after Enter must be exactly what
is saved.

**What the real PDFs do** (analysis of the 8 real documents): wkhtmltopdf (Qt) rounds every advance to
a 0.288 pt grid and declares `/W` widths ~0.8 % below the program's `hmtx`; Pages/Quartz justifies
with `Tc` (+2.4 pt per letter) and puts no letter spacing before a space; Canva positions every glyph
with `Td`, adds 0.03 em letter spacing, splits one typeface over several Type 3 subsets drawn through
a flipped `FontMatrix` and text matrix, and leaves most glyphs of its TrueType subsets empty while
their `cmap` still lists them; Google Sheets and Prawn embed subsets without a usable `cmap` (Prawn's
are symbolic, coded only through `ToUnicode`) and Prawn draws headers inside nested forms; InDesign
embeds Type1C (bare CFF) fonts and kerns with `TJ`; iText leaves the 14 standard fonts unembedded
and fakes small caps with two sizes on one line.

**Editable lines** (`p2d_layout::exact::editable_lines`) now carry, measured against the PDF's own
advances: letter spacing (mean of the letter gaps near the usual one, so grid rounding averages out),
word spacing (both gaps around a space, less two letter spacings; from the font's space width where
the PDF draws no space glyphs), horizontal scale, and the edge to keep (left, right, centre) from the
neighbouring lines, numbers defaulting to the right as in invoice columns. Every space the PDF draws is
kept; rows of 3+ spaces and jumps wider than a word space split a line (form labels, table cells). A
line is a list of runs (font, size, colour); one typeface split over several subset fonts stays one run.

**The editor** (`extension/ui/edit.js`) shows the line being retyped in the PDF's own typeface:
- embedded TrueType/OpenType programs become browser fonts (`p2d_pdf::sfnt::web_font`): `cmap`,
  `name`, `OS/2` and `post` are rebuilt, only glyphs that really have a shape are mapped (the rest falls
  back per character), and `hmtx` is replaced by the PDF's own widths;
- other fonts (Type 1, CFF, Type 3) are redrawn from the glyph outlines the extraction keeps
  (`p2d_pdf::sfnt::outline_font`, cubic curves turned into quadratic ones);
- the text gets the run's size and colour, the line's letter and word spacing and stretch, no kerning
  or ligatures (the PDF has none), its baseline measured in the browser, and its anchored edge.
Measured as the width of each retyped line against the PDF line (8 real documents, 446 lines, every
one shown in the PDF's own typeface): mean deviation **0.00–0.04 %** per document (about 0.1 % on
one utility bill since the half-space rule below), against 4 % before (and Avenir Next instead of Nunito,
Helvetica instead of Montserrat).

Spacing is finally calibrated on the line itself: what the averages leave over goes to the word
spacing, so retyping a line's own text gives back its exact width, unless that would take more than
half of each space (heavy pair kerning: an address line of one real bill would keep 0.6 pt spaces);
the words then keep their spaces and the line comes out 2.5 pt longer. Retyped with the same text,
saved and rendered, 60 of 96 sampled left-aligned lines are pixel-identical; the others differ by
less than a point (InDesign pair kerning, Qt's 0.288 pt grid, which even spacing cannot follow).

After Enter, `/api/edit/preview` applies the pending edits in memory and the page image is rendered
again from that PDF: the page shows exactly what `/api/edit/save` will write.

**Saving** (`p2d_pdf::edit`) sets the new text at the PDF's own size, colour, `Tc`, `Tz` and word
spacing (a `TJ` adjustment after each space, since `Tw` ignores two-byte codes), anchored on the
original edge, in this order of fonts per character:
1. the PDF font itself: derived TrueType dictionaries (Unicode, Mac Roman, symbolic or `ToUnicode`
   lookups; the PDF's declared widths kept), or the font reused as is with the codes the PDF already
   uses for that character (Type 3 glyph procedures, Type 1/CFF and CID fonts for characters the
   document draws, standard fonts), set with the size and text direction the content stream used;
   another subset of the same typeface for what the first lacks;
2. the installed family (same family, or closest look-alike), embedded once per save as a subset of
   the glyphs needed (`p2d_pdf::sfnt::subset_truetype`; licences forbidding embedding are respected);
   a piece needing the PDF font's own installed family is set entirely in it;
3. Helvetica / Times / Courier as the last resort.

