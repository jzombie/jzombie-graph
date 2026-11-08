//! `charts` binary: commit-share pie SVG out.
//!
//! Input is always the `paths::GRAPH_JSON` working copy (single source,
//! never a duplicated path). Output defaults to `paths::SHARE_SVG`.

use std::path::PathBuf;

fn usage() -> String {
    "usage: charts [--output PATH]\n\
     \n\
     Reads the collected JSON (schema-checked) and writes the\n\
     commit-share pie SVG to stdout or --output PATH.\n\
     \n\
     example: charts --output data/svg/portfolio-graph-share.svg"
        .to_string()
}

fn main() {
    let mut output: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                print!("{}", usage());
                return;
            }
            "--output" => {
                let Some(p) = args.next() else {
                    eprintln!("--output needs a PATH\n\n{}", usage());
                    std::process::exit(2);
                };
                output = Some(PathBuf::from(p));
            }
            other => {
                eprintln!("unexpected arg: {other}\n\n{}", usage());
                std::process::exit(2);
            }
        }
    }

    let text = match std::fs::read_to_string(jzombie_graph::paths::GRAPH_JSON) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("reading {}: {e}", jzombie_graph::paths::GRAPH_JSON);
            std::process::exit(1);
        }
    };
    let svg = match charts::render_share_svg(&text) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("render error: {e:#}");
            std::process::exit(1);
        }
    };
    let dest = output.unwrap_or_else(|| PathBuf::from(jzombie_graph::paths::SHARE_SVG));
    if let Err(e) = jzombie_graph::output::write_file(&dest, &svg) {
        eprintln!("writing {}: {e:#}", dest.display());
        std::process::exit(1);
    }
}
