use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use jzombie_graph::{output, paths, pipeline};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq)]
enum Format {
    Json,
    /// Full vertical dump: every node + every edge with all fields. Debug lane.
    #[value(alias = "table")]
    Dump,
    Dot,
    /// Focus ranking from collected JSON: commit heat vs own baseline
    /// minus idle months. Offline — reads a file, no API calls.
    Focus,
    /// JSON Schema for the graph contract, derived from the Rust types.
    /// Static — no selector, no API, no graph data.
    Schema,
}

#[derive(Parser, Debug)]
#[command(
    name = "jzombie-graph",
    about = "Visibility-flagged portfolio graph collector"
)]
struct Cli {
    /// GitHub user to enumerate.
    #[arg(long, conflicts_with = "org")]
    user: Option<String>,

    /// GitHub org to enumerate.
    #[arg(long)]
    org: Option<String>,

    /// Explicit repo list `owner/repo,...` (skips enumeration).
    #[arg(long, value_delimiter = ',')]
    repos: Option<Vec<String>>,

    /// Read a collected graph.json instead of hitting the API (offline).
    /// Pairs with `--format focus` (rank stored nodes) or any other format
    /// to re-render a stored collection without re-collecting.
    #[arg(long)]
    input: Option<PathBuf>,

    /// Output format: json (default) | dump (full debug) | dot (dev eyeballing)
    /// | focus (offline ranking) | schema (JSON Schema, no selector needed).
    #[arg(long, value_enum, default_value = "json")]
    format: Format,

    /// Write to file instead of stdout.
    /// Default per lane (`paths::*`): json -> GRAPH_JSON,
    /// schema -> SCHEMA_JSON; dump/dot/focus default to stdout.
    /// Explicit `--output` beats the default; `--stdout` beats both.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Print to stdout even when the lane has a default file path.
    #[arg(long, default_value_t = false)]
    stdout: bool,

    /// Include private repos in output (briefing lane only). Default output
    /// is the public subgraph: private nodes/edges stripped at write time,
    /// never committed to the public repo.
    #[arg(long, default_value_t = false)]
    include_private: bool,

    /// GitHub token (`JZOMBIE_GRAPH_TOKEN` env — secret-safe name, since GitHub
    /// rejects secret names starting with `GITHUB_`). Required by default;
    /// empty only with explicit `--allow-unauthenticated` (public-only, 60/hr).
    #[arg(long, env = "JZOMBIE_GRAPH_TOKEN", default_value = "")]
    token: String,

    /// Allow empty token (explicit opt-in only, never silent). Public repos
    /// only at 60/hr; private transit and traffic tiers still need a token.
    #[arg(long, default_value_t = false)]
    allow_unauthenticated: bool,

    /// Gauge scope: headlines sum all branches, or default-branch-only.
    /// `branch_heat` always carries every branch either way.
    #[arg(long, value_enum, default_value = "all")]
    velocity_scope: jzombie_graph::graph::VelocityScope,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // `--format schema` short-circuits: it is derived from the Rust types
    // alone, so it takes no selector, no token, and no graph data — and it
    // is byte-identical to the copy the drift test guards (see
    // json/portfolio-graph-schema.json).
    if cli.format == Format::Schema {
        return write_out(
            &output::render_schema(),
            cli.output.as_deref(),
            Some(paths::SCHEMA_JSON),
            cli.stdout,
        );
    }

    let has_list = cli.repos.as_ref().is_some_and(|r| !r.is_empty());
    let has_enum = cli.user.is_some() || cli.org.is_some();
    let has_input = cli.input.is_some();
    let has_live = has_list || has_enum;
    anyhow::ensure!(
        !(has_input && has_live),
        "--input is offline: it takes no --repos/--user/--org"
    );
    anyhow::ensure!(
        !has_live || (has_list ^ has_enum),
        "pass exactly one of --repos OR --user/--org"
    );

    let graph = if let Some(path) = &cli.input {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        jzombie_graph::graph::Graph::from_json(&text)?
    } else if has_live {
        if std::env::var("JZOMBIE_GRAPH_OFFLINE").is_ok() {
            anyhow::bail!(
                "offline mode: no cached repos; pass --repos with a local stub or unset JZOMBIE_GRAPH_OFFLINE for live fetch"
            )
        }
        pipeline::collect_live(
            cli.user.clone(),
            cli.org.clone(),
            cli.repos.clone().unwrap_or_default(),
            &cli.token,
            cli.allow_unauthenticated,
            cli.velocity_scope,
        )
        .await?
    } else {
        // No selector, no --input: offline re-render of the default file.
        // `cargo run -- --format focus` just works after one ingest.
        let text = std::fs::read_to_string(paths::GRAPH_JSON).with_context(|| {
            format!(
                "reading {} (run a live collect first: --user/--org/--repos)",
                paths::GRAPH_JSON
            )
        })?;
        jzombie_graph::graph::Graph::from_json(&text)?
    };

    // Default output is the public subgraph (store-safe): private repos
    // compute as transit nodes, stripped before render — no nodes, stubs,
    // links, counts, or deltas. The true graph is briefing-lane only.
    let rendered = pipeline::select_output(graph, cli.include_private);

    let text = match cli.format {
        Format::Json => output::render_json(&rendered)?,
        Format::Dump => output::render_dump(&rendered),
        Format::Dot => output::render_dot(&rendered),
        Format::Focus => output::render_focus(&rendered, &chrono::Utc::now()),
        // Handled by the early return above; stated as a real branch rather
        // than `unreachable!()` so the lane stays correct if that moves.
        Format::Schema => {
            return write_out(
                &output::render_schema(),
                cli.output.as_deref(),
                Some(paths::SCHEMA_JSON),
                cli.stdout,
            );
        }
    };
    // json is the collected artifact: default file, stdout only on request.
    // Dump/dot/focus are eyeballing lanes: stdout unless --output says file.
    // The schema rides with json: one ingest refreshes both data copies,
    // so the data tree never holds a stale contract. Explicit --output or
    // --stdout means "exactly what I asked" — no side effects.
    let default = match cli.format {
        Format::Json => Some(paths::GRAPH_JSON),
        Format::Dump | Format::Dot | Format::Focus => None,
        Format::Schema => Some(paths::SCHEMA_JSON),
    };
    if cli.format == Format::Json && !cli.stdout && cli.output.is_none() {
        output::write_file(
            std::path::Path::new(paths::SCHEMA_JSON),
            &output::render_schema(),
        )?;
        eprintln!("schema: refreshed {}", paths::SCHEMA_JSON);
    }
    write_out(&text, cli.output.as_deref(), default, cli.stdout)
}

/// One write path for every lane: `--stdout` or no default prints;
/// `--output PATH` or the lane default writes the bytes verbatim.
/// No trailing-newline games — what a lane renders is what lands on disk,
/// which is what the published assets depend on.
fn write_out(
    text: &str,
    output: Option<&std::path::Path>,
    default: Option<&str>,
    stdout: bool,
) -> Result<()> {
    if stdout {
        print!("{text}");
    } else if let Some(path) = output {
        output::write_file(path, text)?;
    } else if let Some(path) = default {
        output::write_file(std::path::Path::new(path), text)?;
    } else {
        print!("{text}");
    }
    Ok(())
}
