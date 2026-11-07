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

    /// Include private repos in output (briefing lane only). Default output
    /// is the public subgraph: private nodes/edges stripped at write time,
    /// never committed to the public repo.
    #[arg(long, default_value_t = false)]
    include_private: bool,

    /// GitHub token (or GITHUB_TOKEN env). Required by default; empty only
    /// with explicit `--allow-unauthenticated` (public-only, 60/hr).
    #[arg(long, env = "GITHUB_TOKEN", default_value = "")]
    token: String,

    /// Allow empty token (explicit opt-in only, never silent). Public repos
    /// only at 60/hr; private transit and traffic tiers still need a token.
    #[arg(long, default_value_t = false)]
    allow_unauthenticated: bool,
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
        collect_live(&cfg, &cli.token, cli.allow_unauthenticated).await?
    };

    // Default output is the public subgraph (store-safe): private repos
    // compute as transit nodes, stripped before render — no nodes, stubs,
    // links, counts, or deltas. The true graph is briefing-lane only.
    let rendered = select_output(graph, cli.include_private);

    let text = match cli.format {
        Format::Json => output::render_json(&rendered)?,
        Format::Table => output::render_table(&rendered),
        Format::Dot => output::render_dot(&rendered),
    };
    if let Some(path) = cli.output {
        std::fs::write(&path, &text).with_context(|| format!("writing {}", path.display()))?;
    } else {
        print!("{text}");
    }
    Ok(())
}

/// Output selection: public subgraph by default (store-safe, committable);
/// true graph only with explicit opt-in (briefing lane, never commit).
fn select_output(graph: Graph, include_private: bool) -> Graph {
    if include_private {
        eprintln!("note: --include-private: true-graph output (briefing lane only, never commit)");
        graph
    } else {
        graph::public_subgraph(&graph)
    }
}

async fn collect_live(cfg: &Config, token: &str, allow_unauthenticated: bool) -> Result<Graph> {
    let client = GitHubClient::with_base_optional(
        token.to_string(),
        "https://api.github.com".into(),
        allow_unauthenticated,
    )?;
    if token.is_empty() {
        eprintln!("note: --allow-unauthenticated: no token (public repos only, 60/hr)");
    }

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

    // API budget (milestone 1 bound, tree-first): per branch, 1 tree call
    // gates every existence probe — manifests, lockfiles, gitmodules, docs
    // are only fetched when present in the tree, so 404 probes (and their
    // x4 drive-retry multiplier) drop to ~zero except on races. Workflows
    // list only when the tree shows a `.github/workflows/` prefix.
    // Per branch worst case: 1 tree + manifests present (<=2 typical) +
    // locks present (<=2) + gitmodules/docs present (<=3) + 1 workflow list +
    // workflow files <=10 + member manifests <=10 + (default branch only)
    // .rs files <=10. Typical: ~6-8/branch. Sequential today; fan-out in 1.1.
    const WORKFLOW_FILES_CAP: usize = 10;
    const MEMBER_MANIFEST_CAP: usize = 10;
    const RS_FILES_CAP: usize = 10;
    {
        let n_repos = repos.len();
        let n_branches = cfg.collection.max_branches.max(1);
        let per_branch_worst = 1 + 2 + 2 + 3 + 1 + WORKFLOW_FILES_CAP + MEMBER_MANIFEST_CAP;
        let per_repo_worst = 2 + n_branches * per_branch_worst + RS_FILES_CAP; // +ahead/parent, +default-branch rs
        let est_worst = n_repos * per_repo_worst;
        let per_branch_typ = 8;
        let est_typ = n_repos * (2 + n_branches.min(3) * per_branch_typ + RS_FILES_CAP.min(5));
        eprintln!(
            "budget: {n_repos} repos x up to {n_branches} branches: ~{est_typ} API calls typical, ~{est_worst} worst-case \
             (tree-first: 1 tree/branch gates all probes, no 404-retry mult; workflows<=10 + members<=10 + rs<=10 default-branch-only; \
             rate limit 5000/hr)"
        );
    }

    // 2. Per-repo collection, sequential (drive middleware concurrency-limits
    // each request; tokio fan-out across repos lands in 1.1).
    let mut nodes: Vec<Node> = vec![];
    let mut all_edges: Vec<Edge> = vec![];
    // Briefing digest lines: every phantom nullification + lockfile gate drop.
    let mut digest: Vec<String> = vec![];

    for repo in &repos {
        let ahead_by = match client.ahead_by(repo).await {
            Ok(v) => {
                if repo.fork && v.is_none() {
                    digest.push(format!(
                        "{}: ahead_by unavailable (private/deleted upstream?) — fail-open to hidden unless load-bearing",
                        repo.full_name
                    ));
                }
                v
            }
            Err(_) => None,
        };
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

            // Tree-first: one tree call gates every existence probe on this
            // branch. Files are only fetched when present in the tree, so
            // missing-file 404s (x4 drive-retry each) drop to ~zero.
            let tree_set: Option<BTreeSet<String>> = if b.sha.is_empty() {
                None // max_branches=0 path: no SHA, fall back to blind probes
            } else {
                match client.tree_paths(&repo.full_name, &b.sha).await {
                    Ok(listing) => {
                        if listing.truncated {
                            digest.push(format!(
                                "{}@{}: git tree truncated — workspace members/.rs list incomplete",
                                repo.full_name, b.name
                            ));
                        }
                        Some(listing.paths.into_iter().collect())
                    }
                    Err(_) => None, // tree failed: fall back to blind probes
                }
            };
            let has = |p: &str| -> bool {
                match &tree_set {
                    Some(set) => set.contains(p),
                    None => true, // no tree: probe (fail-open, may 404)
                }
            };

            // Manifests: Cargo.toml first, then other ecosystems.
            // declaring_members is MAX across branches (not sum): the same
            // workspace members re-declared on N branches are still N members,
            // not N x members. Branches union via declaring_branches, weight via max.
            let cargo_text: Option<String> = if has("Cargo.toml") {
                match client
                    .file_content(&repo.full_name, "Cargo.toml", &b.name)
                    .await
                {
                    Ok(Some(text)) => Some(text),
                    _ => None,
                }
            } else {
                None
            };
            let cargo_present = has("Cargo.toml");
            if let Some(text) = cargo_text
                && let Ok(m) = collect::parse_cargo_manifest(&text)
            {
                for (k, v) in m.deps {
                    manifest_union
                        .deps
                        .entry(k.clone())
                        .and_modify(|d: &mut collect::ManifestDep| {
                            d.declaring_members = d.declaring_members.max(v.declaring_members);
                            d.optional = d.optional && v.optional;
                        })
                        .or_insert(v);
                }
                manifest_union.workspace_members.extend(m.workspace_members);
                manifest_union.workspace_members.sort();
                manifest_union.workspace_members.dedup();
            }
            // Workspace members walked via git trees, collapsed to repo node.
            // Use-site scan runs on the default branch only (budget bound).
            let is_default = b.name == repo.default_branch;
            if cargo_present && let Some(set) = &tree_set {
                // Fetch member Cargo.tomls (cap MEMBER_MANIFEST_CAP to bound API cost).
                let member_files: Vec<_> = set
                    .iter()
                    .filter(|p| p.ends_with("/Cargo.toml") && *p != "Cargo.toml")
                    .take(MEMBER_MANIFEST_CAP)
                    .cloned()
                    .collect();
                for mf in member_files {
                    if let Ok(Some(mt)) = client.file_content(&repo.full_name, &mf, &b.name).await
                        && let Ok(mm) = collect::parse_cargo_manifest(&mt)
                    {
                        for (k, v) in mm.deps {
                            manifest_union
                                .deps
                                .entry(k.clone())
                                .and_modify(|d: &mut collect::ManifestDep| {
                                    d.declaring_members =
                                        d.declaring_members.max(v.declaring_members);
                                })
                                .or_insert(v);
                        }
                    }
                }
                // Rust use-site check: default branch only, .rs files capped.
                // Other branches reuse the default-branch verdict (fail-open match
                // when unscanned). Zero references nullifies (phantom dep).
                if is_default {
                    let total_rs = set.iter().filter(|p| p.ends_with(".rs")).count();
                    let rs_files: Vec<_> = set
                        .iter()
                        .filter(|p| p.ends_with(".rs"))
                        .take(RS_FILES_CAP)
                        .cloned()
                        .collect();
                    if total_rs > rs_files.len() {
                        digest.push(format!(
                                "{}@{}: .rs file list capped at {RS_FILES_CAP} of {total_rs} — use-site verdict is partial",
                                repo.full_name, b.name
                            ));
                    }
                    if !rs_files.is_empty() {
                        let mut contents: Vec<String> = vec![];
                        for f in rs_files.iter() {
                            if let Ok(Some(c)) =
                                client.file_content(&repo.full_name, f, &b.name).await
                            {
                                contents.push(c);
                            }
                        }
                        for pkg in manifest_union.deps.keys().cloned().collect::<Vec<_>>() {
                            let used = contents
                                .iter()
                                .any(|src| collect::rust_crate_used(&pkg, src));
                            use_sites
                                .entry(pkg.clone())
                                .and_modify(|e| *e = *e || used)
                                .or_insert(used);
                            if !used {
                                digest.push(format!(
                                        "{}@{}: phantom-nullified `{}` (zero use-sites in {RS_FILES_CAP} scanned files)",
                                        repo.full_name, b.name, pkg
                                    ));
                            }
                        }
                    }
                }
            }
            if has("package.json")
                && let Ok(Some(text)) = client
                    .file_content(&repo.full_name, "package.json", &b.name)
                    .await
                && let Ok(m) = collect::parse_package_json(&text)
            {
                for (k, v) in m.deps {
                    manifest_union.deps.entry(k).or_insert(v);
                }
            }
            if has("pyproject.toml")
                && let Ok(Some(text)) = client
                    .file_content(&repo.full_name, "pyproject.toml", &b.name)
                    .await
                && let Ok(m) = collect::parse_pyproject(&text)
            {
                for (k, v) in m.deps {
                    manifest_union.deps.entry(k).or_insert(v);
                }
            }
            if has("go.mod")
                && let Ok(Some(text)) = client
                    .file_content(&repo.full_name, "go.mod", &b.name)
                    .await
                && let Ok(m) = collect::parse_go_mod(&text)
            {
                for (k, v) in m.deps {
                    manifest_union.deps.entry(k).or_insert(v);
                }
            }

            // Lockfile gate: presence confirms, absence-with-lockfile deletes.
            // Tree-gated: only fetched when the lockfile is in the tree.
            for lock in ["Cargo.lock", "package-lock.json"] {
                if !has(lock) {
                    continue;
                }
                if let Ok(Some(text)) = client.file_content(&repo.full_name, lock, &b.name).await
                    && let Some(names) = collect::parse_lockfile_names(lock, &text)
                {
                    saw_lockfile = true;
                    lock_union.extend(names);
                    let _ = &mut lock_versions; // versions recorded per-edge later
                }
            }

            // Submodules: .gitmodules -> owner/repo pins (1.0 flat).
            // Tree-gated: skipped when absent from the tree.
            if has(".gitmodules")
                && let Ok(Some(text)) = client
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
            // Tree-gated: skip the list call when no workflow prefix in tree.
            let has_workflows = match &tree_set {
                Some(set) => set.iter().any(|p| p.starts_with(".github/workflows/")),
                None => true,
            };
            if has_workflows
                && let Ok(files) = client
                    .list_dir(&repo.full_name, ".github/workflows", &b.name)
                    .await
            {
                for f in files.iter().take(WORKFLOW_FILES_CAP) {
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

            // Docs: README.md + llms.txt links to siblings. Tree-gated.
            for doc in ["README.md", "llms.txt"] {
                if !has(doc) {
                    continue;
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_graph_with_private() -> Graph {
        let mut nodes = vec![
            Node {
                id: "me/pub".into(),
                owner: "me".into(),
                name: "pub".into(),
                fork: false,
                private: false,
                upstream: None,
                ahead_by: None,
                branches: vec!["main".into()],
                visible: true,
                visible_reason: None,
            },
            Node {
                id: "me/secret".into(),
                owner: "me".into(),
                name: "secret".into(),
                fork: false,
                private: true,
                upstream: None,
                ahead_by: None,
                branches: vec!["main".into()],
                visible: false,
                visible_reason: None,
            },
        ];
        let edges = vec![Edge {
            source: "me/pub".into(),
            target: "me/secret".into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into()],
            recency: None,
            weight: 1.0,
            version: None,
            note: None,
        }];
        graph::apply_visibility(&mut nodes, &edges);
        Graph::new(nodes, edges)
    }

    #[test]
    fn default_output_strips_private_opt_in_keeps() {
        let g = test_graph_with_private();
        assert_eq!(g.nodes.len(), 2);
        let pub_out = select_output(g.clone(), false);
        assert!(pub_out.nodes.iter().all(|n| !n.private));
        assert!(pub_out.edges.is_empty());
        let priv_out = select_output(g, true);
        assert_eq!(priv_out.nodes.len(), 2);
    }
}
