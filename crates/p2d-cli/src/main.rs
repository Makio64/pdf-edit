#[cfg(target_os = "macos")]
mod app_window;
mod assets;
mod corpus;
mod fidelity;
mod pages_app;
mod pages_matrix;
mod places;
mod qa;
mod serve;
mod settings;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use p2d_engine::convert_pdf_never_fail;
use p2d_ir::{OutputFormat, Params};
use p2d_pdf::{ExtractOptions, HayroBackend, PdfBackend};

fn main() {
    let args: Vec<String> = env::args().collect();
    // Opened from the Finder, the program inside « PDF Edit.app » gets no command (old macOS
    // versions pass a `-psn_…` process serial number): it is the Mac app.
    let in_app_bundle = env::current_exe().is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"));
    let cmd = match args.get(1).map(|s| s.as_str()) {
        None => if in_app_bundle { "window" } else { "help" },
        Some(arg) if in_app_bundle && arg.starts_with("-psn_") => "window",
        Some(arg) => arg,
    };

    match cmd {
        "convert" => {
            if args.len() < 4 {
                eprintln!("Usage: p2d-cli convert <input.pdf> <output.docx|html|md|txt|pdf> [--fidele|--exact]");
                std::process::exit(1);
            }
            let input_path = PathBuf::from(&args[2]);
            let output_path = PathBuf::from(&args[3]);
            let pdf_bytes = fs::read(&input_path).expect("Failed to read input PDF");

            let ext = output_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("docx");
            let format = match ext {
                "pages" => OutputFormat::Pages,
                "html" => OutputFormat::Html,
                "md" => OutputFormat::Markdown,
                "txt" => OutputFormat::Txt,
                "pdf" => OutputFormat::SearchablePdf,
                _ => OutputFormat::Docx,
            };
            if format == OutputFormat::Pages && p2d_writers::pages::blank_template().is_none() {
                eprintln!("Writing .pages needs Pages 14.5's Blank/ISO.template in /Applications/Pages.app: write a .docx instead.");
                std::process::exit(1);
            }
            let preset = if args.iter().any(|a| a == "--exact" || a == "--preset=exact")
                || args.windows(2).any(|w| w[0] == "--preset" && w[1] == "exact")
            {
                p2d_ir::Preset::Exact
            } else if args.iter().any(|a| a == "--fidele" || a == "--preset=fidele")
                || args.windows(2).any(|w| w[0] == "--preset" && w[1] == "fidele")
            {
                p2d_ir::Preset::Fidele
            } else {
                p2d_ir::Preset::Editable
            };
            let params = Params {
                preset,
                format,
                ..Default::default()
            };

            let (out_bytes, _flow_doc, report) =
                convert_pdf_never_fail(&pdf_bytes, None, &params).expect("Conversion failed");
            fs::write(&output_path, &out_bytes).expect("Failed to write output file");
            println!(
                "Converted {} -> {} ({} pages, {} words, {} bytes)",
                input_path.display(),
                output_path.display(),
                report.page_count,
                report.total_words,
                out_bytes.len()
            );
        }
        "dump-ir" => {
            if args.len() < 3 {
                eprintln!("Usage: p2d-cli dump-ir <input.pdf>");
                std::process::exit(1);
            }
            let input_path = PathBuf::from(&args[2]);
            let pdf_bytes = fs::read(&input_path).expect("Failed to read input PDF");
            let backend = HayroBackend::new();
            let (doc, cmap_rec) = backend
                .extract_document(Arc::from(pdf_bytes), None, &ExtractOptions::full())
                .expect("Extraction failed");
            println!(
                "{}",
                serde_json::to_string_pretty(&(doc, cmap_rec)).unwrap()
            );
        }
        "render-png" => {
            if args.len() < 4 {
                eprintln!("Usage: p2d-cli render-png <input.pdf> <output.png> [page_index]");
                std::process::exit(1);
            }
            let input_path = PathBuf::from(&args[2]);
            let output_path = PathBuf::from(&args[3]);
            let page_idx = args.get(4).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
            let pdf_bytes = fs::read(&input_path).expect("Failed to read input PDF");
            let backend = HayroBackend::new();
            let png_bytes = backend
                .render_page_png(Arc::from(pdf_bytes), None, page_idx, 144.0)
                .expect("Failed to render page to PNG");
            fs::write(&output_path, &png_bytes).expect("Failed to write PNG");
            println!("Rendered page {} -> {} ({} bytes)", page_idx, output_path.display(), png_bytes.len());
        }
        "edit-lines" => {
            // List the editable lines of a PDF (direct editing), one per row: page, id, text.
            let pdf_bytes = fs::read(args.get(2).expect("Usage: p2d-cli edit-lines <in.pdf>")).expect("read PDF");
            let pdf = p2d_engine::open_for_editing(&pdf_bytes, None).expect("open PDF");
            // Columns: page, id, x0, x1, baseline, size, char/word spacing, stretch, alignment,
            // styles (installed look-alike|embedded font, characters), text.
            for (pi, lines) in pdf.pages.iter().enumerate() {
                for l in lines {
                    let styles: Vec<String> = l
                        .runs
                        .iter()
                        .map(|r| {
                            let own = pdf.run_font(pi, r).map_or("-".to_string(), |f| pdf.fonts[f].family.clone());
                            format!("{}{}|{own}:{}", r.font_family, if r.bold { " Bold" } else { "" }, r.len)
                        })
                        .collect();
                    println!(
                        "{pi}\t{}\t{:.1}\t{:.1}\t{:.1}\t{:.2}\t{:.3}\t{:.3}\t{:.2}\t{:?}\t{}\t{}",
                        l.id,
                        l.bbox.x0,
                        l.bbox.x1,
                        l.baseline,
                        l.font_size_pt,
                        l.char_spacing_pt,
                        l.word_spacing_pt,
                        l.horiz_scale,
                        l.align,
                        styles.join(" + "),
                        l.text
                    );
                }
            }
        }
        "edit" => {
            // p2d-cli edit <in.pdf> <out.pdf> <page> <line_id> <new text> [<page> <line_id> <text>]...
            if args.len() < 7 || (args.len() - 4) % 3 != 0 {
                eprintln!("Usage: p2d-cli edit <in.pdf> <out.pdf> <page> <line_id> <text> [...]");
                std::process::exit(1);
            }
            let pdf_bytes = fs::read(&args[2]).expect("read PDF");
            let pdf = p2d_engine::open_for_editing(&pdf_bytes, None).expect("open PDF");
            let edits: Vec<p2d_engine::LineEdit> = args[4..]
                .chunks(3)
                .map(|c| p2d_engine::LineEdit {
                    page_index: c[0].parse().expect("page index"),
                    line_id: c[1].parse().expect("line id"),
                    text: c[2].clone(),
                    runs: Vec::new(),
                })
                .collect();
            match p2d_engine::save_line_edits(&pdf, &edits) {
                Ok(saved) => {
                    fs::write(&args[3], &saved.pdf).expect("write PDF");
                    println!(
                        "Edited {} line(s) -> {} ({} bytes, {} covered instead of removed)",
                        edits.len(),
                        args[3],
                        saved.pdf.len(),
                        saved.covered
                    );
                }
                Err(e) => {
                    eprintln!("Edit failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        "fidelity" => {
            // p2d-cli fidelity [--preset exact|fidele|editable] [--min-f1 0.95] [--out <dir>] <in.pdf>...
            let mut preset = p2d_ir::Preset::Exact;
            let mut min_f1: Option<f64> = None;
            let mut out_dir = PathBuf::from("qa/reports/fidelity");
            let mut pdfs = Vec::new();
            let mut it = args[2..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--preset" => {
                        preset = match it.next().map(|s| s.as_str()) {
                            Some("fidele") => p2d_ir::Preset::Fidele,
                            Some("editable") => p2d_ir::Preset::Editable,
                            _ => p2d_ir::Preset::Exact,
                        }
                    }
                    "--min-f1" => min_f1 = it.next().and_then(|s| s.parse().ok()),
                    "--out" => out_dir = it.next().map(PathBuf::from).unwrap_or(out_dir),
                    other => pdfs.push(PathBuf::from(other)),
                }
            }
            if pdfs.is_empty() {
                eprintln!("Usage: p2d-cli fidelity [--preset exact|fidele|editable] [--min-f1 0.95] [--out <dir>] <in.pdf>...");
                std::process::exit(1);
            }
            let scores = fidelity::run_fidelity(&pdfs, &out_dir, preset);
            println!("{:<44} | {:>5} | {:>8} | {:>6}", "Document", "Pages", "F1@1.5px", "F1@3px");
            println!("{}", "-".repeat(74));
            let mut failed = false;
            for s in &scores {
                if let Some(err) = &s.error {
                    println!("{:<44} | {err}", s.name);
                    failed = true;
                    continue;
                }
                let tight = s.mean_f1(true);
                let below = min_f1.is_some_and(|m| tight < m) || !s.page_count_matches;
                failed |= below;
                println!(
                    "{:<44} | {:>5} | {:>8.3} | {:>6.3}{}",
                    s.name.chars().take(44).collect::<String>(),
                    s.pages.len(),
                    tight,
                    s.mean_f1(false),
                    if !s.page_count_matches { "  (page count differs)" } else if below { "  BELOW" } else { "" }
                );
            }
            if failed {
                std::process::exit(1);
            }
        }
        "generate-corpus" => {
            let out_dir = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("qa/corpus/synthetic"));
            let manifest = corpus::generate_corpus(&out_dir).expect("Failed to generate corpus");
            println!(
                "Generated {} synthetic French test PDFs in {}",
                manifest.entries.len(),
                out_dir.display()
            );
        }
        "bakeoff" => {
            let corpus_dir = PathBuf::from("qa/corpus/synthetic");
            let decision_path = PathBuf::from("docs/decisions/01-pdf-backend-bakeoff.md");
            qa::run_m0_bakeoff(&corpus_dir, &decision_path).expect("Bake-off failed");
            println!(
                "M0 Bake-off completed -> {}",
                decision_path.display()
            );
        }
        "pages-matrix" => {
            let out_dir = PathBuf::from("qa/pages-matrix");
            let decision_path = PathBuf::from("docs/decisions/02-pages-capability-matrix.md");
            let run_pages = args.iter().any(|a| a == "--run-pages");
            pages_matrix::run_pages_capability_matrix(&out_dir, &decision_path, run_pages)
                .expect("Pages matrix failed");
            println!(
                "M0 Pages 25-fixture matrix completed -> {}",
                decision_path.display()
            );
        }
        "qa" => {
            let corpus_dir = PathBuf::from("qa/corpus/synthetic");
            let out_dir = PathBuf::from("qa/reports/docx_out");
            let scores = qa::run_qa_harness(&corpus_dir, &out_dir).expect("QA harness failed");
            let mut all_passed = true;
            println!(
                "{:<30} | {:<20} | {:>7} | {:>5} | {:>5} | {:>5} | {:>5} | {:>7}",
                "Document", "Category", "Recall", "Glued", "Edit", "Pert", "Exact", "Time"
            );
            println!("{}", "-".repeat(102));
            for s in &scores {
                let ok = s.word_recall >= 0.995
                    && s.glued_words == 0
                    && s.editability_gates_passed
                    && s.perturbation_passed
                    && s.exact_passed;
                if !ok {
                    all_passed = false;
                }
                println!(
                    "{:<30} | {:<20} | {:>6.1}% | {:>5} | {:>5} | {:>5} | {:>5} | {:>5.2}ms",
                    s.id,
                    s.category,
                    s.word_recall * 100.0,
                    s.glued_words,
                    if s.editability_gates_passed { "PASS" } else { "FAIL" },
                    if s.perturbation_passed { "PASS" } else { "FAIL" },
                    if s.exact_passed { "PASS" } else { "FAIL" },
                    s.elapsed_ms
                );
            }
            if !all_passed {
                eprintln!("Some QA gates failed!");
                std::process::exit(1);
            }
            println!("\nAll {} corpus documents passed all M1-slim & Edit-Perturbation hard gates!", scores.len());
        }
        "window" | "--self-test" => {
            #[cfg(target_os = "macos")]
            app_window::run(cmd == "--self-test");
            #[cfg(not(target_os = "macos"))]
            {
                eprintln!("The windowed app only exists on macOS: use `p2d-cli app` and a browser.");
                std::process::exit(1);
            }
        }
        "serve" | "app" => {
            let port = args
                .get(2)
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(7878);
            let no_open = args.iter().any(|a| a == "--no-open");
            serve::run_visual_server(port, !no_open).expect("Visual server failed");
        }
        _ => {
            println!("p2d-cli — PDF Edit developer & QA harness");
            println!("Commands:");
            println!("  app [port]         Launch the visual web app in the browser (default port 7878)");
            println!("  window             Open the Mac app's window (what « PDF Edit.app » runs)");
            println!("  --self-test        Check the Mac app's pages and API in a hidden window (exit 0 = OK)");
            println!("  convert <in.pdf> <out.docx|html|md|txt|pdf>");
            println!("  dump-ir <in.pdf>");
            println!("  edit-lines <in.pdf>                 List the lines that can be edited in place");
            println!("  edit <in.pdf> <out.pdf> <page> <line> <text> [...]   Replace lines directly in the PDF");
            println!("  generate-corpus [dir]");
            println!("  bakeoff");
            println!("  pages-matrix [--run-pages]");
            println!("  qa");
            println!("  fidelity [--preset exact|fidele|editable] [--min-f1 0.95] <in.pdf>...   Pixel round-trip through Apple Pages");
        }
    }
}
