//! `cartographer` binary: collected JSON in, Markdown doc out.
//!
//! Defaults to the standard paths (`paths::GRAPH_JSON` in,
//! `paths::FRAGMENT_MD` out) so bare `cartographer` does the right thing.
//! Explicit `--output` beats the default; `--stdout` beats both. Piped
//! stdin still wins over the default input (pipeline-friendly).

use clap::Parser;
use std::io::IsTerminal as _;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "cartographer",
    about = "Reads collector JSON (schema-checked) and writes the architecture Markdown"
)]
struct Cli {
    /// Input graph JSON file. Defaults to the standard working copy;
    /// piped stdin still wins when present.
    input: Option<PathBuf>,

    /// Write to file instead of stdout. Defaults to the standard fragment.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Print to stdout even though the lane has a default file path.
    #[arg(long, default_value_t = false)]
    stdout: bool,
}

fn main() {
    let cli = Cli::parse();

    let piped = !std::io::stdin().is_terminal();
    // Piped stdin wins when it carries data; an empty pipe (common under
    // `cargo run`, where stdin is closed but not a terminal) falls back
    // to the default file. A graph JSON is never whitespace-only.
    let stdin_text = if piped {
        std::io::read_to_string(std::io::stdin()).unwrap_or_default()
    } else {
        String::new()
    };
    let text = match &cli.input {
        Some(p) => match jzombie_graph::output::read_to_string(p) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("reading {}: {e:#}", p.display());
                std::process::exit(1);
            }
        },
        None if !stdin_text.trim().is_empty() => stdin_text,
        None => match jzombie_graph::output::read_to_string(std::path::Path::new(
            jzombie_graph::paths::GRAPH_JSON,
        )) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "reading {}: {e:#} (run a live collect first?)",
                    jzombie_graph::paths::GRAPH_JSON
                );
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
    let md = cartographer::render_cartographer(&graph);
    if cli.stdout {
        print!("{md}");
        return;
    }
    match &cli.output {
        Some(p) => {
            if let Err(e) = jzombie_graph::output::write_file(p, &md) {
                eprintln!("writing {}: {e:#}", p.display());
                std::process::exit(1);
            }
        }
        None => {
            let dest = std::path::Path::new(jzombie_graph::paths::FRAGMENT_MD);
            if let Err(e) = jzombie_graph::output::write_file(dest, &md) {
                eprintln!("writing {}: {e:#}", dest.display());
                std::process::exit(1);
            }
        }
    }
}
