# PDF Edit — Architecture & Plan (v1, 2026-10-20)

> Original plan (October 2026), kept for its reasoning. What exists today is described in the [README](../README.md); §7 lists what shipped. Folders such as `models/` and `companion/` were never created.

Free, fully local PDF → editable document converter designed for older, non-technical users, including on Intel Macs. One Rust core, two targets:

- **`wasm32-unknown-unknown`** inside a Chrome MV3 extension (works 100% standalone: Gmail attachment & preview buttons + drag-and-drop page).
- **`x86_64-apple-darwin` & `aarch64-apple-darwin`** for an optional native Mac companion (Apple Vision OCR, Dock/Finder entry points, `.pages` output via Pages automation, and 1-click "Attach modified document back to Gmail").

No document byte ever leaves the Mac. The raw research and design-critique notes behind these choices are kept outside the public repository. Everything marked *estimate* is unmeasured until the M0 spikes run.

---

## 1. Key Technical Decisions (and Why)

| Area | Decision | Rejected / Fallback |
|---|---|---|
| **PDF Engine** | **M0 Head-to-Head Bake-Off**: **`hayro`** (pure Rust, MIT/Apache, ~3.8 MB wasm, decrypts RC4/AES, repairs xref, renders via `vello_cpu`, **plus a custom `skrifa` embedded-font `cmap`/`post` inverter** for PDFs missing `/ToUnicode`) vs **PDFium** (Chrome's engine, BSD; `@embedpdf/pdfium` 4.65 MB wasm writing flat binary buffers directly into `p2d-wasm` linear memory; native via `pdfium-render`). Pick primary by measured CER / open-rate / ms-per-page. Loser stays as fallback. | MuPDF (AGPL-3.0 would infect the whole project) = Plan C only. `pdf_oxide` (frame-per-span output, 17.6 MB wasm). `pdf.js` (no per-glyph positions). |
| **Layout Engine** | Rust port of **pdf2docx** (MIT since 0.5.12, 2026-03-09 — port only from MIT-era sources) + **exactdoc** (Apache-2.0) fixes + PdfPig layout analyzers (Apache-2.0), Docling reading order (MIT), pdfplumber/Camelot tables (MIT). | LibreOffice-style frame-per-line output is **never** produced (looks right on open, impossible to edit, worst in Apple Pages). |
| **OCR (Extension)** | **PP-OCRv6** (Apache-2.0) det 1.78 MB + rec pruned to Latin (all French accents), run by **rten** (pure Rust, WASM SIMD128) inside the extension worker. Benchmark `tiny` vs `small` (and `int8`) recognizer tiers on real scans in M0/M4. Models loaded per OCR worker (capped by `navigator.deviceMemory`). | Tesseract (fails on phone photos), `ocrs` (no French accents), `onnxruntime-web` (+14–28 MB; only if WebGPU ≥3× on an average user's GPU). |
| **OCR (Companion)** | **Apple Vision** `VNRecognizeTextRequest` (accurate, `fr-FR` + `en-US`) via `objc2-vision`. | ONNX Runtime native (no `x86_64` macOS builds after 1.23.2). |
| **Layout ML** | None by default (digital PDFs have exact geometry). Optional small model for scans only if it beats rule-based layout. | DocLayout-YOLO (AGPL lineage), LayoutLMv3 (non-commercial). |
| **Writers** | **Own** raw-OOXML DOCX writer (full control: anchors, autofit text boxes, unequal columns, unique `docPr` IDs, zero-warning Pages profile), own ODT / RTF / HTML / MD / TXT writers, `rust_xlsxwriter` (feature `wasm`) for XLSX, `pdf-writer` + `hayro-write` for searchable PDF. One `FlowDoc` model → all serializers. | `docx-rs` (panics on text boxes, `docPr id=1` everywhere, wraps negative offsets). |
| **`.pages` & Return Trip** | Companion drives `/Applications/Pages.app` via AppleScript (using pre-created file sandbox extensions; verified on this dev Mac in M0). Also adds **"Joindre mon document modifié"** to export the active Pages/Word doc back to PDF and attach it in Gmail in 1 click. Without companion: `.docx` auto-opens in Pages (configured as default handler for `.docx` at setup). | `iwork-rs` direct writer = experimental flag only. |
| **Concurrency** | **Single worker first.** Pool of independent single-threaded wasm instances only if the Intel benchmark shows a need (OCR mainly). Stable Rust, `+simd128`, no `SharedArrayBuffer`/COOP/COEP required. | Nightly wasm threads + `rayon` in WASM. |
| **Companion** | Pure-Rust `objc2` `.app` bundle that also serves as the Chrome Native Messaging host. Self-signed code-signing cert (so macOS TCC grants survive rebuilds), installed to `~/Applications`. | Tauri/Electron, Homebrew cask, $99/yr Developer ID. |
| **Pixel-identical output & direct edit** | **`Preset::Exact`** (« Identique au PDF », default in the app): page graphics as vector shapes or a text-less background render, every text block in a page-anchored text box placed with Pages' measured baseline model, same font family when installed, per-glyph character spacing. **Direct PDF editing** (`p2d_pdf::edit`): retyped lines are rewritten inside the PDF as an incremental update. Measurements and limits in [`decisions/03`](decisions/03-pixel-fidelity-and-direct-edit.md). | Flow presets stay for heavy rewriting; frame-per-*line* output is still never produced (boxes are per block). |
| **License** | Project: **MIT OR Apache-2.0**. `cargo-deny` allowlist: MIT, Apache-2.0, BSD-2/3, Zlib, ISC, Unicode-3.0, CC0, OFL-1.1, IJG (`jpeg-encoder`, with credit in `THIRD_PARTY_NOTICES`). Deny AGPL/GPL/NC. | — |

---

## 2. Repository Layout (Consolidated 8-Crate Workspace)

> **Improvement over v0**: Consolidated from 19 micro-crates down to 8 cohesive crates to avoid cross-crate trait boilerplate, orphan-rule friction, and slow rebuilds while keeping a strict sans-IO core and feature-gated binary size.

```
Cargo.toml                 workspace, exact pins
rust-toolchain.toml        stable; targets wasm32-unknown-unknown, x86_64-apple-darwin, aarch64-apple-darwin
deny.toml                  cargo-deny license allowlist (credits are in THIRD_PARTY_NOTICES)
crates/
  p2d-ir/                  Input IR (glyphs, paths, images, links, MCID, Annots) + FlowDoc output model + Params + ConvertReport
  p2d-pdf/                 PdfBackend trait, hayro adapter (+ skrifa cmap/post fallback), PDFium native & zero-copy WASM binary dump decoder, AcroForm/Annots/StructTree walkers
  p2d-layout/              Geometry (Rect/Affine, rstar R-tree, XY-cut), normalisation, words/lines, fonts & wrap prediction, tables, columns/reading order, paragraphs/lists
  p2d-ocr/                 Image preprocessing (¼-scale illumination flattening, deskew, quad warp), PP-OCRv6 pre/post-processing, rten inference
  p2d-writers/             Deterministic zip/XML writer + feature-gated serializers: docx (default), odt, rtf, web (html/md/txt), xlsx, spdf (searchable PDF)
  p2d-engine/              Sans-IO resumable job state machine (Step::{NeedPassword, NeedFontMetrics, NeedOcr, Progress, PageDone, Done}), case routing, never-fail ladder
  p2d-wasm/                wasm-bindgen API + linear-memory arena for zero-copy PDF/page-dump transfer
  p2d-cli/                 CLI + QA harness: convert, dump-ir, oracle-inject (PyMuPDF rawdict), render-pages (osascript Pages.app), diff, bench
qa/                        Corpus manifest, Pages 25-fixture capability matrix, PyMuPDF dev-only oracle scripts, HTML side-by-side report viewer
models/                    PP-OCRv6 ONNX weights + SHA-256 provenance
extension/                 MV3: sw.ts, offscreen.ts, engine.worker.ts, gmail/, ui/, _locales/fr|en
companion/                 objc2 Mac app + native-messaging host + Pages export/return-trip AppleScripts
docs/
  ARCHITECTURE.md
  decisions/               M0 spike results and go/no-go records
```

---

## 3. Conversion Pipeline

1. **Open / Decrypt / Repair**: Empty user password tried automatically; prompt only if needed.
2. **Extract**: Glyphs (Unicode via `ToUnicode` → `skrifa` embedded font `cmap`/`post` inversion → AGL → `ActualText`, effective size, matrix, render mode, clip visibility, MCID), paths, images, links, annotations, form fields.
3. **Normalise (by evidence, not `/Producer`)**:
   - Chrome/Skia `U+200B` spaces (fixes `pdf2docx` bug #379), `0.75` text matrix, ligatures, invisible/clipped/covered text, fake bold.
   - **`Tc`/`Tw`-aware letter-spacing**: subtract `Tc` before gap thresholding and run 2-means intra-word vs inter-word gap clustering per line (prevents `"F A C T U R E"` on tracked headings; carries tracking into `w:spacing`).
4. **Words → Lines**: Per-line gap clustering; Unicode-aware joins (fixes `pdf2docx` gluing French words ending in `é` across lines).
5. **Page Stats + Quality Gates** → route per page (see §4).
6. **Document Analysis**: Margins, body style, real headers/footers (detected by cross-page repetition; `PAGE` fields only when they truly count pages), heading levels, dehyphenation vocabulary, **cross-page continuation table linking**.
7. **Graphics & Annotations**: Strokes/fills, underline/strike/highlight, figures (safe RGB JPEG passthrough only; decode+re-encode CMYK/JPX/SMask/clipped images), `/Annots` (`FreeText` → text, `Ink`/`Stamp` → anchored images, `Highlight` → `w:highlight`).
8. **Segmentation + Mode**: Document-level hysteresis so pages do not flip-flop between `FLOW` and `EXACT`.
9. **Tables**: Struct tree → lattice (ruling lines) → fill/stripe → borderless/stream; content-driven row heights; right/decimal tab stops for label/amount pairs.
10. **Columns + Reading Order**: Struct tree when trusted, else Docling rule-based order / XY-cut.
11. **Paragraphs, Lists, Headings, Hyphenation**: Explicit `w:autoHyphenation` when source is justified + hyphenated.
12. **Fonts & Wrap Prediction**: Map to fonts installed on the user's Mac (no Calibri/Aptos/Marianne without Office; explicit mappings for Aptos and Marianne), measure real advances (`fontKerning` per profile), width/tracking compensation for Pages.
13. **Placement + Self-Check**: Coverage invariant (no text silently dropped).
14. **Serialise**: Format + profile → bytes + `LossReport`.

### Editable Defaults (*Facile à modifier* vs *Fidèle*)
- **Default (`Facile à modifier`)**: Must survive the user's edits (enlarging text by +4 pt, adding sentences):
  - Body leading uses **`AtLeast` or per-renderer `Multiple`** (never `Exact`, which clips enlarged text and French accented capitals like `É`/`À`).
  - **Soft page boundaries** (hard break only when the source page bottom is clearly blank).
  - Cross-page paragraph and table joins on strong evidence.
- **`Fidèle` preset**: Exact leading, hard page breaks, and line-locking.

### Never-Fail Ladder (Per Page)
`Full pipeline` → `Safe layout (no tables/columns)` → `Secondary PDF backend` → `Full-page OCR` → `Page image + extracted text below`.
The user **always** gets a usable file; `ConvertReport` records which rung each page used.

---

## 4. Case Routing

| Case | Detection | Technique |
|---|---|---|
| **Digital letters / admin mail** | Text glyphs, good Unicode, <30% image cover | `FLOW`: real paragraphs, `docDefaults` body style, logo as page-anchored float, real footers |
| **Invoices / bank statements** | Ruling grids, aligned numeric columns, struct-tree `TD` | `FLOW` + tables; XLSX export with typed French numbers/dates; multi-page continuation tables merged |
| **Multi-column** | Whitespace-cover gutters | Section columns (`w:cols` in Word/LibreOffice) or layout table per band (Pages, per S4 capability matrix) |
| **Chrome / Google Docs PDFs** | `U+200B` `ActualText`, `0.75` matrix, one text object per word | Skia normalisation rules, struct tree for lists/tables |
| **Fillable AcroForm** | `/AcroForm` fields | **"Remplir ce formulaire"**: opens directly in Chrome's PDF viewer (or Preview via companion) to fill & attach as-is; conversion is secondary |
| **Flat forms** | Underscores, dotted leaders, boxes | Leader tabs, `☐`, comb tables; dense official forms → `EXACT` |
| **Flyers / brochures** | Overlap, non-Manhattan, many font sizes, art >30% | `EXACT`: 150 dpi background raster + one autofit (`spAutoFit` + 15% height slack) text box per merged block |
| **Text drawn as outlines** | Many small glyph-like filled paths in baseline-aligned rows | Region OCR at 300 dpi on a render → synthetic glyphs into `FLOW`/`EXACT` |
| **Preview / Acrobat Fill & Sign** | `/Annots` `FreeText`, `Ink`, `Stamp`, `Highlight` | `FreeText` → text; `Ink`/`Stamp` → anchored images; `Highlight` → `w:highlight` |
| **Scans** | Full-page image, <20 glyphs | Reuse existing good OCR layer if verified, else OCR → `FLOW`; searchable PDF option |
| **Phone photos** | JPEG/PNG input or single image page | Quad warp, illumination flattening (at ¼–⅛ scale), deskew, orientation → OCR → `FLOW` |
| **Broken text layer** | >5% unmapped / PUA / `U+FFFD` | First try `skrifa` embedded-font `cmap`/`post` inversion; else region OCR keeping geometry; else secondary backend / full OCR |
| **Password-protected** | `NeedsPassword` | Big French password prompt; password kept in worker memory only |
| **Large (40+ pages)** | Page count | Per-page progress, bounded memory, companion recommended for big scans |
| **Rotated pages / text** | `/Rotate`, glyph direction | Landscape sections, rotated cells/text boxes (horizontal fallback in Pages table cells if S4 shows `btLr` breaks); never drop text |

---

## 5. Output Formats & Zero-Warning Pages Profile

| Format | Where | Notes |
|---|---|---|
| **DOCX** (default) | Both | Profiles: **`portable`** (Pages ∩ Word ∩ LibreOffice), `pages`, `word`, `libreoffice`. **Pages profile hard gate**: **zero Pages import warnings** (no default Office `theme1.xml`, all four `w:rFonts` slots `ascii`/`hAnsi`/`eastAsia`/`cs` explicitly set to Mac-installed fonts, `U+202F` → `U+00A0` when needed, no nested tables, no objects inside table cells/headers). |
| **`.pages`** | Companion | AppleScript conversion via `/Applications/Pages.app` (using pre-created file sandbox extension; tested in M0 on this Mac); without companion, `.docx` auto-opens directly in Pages. |
| **Searchable PDF** | Both | Original pages as XObjects + invisible glyphless-font text layer (`hayro-write` + `pdf-writer`). |
| **XLSX** | Both | Tables only, typed French currency/number/date values, multi-page tables linked into one sheet. |
| **HTML / Markdown / TXT** | Both | Semantic + positioned HTML, GFM, plain text. |
| **ODT / RTF** | Both | Own ODF 1.3 and RTF writers (stretch/M6). |

---

## 6. Chrome Extension & Return-Trip UX

- **Gmail Content Script**:
  - Detects PDF and image attachment chips (`span[download_url^="application/pdf:"]`, fallbacks `.hq .aQH > span`, `a[href*="view=att"]`) **and the full-screen preview overlay**.
  - Renders one large French button (`Modifier ce document` or `Remplir ce formulaire` for AcroForms) inside a closed Shadow DOM (zero `innerHTML` for Gmail Trusted Types compliance), debounced against double-clicks, with a selector-miss heartbeat.
- **Reliable Fetch & Handshake**:
  - Content script awaits `ensureOffscreen` from the service worker before opening the job `Port` (prevents cold-start race).
  - Offscreen document fetches the attachment URL (extension origin + host permissions, avoiding CORS), verifies `%PDF-` magic, and detects Gmail virus-scan interstitials.
- **Offscreen Document (`WORKERS` + `BLOBS`)**:
  - Created lazily on first Gmail chip seen or `pointerenter`, closed after ~5 min idle.
  - Hosts the single module worker running `p2d-wasm`; uses **IndexedDB** for font metrics, model blobs, and diagnostics (`chrome.storage` is only available in the service worker).
- **One-Click Auto-Open & Gesture Fallback**:
  - Downloads into `Téléchargements/Documents modifiables/<nom>.docx` with Chrome's *Always open files of this type* enabled and LaunchServices configured to open `.docx` in Pages.
  - Job only marks complete when `downloads.onChanged` reports `opened` or the companion opens it via `NSWorkspace`. If still unopened after 3 s, the pill turns into a large **"Ouvrir le document"** button inside a web-accessible extension `iframe` (providing a genuine user gesture for `chrome.downloads.open`).
- **Return-Trip Helper ("Joindre mon document modifié")**:
  - **In Gmail Reply/Compose**: Adds a clear **"Joindre mon document modifié"** button next to Send.
  - **With Companion**: Exports the frontmost edited Pages/Word document to PDF (or `.docx`) and attaches it to the open Gmail reply in 1 click.
  - **Without Companion**: Opens a pre-filtered picker starting in `Téléchargements/Documents modifiables/` so the user doesn't have to hunt through Finder.
- **Local-Only Quality Feedback**:
  - Subtle `"Pas bien converti ?"` link after conversion records the attachment key and `ConvertReport` locally so the developer can inspect failures later.

---

## 7. Native Mac Companion (M2.5 Minimal First)

**What shipped (2026-10-26):** `Modifier mes PDF.app` is `p2d-cli` itself (universal arm64 + x86_64, Developer ID signed and notarized, built by `scripts/build-macos.sh`, see [`RELEASE-macOS.md`](RELEASE-macOS.md)). Opened from the Finder it runs `app_window`: one `wry`/`tao` window (WKWebView) showing the built-in `extension/ui` pages through a private `app://` scheme — no TCP port — answered by the same router as the developer's web server (`serve::respond`), with French menus (⌘O, text size), PDFs from « Ouvrir avec » / the Dock icon, native folder and file dialogs (`rfd`), and a check for unsaved corrections before closing. `p2d-cli --self-test` loads the pages in a hidden window and exercises the API end to end. Not built (yet): Vision OCR, return trip, Native Messaging, Quick Action.

The original plan: a pure-Rust `objc2` `.app` bundle (`ConvertisseurPDF.app`), acting as both the standalone Mac app and the Chrome Native Messaging host (`argv[1] = chrome-extension://…`):
1. **Apple Vision OCR** (`VNRecognizeTextRequest`, accurate `fr-FR` + `en-US`).
2. **Real `.pages` output** via Pages AppleScript + reliable frontmost window activation (`NSWorkspace`).
3. **1-Click Return Trip**: Exports the active Pages document to PDF for attaching back in Gmail.
4. **CoreText font metrics** + setup installer for OFL metric-compatible fonts (Carlito for Calibri, Caladea for Cambria, Liberation for Arial/Times).
5. **Dock drop / "Ouvrir avec" / Finder Quick Action**.
6. Installed in `~/Applications` with a persistent self-signed certificate so macOS TCC permissions survive updates.

---

## 8. Dev Machine vs Older Intel Mac Strategy

- **This Dev Machine (`arm64` Apple Silicon, `/Applications/Pages.app` installed)**:
  - Runs the full Rust workspace (`aarch64-apple-darwin`, `x86_64-apple-darwin` via Rosetta 2, and `wasm32-unknown-unknown`).
  - Runs the **Pages capability matrix (S4)** and **Pages render-back QA harness** immediately via `osascript` driving local `/Applications/Pages.app`.
  - Benchmarks `x86_64-apple-darwin` builds under Rosetta 2 and WASM builds in headless Chrome with CPU throttling as a fast local proxy before testing on the target Intel Mac.
- **Older Intel Mac**:
  - Used for final M0 speed calibration, Gmail multi-account/auto-open verification, and checks on real-world PDFs.

---

## 9. Quality Harness (`p2d-cli`)

- **Oracle Injection Mode**: Converts PyMuPDF `rawdict` + `texttrace` JSON (from a dev-only Python venv) into `PageIr`, allowing us to validate `p2d-layout` and `p2d-writers` against `pdf2docx` independently of `hayro`/PDFium extraction.
- **Render-Back Gate**: Automates `/Applications/Pages.app` via `osascript` to open generated `.docx`, check for zero Pages import warnings, export back to PDF, and rasterise with PDFium.
- **Editability Hard Gates**:
  - `FLOW` pages must have **0 text boxes**, **0 line-locks** in `Editable` mode, and **0 exact-spaced lines below 1.0× natural height**.
  - **Edit-Perturbation Tests**: Programmatically append ~25 words to a body paragraph, scale all font sizes by +4 pt, and delete a table row → render back in Pages → assert 0 text overlaps, 0 clipped lines, and 0 unexpected blank pages.
- **Determinism**: Assert byte-identical XML inside DOCX between native and WASM builds under a fixed `FontMetricsSnapshot`, and perceptual similarity on re-encoded media.

---

## 10. Roadmap

| Milestone | Scope | Exit Criteria |
|---|---|---|
| **M0 — Spikes & Workspace** | Install `rustup` (`wasm32`, `x86_64-apple-darwin`, `aarch64-apple-darwin`); scaffold the 8-crate workspace + `deny.toml`; **S2 vs S10**: `hayro` (+ `skrifa` cmap fallback) vs `@embedpdf/pdfium` zero-copy binary dump on 60 PDFs; **S4**: 25-fixture Pages DOCX capability matrix + warning check on local `/Applications/Pages.app`; **S3**: MV3 + Gmail plumbing + auto-open test; **S5**: font metrics & wrap prediction. | Written go/no-go records in `docs/decisions/`; workspace compiles cleanly on all 3 targets. |
| **M1-slim — First Usable Release** | Minimal QA harness + PyMuPDF oracle injection (week 1); `FLOW` paragraphs (`AtLeast` leading, `Tc`/`Tw` clustering, French accent joins), safe JPEG images, lattice tables, page-image fallback rung; Gmail chip button + drop page + `"Ouvrir"` gesture fallback + AcroForm `"Remplir ce formulaire"` shortcut. | ≥20 real/representative French PDFs: 0 crashes, word recall ≥99.5%, 0 glued words, 0 Pages warnings, beats `pdf2docx`→Pages baseline, passes +4 pt edit-perturbation test. |
| **M2 — Full Digital Layout** | Borderless/stream tables, cross-page continuation tables, real headers/footers, lists, headings, Gmail preview-overlay button, return-trip compose picker. | Table & list F1 targets met; CI regression gates active. |
| **M2.5 — Companion Minimal** | Native-messaging host, Apple Vision OCR, real `.pages` save via Pages, 1-click `"Joindre mon document modifié"` return trip, `NSWorkspace` open-in-front, OFL clone font installer. | Scans + real `.pages` + 3-click return trip working end-to-end. |
| **M3 — Columns, Fonts & Speed** | Multi-column (`w:cols` / Pages band tables), rotation, `/Annots` (Preview Fill & Sign), runtime font wrap prediction, Pages profile finalised. | Reading order ≥95%, 40 digital pages ≤10 s. |
| **M4 — Standalone WASM OCR** | `p2d-ocr` (`rten` + PP-OCRv6 in WASM), phone photo quad-warp & ¼-scale illumination flattening, outline-text region OCR, searchable PDF writer. | Scans in extension-only mode: CER ≤1.5%, amounts exact ≥99%. |
| **M5 — EXACT Mode & Flyers** | `hayro` render-without-text background + autofit text boxes, mode hysteresis, full never-fail ladder, fuzzing. | Flyer SSIM ≥0.9 with ≥99% live editable text. |
| **M6 — Extra Formats & Polish** | XLSX, HTML/MD/TXT, ODT, RTF, Chrome Web Store Private listing, diagnostics. | Every format opens cleanly; 1 month of daily use with 0 unresolved failures. |
