//! `charts` binary: chart SVGs out.
//!
//! Input is always the `paths::GRAPH_JSON` working copy (single source,
//! never a duplicated path). Output defaults per chart (`paths::SHARE_SVG`,
//! `paths::DEVELOPER_FOCUS_SVG`).

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
    /// Default per chart (`paths::SHARE_SVG`, `paths::DEVELOPER_FOCUS_SVG`).
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
    Flame,
    Indicators,
    Preview,
}

/// Badge files + proof copies + indicator table. Shared by the
/// indicators lane and the full proof build.
fn write_indicator_badges() -> anyhow::Result<()> {
    for name in charts::indicator_names() {
        let svg = charts::render_indicator_svg(&name)?;
        let dest =
            std::path::Path::new(jzombie_graph::paths::INDICATORS_DIR).join(format!("{name}.svg"));
        jzombie_graph::output::write_file(&dest, &svg)?;
    }
    let md = charts::render_indicators_md()?;
    jzombie_graph::output::write_file(
        std::path::Path::new(jzombie_graph::paths::PROOF_INDICATORS_MD),
        &md,
    )?;
    Ok(())
}

/// Proof inputs: every lane rendered, image files staged, data table
/// fragment written. The proof page itself is assembled by
/// `oxdock-render` from the map fragment, the commit-data fragment,
/// and the indicator table — this lane only produces those inputs.
/// All paths from `paths::` — the script that calls this names nothing.
fn run_proof() -> anyhow::Result<()> {
    let input = std::fs::read_to_string(jzombie_graph::paths::GRAPH_JSON)?;
    for (svg, dest) in [
        (
            charts::render_share_svg(&input)?,
            jzombie_graph::paths::SHARE_SVG,
        ),
        (
            charts::render_focus_svg(&input)?,
            jzombie_graph::paths::DEVELOPER_FOCUS_SVG,
        ),
        (
            charts::render_flame_svg(&input)?,
            jzombie_graph::paths::FLAME_SVG,
        ),
        (
            charts::render_indicators_svg()?,
            jzombie_graph::paths::INDICATORS_SVG,
        ),
    ] {
        jzombie_graph::output::write_file(std::path::Path::new(dest), &svg)?;
    }
    write_indicator_badges()?;
    let data = charts::render_data_table_md(&input)?;
    jzombie_graph::output::write_file(
        std::path::Path::new(jzombie_graph::paths::COMMIT_DATA_MD),
        &data,
    )?;
    Ok(())
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
    // Proof builds all proof inputs — every path from `paths::`,
    // nothing named in any script. Page assembly is RENDER's job.
    // Fixed layout: no --stdout/--output.
    if cli.chart == Chart::Preview {
        if cli.stdout || cli.output.is_some() {
            eprintln!("--chart preview writes the fixed proof tree; no --stdout/--output");
            std::process::exit(1);
        }
        if let Err(e) = run_proof() {
            eprintln!("proof error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    // Indicators also drop one-off badge files beside the proof strip
    // plus the committed render proof (table + badges) — the strip
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
        Chart::Flame => charts::render_flame_svg(&text),
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
        Chart::Focus => jzombie_graph::paths::DEVELOPER_FOCUS_SVG,
        Chart::Flame => jzombie_graph::paths::FLAME_SVG,
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
