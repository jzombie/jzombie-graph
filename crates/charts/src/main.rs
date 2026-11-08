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

    /// Input graph JSON file. Defaults to the standard working copy.
    #[arg(long, default_value = jzombie_graph::paths::GRAPH_JSON)]
    input: PathBuf,

    /// Write to file instead of stdout.
    /// Default per chart (`paths::SHARE_SVG`, `paths::FOCUS_SVG`).
    #[arg(long)]
    output: Option<PathBuf>,

    /// Print to stdout even though the lane has a default file path.
    #[arg(long, default_value_t = false)]
    stdout: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, clap::ValueEnum)]
enum Chart {
    Share,
    Focus,
    Glyphs,
}

fn main() {
    let cli = Cli::parse();

    let text = match std::fs::read_to_string(&cli.input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("reading {}: {e}", cli.input.display());
            std::process::exit(1);
        }
    };
    // Glyphs also drop one-off badge files beside the preview strip —
    // the strip is the look-check, the files are what tables embed.
    if cli.chart == Chart::Glyphs && !cli.stdout && cli.output.is_none() {
        for name in charts::glyph_names() {
            match charts::render_glyph_svg(&name) {
                Ok(svg) => {
                    let dest = std::path::Path::new(jzombie_graph::paths::GLYPHS_DIR)
                        .join(format!("{name}.svg"));
                    if let Err(e) = jzombie_graph::output::write_file(&dest, &svg) {
                        eprintln!("writing {}: {e:#}", dest.display());
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("render error: {e:#}");
                    std::process::exit(1);
                }
            }
        }
        eprintln!("badges: files in {}", jzombie_graph::paths::GLYPHS_DIR);
    }
    let svg = match cli.chart {
        Chart::Share => charts::render_share_svg(&text),
        Chart::Focus => charts::render_focus_svg(&text),
        Chart::Glyphs => charts::render_glyphs_svg(),
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
        Chart::Glyphs => jzombie_graph::paths::GLYPHS_SVG,
        Chart::Share => jzombie_graph::paths::SHARE_SVG,
    };
    if cli.stdout {
        print!("{svg}");
        return;
    }
    let dest = cli.output.unwrap_or_else(|| PathBuf::from(default));
    if let Err(e) = jzombie_graph::output::write_file(&dest, &svg) {
        eprintln!("writing {}: {e:#}", dest.display());
        std::process::exit(1);
    }
}
