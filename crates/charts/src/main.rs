//! `charts` binary: chart SVGs out.
//!
//! Input is always the `paths::GRAPH_JSON` working copy (single source,
//! never a duplicated path). Output defaults per chart (`paths::SHARE_SVG`,
//! `paths::FOCUS_SVG`).

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "charts", about = "Chart renderers for jzombie-graph JSON")]
struct Cli {
    /// Which chart to render.
    #[arg(long, value_enum, default_value = "share")]
    chart: Chart,

    /// Write to file instead of stdout.
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, clap::ValueEnum)]
enum Chart {
    Share,
    Focus,
}

fn main() {
    let cli = Cli::parse();

    let text = match std::fs::read_to_string(jzombie_graph::paths::GRAPH_JSON) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("reading {}: {e}", jzombie_graph::paths::GRAPH_JSON);
            std::process::exit(1);
        }
    };
    let svg = match cli.chart {
        Chart::Share => charts::render_share_svg(&text),
        Chart::Focus => charts::render_focus_svg(&text),
    };
    let svg = match svg {
        Ok(s) => s,
        Err(e) => {
            eprintln!("render error: {e:#}");
            std::process::exit(1);
        }
    };
    let default = match cli.chart {
        Chart::Focus => jzombie_graph::paths::FOCUS_SVG,
        Chart::Share => jzombie_graph::paths::SHARE_SVG,
    };
    let dest = cli.output.unwrap_or_else(|| PathBuf::from(default));
    if let Err(e) = jzombie_graph::output::write_file(&dest, &svg) {
        eprintln!("writing {}: {e:#}", dest.display());
        std::process::exit(1);
    }
}
