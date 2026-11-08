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
    Indicators,
    Preview,
}

/// Badge files + preview copies + indicator table. Shared by the
/// indicators lane and the full preview build.
fn write_indicator_badges() -> anyhow::Result<()> {
    for name in charts::indicator_names() {
        let svg = charts::render_indicator_svg(&name)?;
        let dest =
            std::path::Path::new(jzombie_graph::paths::INDICATORS_DIR).join(format!("{name}.svg"));
        jzombie_graph::output::write_file(&dest, &svg)?;
        let preview = std::path::Path::new(jzombie_graph::paths::PREVIEW_INDICATORS_DIR)
            .join(format!("{name}.svg"));
        jzombie_graph::output::write_file(&preview, &svg)?;
    }
    let md = charts::render_indicators_md()?;
    jzombie_graph::output::write_file(
        std::path::Path::new(jzombie_graph::paths::PREVIEW_INDICATORS_MD),
        &md,
    )?;
    Ok(())
}

/// Full preview tree: every lane rendered, every preview file written,
/// image copies staged. All paths from `paths::` — the script that calls
/// this names nothing.
fn run_preview() -> anyhow::Result<()> {
    let input = std::fs::read_to_string(jzombie_graph::paths::GRAPH_JSON)?;
    for (svg, dest) in [
        (
            charts::render_share_svg(&input)?,
            jzombie_graph::paths::SHARE_SVG,
        ),
        (
            charts::render_focus_svg(&input)?,
            jzombie_graph::paths::FOCUS_SVG,
        ),
        (
            charts::render_indicators_svg()?,
            jzombie_graph::paths::INDICATORS_SVG,
        ),
    ] {
        jzombie_graph::output::write_file(std::path::Path::new(dest), &svg)?;
        std::fs::copy(dest, std::path::Path::new("preview").join(basename(dest)?))?;
    }
    write_indicator_badges()?;
    let frag = std::fs::read_to_string(jzombie_graph::paths::FRAGMENT_MD)?;
    let table = std::fs::read_to_string(jzombie_graph::paths::PREVIEW_INDICATORS_MD)?;
    let md = charts::render_preview_md(
        &frag,
        &basename(jzombie_graph::paths::SHARE_SVG)?,
        &basename(jzombie_graph::paths::FOCUS_SVG)?,
        &table,
    )?;
    jzombie_graph::output::write_file(
        std::path::Path::new(jzombie_graph::paths::PREVIEW_PORTFOLIO_MD),
        &md,
    )?;
    Ok(())
}

fn basename(path: &str) -> anyhow::Result<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("no file name in {path}"))
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
    // Preview builds the whole proof tree — every path from `paths::`,
    // nothing named in any script. Fixed layout: no --stdout/--output.
    if cli.chart == Chart::Preview {
        if cli.stdout || cli.output.is_some() {
            eprintln!("--chart preview writes the fixed preview tree; no --stdout/--output");
            std::process::exit(1);
        }
        if let Err(e) = run_preview() {
            eprintln!("preview error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    // Indicators also drop one-off badge files beside the preview strip
    // plus the committed render proof (table + badge copies) — the strip
    // is the look-check, the files are what tables embed.
    if cli.chart == Chart::Indicators && !cli.stdout && cli.output.is_none() {
        if let Err(e) = write_indicator_badges() {
            eprintln!("render error: {e:#}");
            std::process::exit(1);
        }
        eprintln!("badges: files in {}", jzombie_graph::paths::INDICATORS_DIR);
    }
    let svg = match cli.chart {
        Chart::Share => charts::render_share_svg(&text),
        Chart::Focus => charts::render_focus_svg(&text),
        Chart::Indicators => charts::render_indicators_svg(),
        Chart::Preview => unreachable!("handled by the early return above"),
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
        Chart::Indicators => jzombie_graph::paths::INDICATORS_SVG,
        Chart::Share => jzombie_graph::paths::SHARE_SVG,
        Chart::Preview => unreachable!("handled by the early return above"),
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
