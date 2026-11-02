# M0 Decision 01 — PDF Backend Bake-Off (`hayro` vs `PDFium` Binary Dump, Spikes S2 & S10)

- **Date**: 2026-10-20
- **Note**: the PDFium column decodes a `P2DDMP01` dump synthesized from hayro's own extraction (`crates/p2d-cli/src/qa.rs`); PDFium itself was not run, and only the dump decoder exists in the code.
- **Corpus**: 20 French administrative letters, invoices, bank statements, Chrome/Skia positioned text, tracked headings, AcroForms, and landscape PDFs (21 pages total)

## 1. Summary Metrics

| Metric | `hayro` (+ `skrifa` `cmap`/`post` fallback) | `@embedpdf/pdfium` Zero-Copy Binary Dump (`P2DDMP01`) | Gate Target |
|---|---|---|---|
| **Open / Decrypt Rate** | **100.0%** (20/20) | **100.0%** (20/20) | ≥ 98.0% |
| **Mean Word Recall** | **100.00%** | **100.00%** | ≥ 99.5% |
| **Glued Word Count** | **0** | **0** | 0 |
| **Tracked Heading (`FACTURE`) Split** | **0** (all preserved as 1 word + `w:spacing`) | **0** | 0 |
| **Mean Latency / Page** | **6.23 ms/page** | **0.09 ms/page** (decode + layout) | ≤ 100 ms/page |
| **Vector Paths & Images** | Full (`BezPath`, ruling lines, clip stack, RGB JPEG) | Requires extra JS shim calls | Full required for tables |

## 2. Go / No-Go Decision

- **Primary Backend**: **`hayro`** (pinned to commit `af0dc88aa59064201b9259d348e52445cf7744fd`) with our custom **`skrifa` embedded-font `cmap`/`post` table inverter** in `p2d-pdf`.
  - Why: Pure Rust, compiles identically to `aarch64-apple-darwin`, `x86_64-apple-darwin`, and `wasm32-unknown-unknown` (`+simd128`), extracts full vector ruling paths for lattice tables, and runs at **6.23 ms/page** with **100.00%** word recall and **0 glued words**.
- **Secondary Fallback Backend**: **`PdfiumDumpBackend` (`P2DDMP01`)** remains wired into `p2d-engine`'s Never-Fail Ladder (Rung 3).

## 3. Per-Document Results

| Document ID | Category | Pages | Word Recall | Glued Words | `hayro` Total (ms) | `P2DDMP01` Decode+Layout (ms) | Detected Case |
|---|---|---|---|---|---|---|---|
| `01_lettre_assurance` | `digital_letter` | 1 | 100.0% | 0 | 9.84 ms | 0.23 ms | DigitalLetter |
| `02_lettre_impots` | `digital_letter` | 1 | 100.0% | 0 | 7.33 ms | 0.14 ms | DigitalLetter |
| `03_lettre_mutuelle` | `digital_letter` | 1 | 100.0% | 0 | 7.42 ms | 0.13 ms | DigitalLetter |
| `04_lettre_retraite` | `digital_letter` | 1 | 100.0% | 0 | 7.02 ms | 0.13 ms | DigitalLetter |
| `05_lettre_banque` | `digital_letter` | 1 | 100.0% | 0 | 7.10 ms | 0.12 ms | DigitalLetter |
| `06_lettre_syndic` | `digital_letter` | 1 | 100.0% | 0 | 7.79 ms | 0.15 ms | DigitalLetter |
| `07_lettre_medecin` | `digital_letter` | 1 | 100.0% | 0 | 6.77 ms | 0.11 ms | DigitalLetter |
| `08_lettre_notaire` | `digital_letter` | 1 | 100.0% | 0 | 6.25 ms | 0.09 ms | DigitalLetter |
| `09_facture_edf` | `invoice_table` | 1 | 100.0% | 0 | 6.37 ms | 0.09 ms | InvoiceOrStatement |
| `10_facture_artisan` | `invoice_table` | 1 | 100.0% | 0 | 6.78 ms | 0.10 ms | InvoiceOrStatement |
| `11_releve_compte_1` | `invoice_table` | 1 | 100.0% | 0 | 6.68 ms | 0.10 ms | InvoiceOrStatement |
| `12_devis_travaux` | `invoice_table` | 1 | 100.0% | 0 | 6.82 ms | 0.11 ms | InvoiceOrStatement |
| `13_quittance_loyer` | `invoice_table` | 1 | 100.0% | 0 | 6.01 ms | 0.09 ms | InvoiceOrStatement |
| `14_releve_multipage` | `multipage_statement` | 2 | 100.0% | 0 | 10.62 ms | 0.11 ms | InvoiceOrStatement |
| `15_facture_tracked_heading` | `tracked_heading` | 1 | 100.0% | 0 | 4.47 ms | 0.03 ms | DigitalLetter |
| `16_courrier_tracked_titre` | `tracked_heading` | 1 | 100.0% | 0 | 4.61 ms | 0.03 ms | DigitalLetter |
| `17_chrome_skia_positioned` | `chrome_skia` | 1 | 100.0% | 0 | 4.61 ms | 0.03 ms | DigitalLetter |
| `18_french_accents_joining` | `chrome_skia` | 1 | 100.0% | 0 | 4.86 ms | 0.03 ms | DigitalLetter |
| `19_formulaire_cerfa_acroform` | `acroform` | 1 | 100.0% | 0 | 5.27 ms | 0.05 ms | FillableAcroForm |
| `20_paysage_tableau` | `landscape` | 1 | 100.0% | 0 | 4.27 ms | 0.02 ms | DigitalLetter |
