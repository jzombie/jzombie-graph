//! `cathedral` binary: `graph.json` in, Markdown doc out.

use std::path::PathBuf;

fn usage() -> String {
    "usage: cathedral [INPUT] [--output PATH]\n\
     \n\
     Reads collector JSON (schema-checked) from INPUT or stdin and writes\n\
     the architecture Markdown to stdout or --output PATH.\n\
     \n\
     example: cathedral graph.json --output ARCHITECTURE.md"
        .to_string()
}

fn main() {
    let mut input: Option<PathBuf> = None;
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
                if input.is_none() && !other.starts_with('-') {
                    input = Some(PathBuf::from(other));
                } else {
                    eprintln!("unexpected arg: {other}\n\n{}", usage());
                    std::process::exit(2);
                }
            }
        }
    }

    let text = match &input {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("reading {}: {e}", p.display());
                std::process::exit(1);
            }
        },
        None => match std::io::read_to_string(std::io::stdin()) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("reading stdin: {e}");
                std::process::exit(1);
            }
        },
    };
    let graph = match jzombie_graph::graph::Graph::from_json(&text) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("parse error (regen graph.json with the current collector?): {e:#}");
            std::process::exit(1);
        }
    };
    let md = cathedral::render_cathedral(&graph);
    match &output {
        Some(p) => {
            if let Err(e) = std::fs::write(p, &md) {
                eprintln!("writing {}: {e}", p.display());
                std::process::exit(1);
            }
        }
        None => print!("{md}"),
    }
}
