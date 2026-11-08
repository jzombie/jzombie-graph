use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use jzombie_graph::{output, pipeline};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Json,
    /// Full vertical dump: every node + every edge with all fields. Debug lane.
    #[value(alias = "table")]
    Dump,
    Dot,
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

    /// Output format: json (default) | dump (full debug) | dot (dev eyeballing).
    #[arg(long, value_enum, default_value = "json")]
    format: Format,

    /// Write to file instead of stdout.
    #[arg(long)]
    output: Option<PathBuf>,

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let has_list = cli.repos.as_ref().is_some_and(|r| !r.is_empty());
    let has_enum = cli.user.is_some() || cli.org.is_some();
    anyhow::ensure!(
        has_list ^ has_enum,
        "pass exactly one of --repos OR --user/--org"
    );

    let graph = if std::env::var("JZOMBIE_GRAPH_OFFLINE").is_ok() {
        anyhow::bail!(
            "offline mode: no cached repos; pass --repos with a local stub or unset JZOMBIE_GRAPH_OFFLINE for live fetch"
        )
    } else {
        pipeline::collect_live(
            cli.user.clone(),
            cli.org.clone(),
            cli.repos.clone().unwrap_or_default(),
            &cli.token,
            cli.allow_unauthenticated,
        )
        .await?
    };

    // Default output is the public subgraph (store-safe): private repos
    // compute as transit nodes, stripped before render — no nodes, stubs,
    // links, counts, or deltas. The true graph is briefing-lane only.
    let rendered = pipeline::select_output(graph, cli.include_private);

    let text = match cli.format {
        Format::Json => output::render_json(&rendered)?,
        Format::Dump => output::render_dump(&rendered),
        Format::Dot => output::render_dot(&rendered),
    };
    if let Some(path) = cli.output {
        std::fs::write(&path, &text).with_context(|| format!("writing {}", path.display()))?;
    } else {
        print!("{text}");
    }
    Ok(())
}
