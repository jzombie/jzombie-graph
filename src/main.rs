use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use jzombie_graph::{
    collect,
    config::Config,
    github::{GitHubClient, RepoMeta},
    graph::{self, Edge, EdgeKind, Graph, Node},
    output,
};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Json,
    Table,
    Dot,
}

#[derive(Parser, Debug)]
#[command(
    name = "jzombie-graph",
    about = "Visibility-flagged portfolio graph collector"
)]
struct Cli {
    /// Config file (values.toml). All behavior is config-driven.
    #[arg(long)]
    config: Option<PathBuf>,

    /// GitHub user to enumerate (overrides config).
    #[arg(long, conflicts_with = "org")]
    user: Option<String>,

    /// GitHub org to enumerate (overrides config).
    #[arg(long)]
    org: Option<String>,

    /// Explicit repo list `owner/repo,...` (overrides config; skips enumeration).
    #[arg(long, value_delimiter = ',')]
    repos: Option<Vec<String>>,

    /// Output format: json (default) | table (human summary) | dot (dev eyeballing).
    #[arg(long, value_enum, default_value = "json")]
    format: Format,

    /// Write to file instead of stdout.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Max branches per repo to union edges across (0 = default branch only).
    #[arg(long)]
    max_branches: Option<usize>,

    /// GitHub token (or GITHUB_TOKEN env). Only needed for enumeration/live fetch.
    #[arg(long, env = "GITHUB_TOKEN", default_value = "")]
    token: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = Config::load(cli.config.as_deref())?;
    if let Some(u) = cli.user.clone() {
        cfg.source.user = Some(u);
        cfg.source.org = None;
        cfg.source.repos.clear();
    }
    if let Some(o) = cli.org.clone() {
        cfg.source.org = Some(o);
        cfg.source.user = None;
        cfg.source.repos.clear();
    }
    if let Some(repos) = cli.repos.clone() {
        cfg.source.repos = repos;
        cfg.source.user = None;
        cfg.source.org = None;
    }
    if let Some(mb) = cli.max_branches {
        cfg.collection.max_branches = mb;
    }
    cfg.validate()?;

    let graph = if std::env::var("JZOMBIE_GRAPH_OFFLINE").is_ok() {
        anyhow::bail!(
            "offline mode: no cached repos; pass --repos with a local stub or unset JZOMBIE_GRAPH_OFFLINE for live fetch"
        )
    } else {
        collect_live(&cfg, &cli.token).await?
    };

    let text = match cli.format {
        Format::Json => output::render_json(&graph)?,
        Format::Table => output::render_table(&graph),
        Format::Dot => output::render_dot(&graph),
    };
    if let Some(path) = cli.output {
        std::fs::write(&path, &text).with_context(|| format!("writing {}", path.display()))?;
    } else {
        print!("{text}");
    }
    Ok(())
}

async fn collect_live(cfg: &Config, token: &str) -> Result<Graph> {
    let client = GitHubClient::new(token.to_string())?;

    // 1. Repo set: explicit list (fetched for fork/private facts) or enumerate.
    let repos: Vec<RepoMeta> = if !cfg.source.repos.is_empty() {
        let mut out = vec![];
        for full in &cfg.source.repos {
            out.push(
                client
                    .get_repo(full)
                    .await
                    .with_context(|| format!("fetching repo {full}"))?,
            );
        }
        out
    } else {
        client
            .enumerate_repos(
                cfg.source.user.as_deref(),
                cfg.source.org.as_deref(),
                cfg.source.include_forks,
            )
            .await?
    };

    let portfolio: BTreeSet<String> = repos.iter().map(|r| r.full_name.clone()).collect();
    let aliases = cfg.alias_map();

    // 2. Per-repo collection (sequential for now; parallel fetches are the
    // reason this is Rust — tokio fan-out lands when rate limits are mapped).
    let mut nodes: Vec<Node> = vec![];
    let mut all_edges: Vec<Edge> = vec![];
    // Briefing digest lines: every phantom nullification + lockfile gate drop.
    let mut digest: Vec<String> = vec![];

    for repo in &repos {
        let ahead_by = client.ahead_by(repo).await.unwrap_or(None);
        let max_b = cfg.collection.max_branches;
        let branches = if max_b == 0 {
            vec![jzombie_graph::github::Branch {
                name: repo.default_branch.clone(),
                sha: String::new(),
            }]
        } else {
            client
                .list_branches(&repo.full_name, max_b)
                .await
                .unwrap_or_else(|_| {
                    vec![jzombie_graph::github::Branch {
                        name: repo.default_branch.clone(),
                        sha: String::new(),
                    }]
                })
        };
        let branch_names: Vec<String> = branches.iter().map(|b| b.name.clone()).collect();

        // Union manifests across branches; each edge carries all declaring branches.
        let mut manifest_union = collect::ManifestSet::default();
        let mut lock_union: BTreeSet<String> = BTreeSet::new();
        let mut saw_lockfile = false;
        let mut lock_versions: HashMap<String, String> = HashMap::new();
        let mut submodule_targets: BTreeSet<String> = BTreeSet::new();
        let mut workflow_targets: BTreeSet<String> = BTreeSet::new();
        let mut docs_targets: BTreeSet<String> = BTreeSet::new();
        // package -> referenced? (None = unscanned -> match)
        let mut use_sites: HashMap<String, bool> = HashMap::new();
        let mut branch_recency: Vec<collect::BranchDecl> = vec![];

        for b in &branches {
            branch_recency.push(collect::BranchDecl {
                branch: b.name.clone(),
                recency: None, // commit-date recency lands with the trees API timestamp
            });

            // Manifests: Cargo.toml first, then other ecosystems.
            if let Ok(Some(text)) = client
                .file_content(&repo.full_name, "Cargo.toml", &b.name)
                .await
            {
                if let Ok(m) = collect::parse_cargo_manifest(&text) {
                    for (k, v) in m.deps {
                        manifest_union
                            .deps
                            .entry(k.clone())
                            .and_modify(|d| {
                                d.declaring_members += v.declaring_members.saturating_sub(1);
                            })
                            .or_insert(v);
                    }
                    manifest_union.workspace_members.extend(m.workspace_members);
                }
                // Workspace members walked via git trees, collapsed to repo node.
                if !b.sha.is_empty()
                    && let Ok(paths) = client.tree_paths(&repo.full_name, &b.sha).await
                {
                    for member in manifest_union.workspace_members.clone() {
                        let prefix = member.trim_end_matches("/*").to_string() + "/Cargo.toml";
                        let _ = prefix;
                    }
                    // Fetch member Cargo.tomls (cap 20 to bound API cost).
                    let member_files: Vec<_> = paths
                        .iter()
                        .filter(|p| p.ends_with("/Cargo.toml") && *p != "Cargo.toml")
                        .take(20)
                        .cloned()
                        .collect();
                    for mf in member_files {
                        if let Ok(Some(mt)) =
                            client.file_content(&repo.full_name, &mf, &b.name).await
                            && let Ok(mm) = collect::parse_cargo_manifest(&mt)
                        {
                            for (k, v) in mm.deps {
                                manifest_union
                                    .deps
                                    .entry(k.clone())
                                    .and_modify(|d| {
                                        d.declaring_members += 1;
                                    })
                                    .or_insert(v);
                            }
                        }
                    }
                    // Rust use-site check: scan .rs files (cap 50) via syn.
                    let rs_files: Vec<_> = paths
                        .iter()
                        .filter(|p| p.ends_with(".rs"))
                        .take(50)
                        .cloned()
                        .collect();
                    if !rs_files.is_empty() {
                        let mut contents: Vec<String> = vec![];
                        for f in rs_files.iter().take(50) {
                            if let Ok(Some(c)) =
                                client.file_content(&repo.full_name, f, &b.name).await
                            {
                                contents.push(c);
                            }
                        }
                        for pkg in manifest_union.deps.keys().cloned().collect::<Vec<_>>() {
                            // Crate name for `use` matching: alias target's
                            // repo name is the common case; fall back to package.
                            let used = contents
                                .iter()
                                .any(|src| collect::rust_crate_used(&pkg, src));
                            use_sites
                                .entry(pkg.clone())
                                .and_modify(|e| *e = *e || used)
                                .or_insert(used);
                            if !used {
                                digest.push(format!(
                                    "{}@{}: phantom-nullified `{}` (zero use-sites)",
                                    repo.full_name, b.name, pkg
                                ));
                            }
                        }
                    }
                }
            }
            if let Ok(Some(text)) = client
                .file_content(&repo.full_name, "package.json", &b.name)
                .await
                && let Ok(m) = collect::parse_package_json(&text)
            {
                for (k, v) in m.deps {
                    manifest_union.deps.entry(k).or_insert(v);
                }
            }
            if let Ok(Some(text)) = client
                .file_content(&repo.full_name, "pyproject.toml", &b.name)
                .await
                && let Ok(m) = collect::parse_pyproject(&text)
            {
                for (k, v) in m.deps {
                    manifest_union.deps.entry(k).or_insert(v);
                }
            }
            if let Ok(Some(text)) = client
                .file_content(&repo.full_name, "go.mod", &b.name)
                .await
                && let Ok(m) = collect::parse_go_mod(&text)
            {
                for (k, v) in m.deps {
                    manifest_union.deps.entry(k).or_insert(v);
                }
            }

            // Lockfile gate: presence confirms, absence-with-lockfile deletes.
            for lock in ["Cargo.lock", "package-lock.json"] {
                if let Ok(Some(text)) = client.file_content(&repo.full_name, lock, &b.name).await
                    && let Some(names) = collect::parse_lockfile_names(lock, &text)
                {
                    saw_lockfile = true;
                    lock_union.extend(names);
                    let _ = &mut lock_versions; // versions recorded per-edge later
                }
            }

            // Submodules: .gitmodules -> owner/repo pins (1.0 flat).
            if let Ok(Some(text)) = client
                .file_content(&repo.full_name, ".gitmodules", &b.name)
                .await
            {
                for (_, url) in collect::parse_gitmodules(&text) {
                    if let Some(target) = collect::github_repo_from_url(&url)
                        && portfolio.contains(&target)
                    {
                        submodule_targets.insert(target);
                    }
                }
            }

            // Workflows: list .github/workflows, scan `uses: owner/repo`.
            if let Ok(files) = client
                .list_dir(&repo.full_name, ".github/workflows", &b.name)
                .await
            {
                for f in files.iter().take(20) {
                    if let Ok(Some(text)) = client.file_content(&repo.full_name, f, &b.name).await {
                        for u in collect::scan_workflow_uses(&text) {
                            // `uses: owner/repo` — strip action subpaths (`me/reusable/.github/...`).
                            let base = u.split('/').take(2).collect::<Vec<_>>().join("/");
                            if portfolio.contains(&base) && base != repo.full_name {
                                workflow_targets.insert(base);
                            }
                        }
                    }
                }
            }

            // Docs: README.md + llms.txt links to siblings.
            for doc in ["README.md", "llms.txt"] {
                if let Ok(Some(text)) = client.file_content(&repo.full_name, doc, &b.name).await {
                    for link in collect::scan_docs_links(&text) {
                        if portfolio.contains(&link) && link != repo.full_name {
                            docs_targets.insert(link);
                        }
                    }
                }
            }
        }

        // Manifest edges (one per target, branches unioned).
        let lock_gate: Option<BTreeSet<String>> =
            if saw_lockfile { Some(lock_union) } else { None };
        let mut edges = collect::build_manifest_edges(
            &repo.full_name,
            &branch_recency,
            &manifest_union,
            &aliases,
            &portfolio,
            lock_gate.as_ref(),
            &use_sites,
            &lock_versions,
        );
        // Submodule / workflow / docs associations (weight 0, displayed only).
        for t in submodule_targets {
            edges.push(Edge {
                source: repo.full_name.clone(),
                target: t,
                kind: EdgeKind::Submodule,
                declaring_branches: branch_names.clone(),
                recency: None,
                weight: 1.0,
                version: None,
                note: None,
            });
        }
        for t in workflow_targets {
            edges.push(Edge {
                source: repo.full_name.clone(),
                target: t,
                kind: EdgeKind::Workflow,
                declaring_branches: branch_names.clone(),
                recency: None,
                weight: 0.0,
                version: None,
                note: None,
            });
        }
        for t in docs_targets {
            edges.push(Edge {
                source: repo.full_name.clone(),
                target: t,
                kind: EdgeKind::Docs,
                declaring_branches: branch_names.clone(),
                recency: None,
                weight: 0.0,
                version: None,
                note: None,
            });
        }
        // Registry (reverse-deps) + xrefs land in milestone 1.1 — same shape.
        all_edges.extend(edges);

        nodes.push(Node {
            id: repo.full_name.clone(),
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            fork: repo.fork,
            private: repo.private,
            upstream: repo.parent_full_name.clone(),
            ahead_by,
            branches: branch_names,
            visible: false, // computed below
            visible_reason: None,
        });
    }

    graph::apply_visibility(&mut nodes, &all_edges);
    let g = Graph::new(nodes, all_edges);

    if !digest.is_empty() {
        eprintln!("-- briefing digest ({} nullifications) --", digest.len());
        for line in digest.iter().take(50) {
            eprintln!("  {line}");
        }
    }
    Ok(g)
}
