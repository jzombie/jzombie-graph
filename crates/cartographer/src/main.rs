//! `cartographer` binary: collected JSON in, Markdown doc out.

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "cartographer",
    about = "Reads collector JSON (schema-checked) from INPUT or stdin and writes the architecture Markdown to stdout or --output PATH"
)]
struct Cli {
    /// Input graph JSON file. Reads stdin when absent.
    input: Option<PathBuf>,

    /// Write to file instead of stdout.
    #[arg(long)]
    output: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();

    let text = match &cli.input {
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
    let md = cartographer::render_cartographer(&graph);
    match &cli.output {
        Some(p) => {
            if let Err(e) = jzombie_graph::output::write_file(p, &md) {
                eprintln!("writing {}: {e:#}", p.display());
                std::process::exit(1);
            }
        }
        None => print!("{md}"),
    }
}
