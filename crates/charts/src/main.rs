//! `charts` binary: chart SVGs out.
//!
//! One artifact family per invocation — no bundles, no sequencing.
//! The lanes own order (`BUILD_PROOF_INPUTS` runs all five); this binary
//! only computes. Input is always the `paths::graph_json` working copy
//! (single source, never a duplicated path). Output defaults per chart
//! (`paths::share_svg`, `paths::developer_focus_svg`).

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "charts", about = "Chart renderers for jzombie-graph JSON")]
struct Cli {
    /// Which artifact to render. No default — say what you want.
    #[arg(long, value_enum)]
    chart: Chart,

    /// Input graph JSON file. Defaults to the standard working copy.
    #[arg(long)]
    input: Option<PathBuf>,

    /// Write to file instead of stdout.
    /// Default per chart (`paths::share_svg`, `paths::developer_focus_svg`).
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
    Languages,
}

/// Badge files for the heat bands. Written atomically with the strip
/// SVG below: table cells embed the badge files, so splitting them
/// across invocations risks a half-written family.
fn write_indicator_badges() -> anyhow::Result<()> {
    for name in charts::indicator_names() {
        let svg = charts::render_indicator_svg(&name)?;
        let dest = jzombie_graph::paths::indicator_badge(&name);
        jzombie_graph::output::write_file(std::path::Path::new(&dest), &svg)?;
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();

    let input = cli
        .input
        .unwrap_or_else(|| PathBuf::from(jzombie_graph::paths::graph_json()));
    let text = match std::fs::read_to_string(&input) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("reading {}: {e}", input.display());
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
        let dest = jzombie_graph::paths::commit_data_fragment();
        if let Err(e) = jzombie_graph::output::write_file(std::path::Path::new(&dest), &md) {
            eprintln!("writing {dest}: {e:#}");
            std::process::exit(1);
        }
        let foot = match charts::render_footer_md(&text) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("render error: {e:#}");
                std::process::exit(1);
            }
        };
        let foot_dest = jzombie_graph::paths::footer_fragment();
        if let Err(e) = jzombie_graph::output::write_file(std::path::Path::new(&foot_dest), &foot) {
            eprintln!("writing {foot_dest}: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    // Pins emit a file family — one SVG per repo plus the fragment
    // table that embeds them. Fixed layout: no --stdout/--output.
    // Stale cards from a previous top-9 never linger: the lane owns the
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
        Chart::Languages => charts::render_languages_svg(&text),
    };
    let svg = match svg {
        Ok(s) => s,
        Err(e) => {
            eprintln!("render error: {e:#}");
            std::process::exit(1);
        }
    };
    let default = match cli.chart {
        Chart::Focus => jzombie_graph::paths::developer_focus_svg(),
        Chart::Flame => jzombie_graph::paths::flame_svg(),
        Chart::Pins => unreachable!("handled by the family return above"),
        Chart::Indicators => jzombie_graph::paths::indicators_svg(),
        Chart::Share => jzombie_graph::paths::share_svg(),
        Chart::Data => unreachable!("handled by the early return above"),
        Chart::Languages => jzombie_graph::paths::languages_svg(),
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
    let frag: Option<(String, anyhow::Result<String>)> = match cli.chart {
        Chart::Share => Some((
            jzombie_graph::paths::share_fragment(),
            charts::render_share_md(&text),
        )),
        Chart::Focus => Some((
            jzombie_graph::paths::focus_fragment(),
            charts::render_focus_md(&text),
        )),
        Chart::Flame => Some((
            jzombie_graph::paths::flame_fragment(),
            charts::render_flame_md(&text),
        )),
        Chart::Languages => Some((
            jzombie_graph::paths::languages_fragment(),
            charts::render_languages_md(&text),
        )),
        _ => None,
    };
    if let Some((rel, md)) = frag {
        match md {
            Ok(m) => {
                if let Err(e) = jzombie_graph::output::write_file(std::path::Path::new(&rel), &m) {
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
