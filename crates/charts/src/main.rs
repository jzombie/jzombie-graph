//! `charts` binary: chart SVGs out.
//!
//! One artifact family per invocation — no bundles, no sequencing.
//! The lanes own order (`BUILD_PROOF_INPUTS` runs all five); this binary
//! only computes. Input is always the `paths::GRAPH_JSON` working copy
//! (single source, never a duplicated path). Output defaults per chart
//! (`paths::SHARE_SVG`, `paths::DEVELOPER_FOCUS_SVG`).

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "charts", about = "Chart renderers for jzombie-graph JSON")]
struct Cli {
    /// Which artifact to render. No default — say what you want.
    #[arg(long, value_enum)]
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
    Pins,
    Indicators,
    Data,
}

/// Badge files + indicator table. Written atomically with the strip
/// SVG below: the table embeds the badge files, so splitting them
/// across invocations risks a half-written family.
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

fn main() {
    let cli = Cli::parse();

    let text = match std::fs::read_to_string(&cli.input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("reading {}: {e}", cli.input.display());
            std::process::exit(1);
        }
    };
    // Data emits the machine-readable commit table fragment — one
    // artifact, like every other selection. Bundling lives in the
    // lanes (`BUILD_PROOF_INPUTS`), never here.
    // Fixed layout: no --stdout/--output.
    if cli.chart == Chart::Data {
        if cli.stdout || cli.output.is_some() {
            eprintln!("--chart data writes the fixed fragment path; no --stdout/--output");
            std::process::exit(1);
        }
        let md = match charts::render_data_table_md(&text) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("render error: {e:#}");
                std::process::exit(1);
            }
        };
        if let Err(e) = jzombie_graph::output::write_file(
            std::path::Path::new(jzombie_graph::paths::COMMIT_DATA_MD),
            &md,
        ) {
            eprintln!("writing {}: {e:#}", jzombie_graph::paths::COMMIT_DATA_MD);
            std::process::exit(1);
        }
        return;
    }
    // Pins emit a file family — one SVG per repo plus the fragment
    // table that embeds them. Fixed layout: no --stdout/--output.
    // Stale cards from a previous top-6 never linger: the lane owns the
    // whole `portfolio-graph-pin-*` set, so it clears before writing.
    if cli.chart == Chart::Pins {
        if cli.stdout || cli.output.is_some() {
            eprintln!("--chart pins writes the fixed family paths; no --stdout/--output");
            std::process::exit(1);
        }
        let bundle = match charts::render_pins_bundle(&text) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("render error: {e:#}");
                std::process::exit(1);
            }
        };
        let dir = std::path::Path::new(jzombie_graph::paths::CHARTS_DIR);
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("portfolio-graph-pin-") && name.ends_with(".svg") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        for (rel, content) in &bundle {
            if let Err(e) = jzombie_graph::output::write_file(std::path::Path::new(rel), content) {
                eprintln!("writing {rel}: {e:#}");
                std::process::exit(1);
            }
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
        Chart::Pins => unreachable!("handled by the family return above"),
        Chart::Indicators => charts::render_indicators_svg(),
        Chart::Data => unreachable!("handled by the early return above"),
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
        Chart::Pins => unreachable!("handled by the family return above"),
        Chart::Indicators => jzombie_graph::paths::INDICATORS_SVG,
        Chart::Share => jzombie_graph::paths::SHARE_SVG,
        Chart::Data => unreachable!("handled by the early return above"),
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
    // Section fragments carry the same bust token as pins, so the
    // assembled page refetches these SVGs after every nightly rebuild.
    // Fixed layout: no --stdout/--output.
    let frag: Option<(&str, anyhow::Result<String>)> = match cli.chart {
        Chart::Share => Some((
            jzombie_graph::paths::SHARE_FRAG_MD,
            charts::render_share_md(&text),
        )),
        Chart::Focus => Some((
            jzombie_graph::paths::FOCUS_FRAG_MD,
            charts::render_focus_md(&text),
        )),
        Chart::Flame => Some((
            jzombie_graph::paths::FLAME_FRAG_MD,
            charts::render_flame_md(&text),
        )),
        _ => None,
    };
    if let Some((rel, md)) = frag {
        match md {
            Ok(m) => {
                if let Err(e) = jzombie_graph::output::write_file(std::path::Path::new(rel), &m) {
                    eprintln!("writing {rel}: {e:#}");
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("render error: {e:#}");
                std::process::exit(1);
            }
        }
    }
}
