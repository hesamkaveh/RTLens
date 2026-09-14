//! Offline inspector: pipe terminal output in, get analysed output back.
//!
//! Exists so the engine can be exercised — and eyeballed — without building the GUI,
//! granting Accessibility, or owning a Mac at all.
//!
//!   rtlens-dump --html      < fixture.txt > out.html
//!   rtlens-dump --fragment  < fixture.txt
//!   rtlens-dump --json  < fixture.txt
//!   rtlens-dump --copy bidi-safe < fixture.txt

use rtlens_core::{copyback, process, CaptureSource, CopyMode, Options};
use std::io::{Read, Write};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mode = "html";
    let mut copy_mode = CopyMode::Plain;
    let mut opts = Options::default();
    let mut title = String::from("RTLens");

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--html" | "--json" | "--text" | "--fragment" => mode = args[i].trim_start_matches("--"),
            "--copy" => {
                mode = "copy";
                i += 1;
                copy_mode = match args.get(i).map(String::as_str) {
                    Some("bidi-safe") => CopyMode::BidiSafe,
                    Some("markdown") => CopyMode::Markdown,
                    _ => CopyMode::Plain,
                };
            }
            "--title" => {
                i += 1;
                title = args.get(i).cloned().unwrap_or_default();
            }
            "--no-unwrap" => opts.soft_unwrap = false,
            "--no-normalize" => opts.normalize_persian = false,
            "--keep-backticks" => opts.strip_backticks = false,
            other => {
                eprintln!("rtlens-dump: unknown argument {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let doc = process(&input, &opts, CaptureSource::Selection);

    let out = match mode {
        "json" => serde_json::to_string_pretty(&doc).expect("Doc is always serialisable"),
        "text" => copyback::render(&doc, CopyMode::Plain),
        "copy" => copyback::render(&doc, copy_mode),
        "fragment" => rtlens_core::render::fragment(&doc),
        _ => rtlens_core::render::standalone(&doc, &title),
    };
    std::io::stdout().write_all(out.as_bytes())?;
    std::io::stdout().write_all(b"\n")
}
