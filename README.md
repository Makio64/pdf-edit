# PDF Edit

**Modifier un PDF sur Mac, simplement : corriger quelques mots, ou le transformer en document Word ou Pages. Gratuit, sans inscription, et vos documents restent sur votre ordinateur.**

PDF Edit (formerly « Modifier mes PDF ») is a free, fully local PDF editor and PDF → Word / Pages converter for macOS, with the same engine running in the browser through WebAssembly. It was built for non-technical French-speaking users: one question per screen, large type, and it always says where the new file is.

- Website: <https://pdf-edit-mac.netlify.app>
- Download for Mac: <https://github.com/Makio64/pdf-edit/releases/latest>
- Try it in the browser: <https://pdf-edit-mac.netlify.app/en-ligne/>

## What it does

1. **Choose a PDF** (or drop it on the window).
2. **Choose what to do with it:**
   - **Corriger quelques mots**: retype lines directly in the PDF. The new text is drawn in the PDF's own typeface, with its spacing and alignment, and the rest of the page does not move.
   - **Modifier dans Pages**: a real `.pages` document, laid out to look like the PDF (needs Apple Pages).
   - **Modifier dans Word**: a `.docx` that opens in Word, Pages or LibreOffice.
3. **Choose where to save it** (Desktop, Downloads or a folder). The original PDF is never changed and existing files are never overwritten.

Password-protected PDFs can be converted (the app asks for the password); correcting a protected PDF directly is not possible yet.

Limits: the interface is in French, and scanned PDFs (pictures of pages, with no text inside) are not recognised yet. The engine has an OCR hook (`p2d-ocr`, `Step::NeedOcr`) but ships no recognizer.

## Privacy

Documents are read, converted and edited on the device: in the Mac app, or in the browser tab for the web version. Nothing is uploaded, there is no account and no advertising.

Two things do go online, and neither carries document content:

- **Fonts.** If a document uses an open-source font the Mac lacks (for example Calibri, replaced by its metric twin Carlito), the app downloads it once from Google Fonts. The browser version fetches such fonts from a public font CDN (jsDelivr). Only the font family name is requested.
- **Website statistics.** The website counts anonymous, cookieless events (page views, conversions, downloads), as described in [docs/WEB.md](docs/WEB.md).

## Requirements

- macOS 11 Big Sur or later, Intel or Apple Silicon (universal app, signed with Developer ID and notarized by Apple).
- The browser version works in recent Chrome, Edge, Safari and Firefox; there, both « Pages » and « Word » produce a `.docx`.

## Building from source

The engine is a Rust workspace (stable toolchain, see `rust-toolchain.toml`):

| Crate | Role |
|---|---|
| `p2d-ir` | Shared model: input glyphs, paths and images, output document, parameters |
| `p2d-pdf` | PDF reading (hayro, git rev `af0dc88`; its renderer is vendored in `vendor/hayro`), font inversion, direct line editing |
| `p2d-layout` | Words, lines, columns, tables, fonts and the pixel-exact layout preset |
| `p2d-writers` | DOCX and other serializers |
| `p2d-engine` | Conversion pipeline |
| `p2d-ocr` | OCR pre- and post-processing |
| `p2d-wasm` | WebAssembly bindings used by the browser version and the Chrome extension |
| `p2d-cli` | Command-line tool, local web app, macOS app window, QA harness |

```bash
pnpm test
```

```bash
pnpm app
```

`pnpm test` runs `cargo test --workspace`; `pnpm app` opens the local web app. The other scripts in `package.json` build the Chrome extension (`pnpm extension`), the signed Mac app and disk image (`pnpm mac:app`, `pnpm mac:release`, see [docs/RELEASE-macOS.md](docs/RELEASE-macOS.md)) and the website (`pnpm web:build`, see [docs/WEB.md](docs/WEB.md)). Design notes are in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and [docs/decisions/](docs/decisions/).

## License

The code is licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option. The « PDF Edit » name and icon (`packaging/macos/AppIcon*`, `packaging/macos/icon/preview.png`, `extension/icons/`, `site/src/*.png`, `site/src/favicon.svg`), and the website's texts and images (`site/src/**/*.html` prose, `site/src/img/`, `site/src/og.png`), are © 2026 Makio and not covered by these licenses. Third-party code, fonts and data are credited in [THIRD_PARTY_NOTICES](THIRD_PARTY_NOTICES).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

Mac, macOS, Pages and Aperçu (Preview) are trademarks of Apple Inc.; Word is a trademark of Microsoft Corporation; Chrome, Gmail and Google Fonts are trademarks of Google LLC. PDF Edit is not affiliated with any of these companies.
