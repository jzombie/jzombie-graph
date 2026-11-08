use crate::{
    collect,
    github::{GitHubClient, RepoMeta},
    graph::{self, Edge, EdgeKind, Graph, Node},
};
use anyhow::{Context, Result};
use std::collections::{BTreeSet, HashMap};

/// Output selection: public subgraph by default (store-safe, committable);
/// true graph only with explicit opt-in (briefing lane, never commit).
pub fn select_output(graph: Graph, include_private: bool) -> Graph {
    if include_private {
        eprintln!("note: --include-private: true-graph output (briefing lane only, never commit)");
        graph
    } else {
        graph::public_subgraph(&graph)
    }
}

/// Provider map, derived live: package name -> `owner/repo` that provides it.
///
/// Reads every portfolio repo's root manifests (Cargo `[package]`,
/// package.json, pyproject, go.mod) plus workspace member Cargo.tomls found
/// via the git tree. Multimap: package -> every providing repo, each list
/// sorted and deduped (deterministic). Collisions are NOT resolved here —
/// the edge pass reports them per-consumer and asserts no edge rather than
/// a wrong one. Truncated trees warn (member list may be partial). No caps —
/// completeness matters here, unlike the budget-bound structure pass.
pub async fn collect_providers(
    client: &GitHubClient,
    repos: &[RepoMeta],
) -> HashMap<String, Vec<String>> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut providers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut sorted: Vec<&RepoMeta> = repos.iter().collect();
    sorted.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    let n = sorted.len();
    for (i, repo) in sorted.iter().enumerate() {
        eprintln!("[providers {}/{}] {}", i + 1, n, repo.full_name);
        let sha = client
            .branch_sha(&repo.full_name, &repo.default_branch)
            .await
            .unwrap_or_default();
        // (path, kind) manifest candidates for name extraction.
        let mut candidates: Vec<(String, &'static collect::ManifestKind)> = vec![];
        if sha.is_empty() {
            // SHA unresolved: blind probes on registry roots, no members.
            for kind in collect::MANIFEST_KINDS {
                candidates.push((kind.file.to_string(), kind));
            }
        } else {
            match client.tree_paths(&repo.full_name, &sha).await {
                Ok(listing) => {
                    if listing.truncated {
                        eprintln!(
                            "warning: {} tree truncated — provider list may be partial",
                            repo.full_name
                        );
                    }
                    for p in &listing.paths {
                        if let Some(kind) = collect::manifest_kind_for_root(p) {
                            candidates.push((p.clone(), kind));
                        } else if let Some(kind) = collect::manifest_kind_for_member(p) {
                            candidates.push((p.clone(), kind));
                        }
                    }
                }
                Err(_) => {
                    for kind in collect::MANIFEST_KINDS {
                        candidates.push((kind.file.to_string(), kind));
                    }
                }
            }
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0));
        candidates.dedup_by(|a, b| a.0 == b.0);
        for (path, kind) in candidates {
            let text = match client
                .file_content(&repo.full_name, &path, &repo.default_branch)
                .await
            {
                Ok(Some(t)) => t,
                _ => continue,
            };
            if let Some(pkg) = (kind.package_name)(&text) {
                providers
                    .entry(pkg)
                    .or_default()
                    .insert(repo.full_name.clone());
            }
        }
    }
    providers
        .into_iter()
        .map(|(k, v)| (k, v.into_iter().collect()))
        .collect()
}

pub async fn collect_live(
    user: Option<String>,
    org: Option<String>,
    explicit_repos: Vec<String>,
    token: &str,
    allow_unauthenticated: bool,
) -> Result<Graph> {
    let client = GitHubClient::with_base_optional(
        token.to_string(),
        "https://api.github.com".into(),
        allow_unauthenticated,
    )?;
    if token.is_empty() {
        eprintln!("note: --allow-unauthenticated: no token (public repos only, 60/hr)");
    }

    // 1. Repo set: explicit list (fetched for fork/private facts) or enumerate.
    // Forks included: filtering is the visibility pass's job, not collection's.
    let repos: Vec<RepoMeta> = if !explicit_repos.is_empty() {
        let mut out = vec![];
        for full in &explicit_repos {
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
            .enumerate_repos(user.as_deref(), org.as_deref(), true)
            .await?
    };

    let portfolio: BTreeSet<String> = repos.iter().map(|r| r.full_name.clone()).collect();
    // Provider map, derived live every run: each portfolio repo's own
    // manifests declare the package names it provides. Nothing checked in,
    // nothing regenerated by hand — rename a package and the next sweep
    // picks it up. Private repos included (transit; never rendered).
    let aliases = collect_providers(&client, &repos).await;
    if aliases.is_empty() {
        eprintln!("warning: no packages discovered in portfolio manifests — no manifest edges.");
    }

    // API budget (default-branch-only, tree-first, two passes): providers pass
    // (1 SHA + 1 tree + root manifests + member Cargo.tomls, uncapped) then the
    // structure pass (1 tree gates every probe; workflows<=10 + members<=10 +
    // rs<=10; +1 parent fetch for forks missing it — the enumerate endpoint
    // returns parent:null; +2 trailing-window commit-velocity probes
    // (5d heat, 30d baseline).
    // Typical ~26/repo; sequential today, fan-out in 1.1.
    const WORKFLOW_FILES_CAP: usize = 10;
    const MEMBER_MANIFEST_CAP: usize = 10;
    const RS_FILES_CAP: usize = 10;
    {
        let n_repos = repos.len();
        // Providers pass: 1 SHA + 1 tree + ~4 roots + members (uncapped, ~10 typical).
        // Structure pass: as before (tree-gated, caps above).
        let per_repo_worst = (2 + 1 + 4 + 40)
            + (2 + 1 + 1 + 2 + 2 + 3 + 1 + WORKFLOW_FILES_CAP + MEMBER_MANIFEST_CAP + RS_FILES_CAP);
        let est_worst = n_repos * per_repo_worst;
        let est_typ = n_repos * 26;
        eprintln!(
            "budget: {n_repos} repos (default branch only): ~{est_typ} API calls typical, ~{est_worst} worst-case \
             (two passes: live provider map + tree-gated structure; no 404-retry mult; rate limit 5000/hr)"
        );
    }

    // 2. Per-repo collection, sequential (drive middleware concurrency-limits
    // each request; tokio fan-out across repos lands in 1.1).
    // Fork parents: the enumerate endpoint returns `parent: null` for forks,
    // so resolve it here (+1 call/fork) — without it `ahead_by` short-returns
    // None for every fork and the divergence rule never fires.
    let mut repos = repos;
    for repo in repos
        .iter_mut()
        .filter(|r| r.fork && r.parent_full_name.is_none())
    {
        if let Ok(full) = client.get_repo(&repo.full_name).await {
            repo.parent_full_name = full.parent_full_name;
        }
    }
    let mut nodes: Vec<Node> = vec![];
    let mut all_edges: Vec<Edge> = vec![];
    // Briefing digest lines: every phantom nullification + lockfile gate drop.
    let mut digest: Vec<String> = vec![];

    let five_days_ago = chrono::Utc::now() - chrono::Duration::days(5);
    let thirty_days_ago = five_days_ago - chrono::Duration::days(25);
    for (i, repo) in repos.iter().enumerate() {
        eprintln!(
            "[{}/{}] {} @{} …",
            i + 1,
            repos.len(),
            repo.full_name,
            repo.default_branch
        );
        let ahead_by = match client.ahead_by(repo).await {
            Ok(v) => {
                if repo.fork && v.is_none() {
                    digest.push(format!(
                        "{}: ahead_by unavailable (private/deleted upstream?) — fail-open to hidden unless load-bearing",
                        repo.full_name
                    ));
                }
                if let Some(a) = &v
                    && a.raw > a.own
                {
                    digest.push(format!(
                        "{}: {} ahead commits, {} authored by you — stale fork of rewritten upstream counts as pristine, not diverged",
                        repo.full_name, a.raw, a.own
                    ));
                }
                v.map(|a| a.own)
            }
            Err(_) => None,
        };
        // Trailing-window commit velocity: 5d heat vs its own 30d
        // baseline (self-normalized, so busy repos don't auto-win).
        // Fail-open 0 with a digest note — a silent 0 would read as dead.
        let (commits_5d, _) = match client
            .commit_count_since(&repo.full_name, &repo.default_branch, Some(&five_days_ago))
            .await
        {
            Ok(v) => v,
            Err(_) => {
                digest.push(format!(
                    "{}: 5d commit velocity unavailable — heat fail-open 0",
                    repo.full_name
                ));
                (0, None)
            }
        };
        let (commits_30d, latest_30d) = match client
            .commit_count_since(
                &repo.full_name,
                &repo.default_branch,
                Some(&thirty_days_ago),
            )
            .await
        {
            Ok(v) => v,
            Err(_) => {
                digest.push(format!(
                    "{}: 30d commit velocity unavailable — baseline fail-open 0",
                    repo.full_name
                ));
                (0, None)
            }
        };
        // Latest default-branch commit: the 30d window's newest IS the
        // newest overall when non-empty. Only empty baselines need one
        // unbounded probe (dormant repos — exactly where idle matters).
        // `pushed_at` is repo-level (any branch); idle measures default.
        let last_commit_at = match latest_30d {
            Some(d) => Some(d),
            None if commits_30d == 0 => match client
                .commit_count_since(&repo.full_name, &repo.default_branch, None)
                .await
            {
                Ok((_, d)) => d,
                Err(_) => {
                    digest.push(format!(
                        "{}: latest-commit probe unavailable — idle falls back to pushed_at",
                        repo.full_name
                    ));
                    None
                }
            },
            None => None,
        };
        // Captured gauges as of collection (frozen into the JSON; live
        // paths recompute from the inputs above).
        let commit_heat = if commits_30d == 0 {
            None
        } else {
            Some(commits_5d as f32 * 6.0 / commits_30d as f32)
        };
        let focus = last_commit_at.or(repo.pushed_at).map(|t| {
            let idle = (chrono::Utc::now() - t).num_days().max(0) as f32;
            commit_heat.unwrap_or(0.0) - idle / 30.0
        });
        // Default-branch-only collection: one branch scanned, period.
        // Unmerged branch deps are proposals, not structure; a repo that
        // develops on `next` while `main` lags needs a per-repo override,
        // not a sweep. `declaring_branches` still records the single branch.
        let b_name = repo.default_branch.clone();
        let b_sha: String = client
            .branch_sha(&repo.full_name, &b_name)
            .await
            .unwrap_or_default();
        let branch_names: Vec<String> = vec![b_name.clone()];

        // Manifests/etc. from the default branch; each edge carries it.
        let mut manifest_union = collect::ManifestSet::default();
        let mut lock_union: BTreeSet<String> = BTreeSet::new();
        let mut saw_lockfile = false;
        let mut lock_versions: HashMap<String, String> = HashMap::new();
        let mut submodule_targets: BTreeSet<String> = BTreeSet::new();
        let mut workflow_targets: BTreeSet<String> = BTreeSet::new();
        let mut docs_targets: BTreeSet<String> = BTreeSet::new();
        // package -> referenced? (None = unscanned -> match)
        let mut use_sites: HashMap<String, bool> = HashMap::new();
        // Default-branch-only: edge recency is the source repo's own
        // freshness signal (`pushed_at`, free in enumerate/get). No extra
        // commit-date fetch; single branch means max() is identity.
        let branch_recency: Vec<collect::BranchDecl> = vec![collect::BranchDecl {
            branch: b_name.clone(),
            recency: repo.pushed_at,
        }];
        // Agent-parseability audit (free: derived from the tree already fetched).
        let mut has_readme = false;
        let mut has_llms_txt = false;
        let mut has_agents_md = false;
        let mut has_license_file = false;

        {
            // Tree-first: one tree call gates every existence probe.
            // Files are only fetched when present in the tree, so
            // missing-file 404s (x4 drive-retry each) drop to ~zero.
            let tree_set: Option<BTreeSet<String>> = if b_sha.is_empty() {
                None // SHA unresolved: fall back to blind probes
            } else {
                match client.tree_paths(&repo.full_name, &b_sha).await {
                    Ok(listing) => {
                        if listing.truncated {
                            digest.push(format!(
                                "{}@{}: git tree truncated — workspace members/.rs list incomplete",
                                repo.full_name, b_name
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
            // has_* surfaces: exact tree hits (case-insensitive where GitHub varies).
            if let Some(set) = &tree_set {
                has_readme = set.iter().any(|p| {
                    p.eq_ignore_ascii_case("README.md") || p.eq_ignore_ascii_case("README")
                });
                has_llms_txt = set.iter().any(|p| p == "llms.txt" || p == "llms-full.txt");
                has_agents_md = set.iter().any(|p| {
                    p.eq_ignore_ascii_case("AGENTS.md") || p.eq_ignore_ascii_case("agents.md")
                });
                has_license_file = set.iter().any(|p| {
                    p == "LICENSE"
                        || p.starts_with("LICENSE-")
                        || p.starts_with("LICENSE.")
                        || p.eq_ignore_ascii_case("licence")
                });
            }

            // Manifests, registry-driven: one loop over MANIFEST_KINDS.
            // Presence-union: the package is declared or it isn't. Walked
            // kinds (Cargo) additionally merge workspace members (below) and
            // run the use-site scan. No per-ecosystem branches.
            let mut walked_present = false;
            for kind in collect::MANIFEST_KINDS {
                if !has(kind.file) {
                    continue;
                }
                if kind.walk_members {
                    walked_present = true;
                }
                let text: Option<String> = client
                    .file_content(&repo.full_name, kind.file, &b_name)
                    .await
                    .unwrap_or_default();
                if let Some(text) = text
                    && let Ok(m) = (kind.parse)(&text)
                {
                    // Presence-union: the package is declared or it isn't.
                    for (k, v) in m.deps {
                        manifest_union.deps.entry(k).or_insert(v);
                    }
                    manifest_union.workspace_members.extend(m.workspace_members);
                    manifest_union.workspace_members.sort();
                    manifest_union.workspace_members.dedup();
                }
            }
            // Workspace members walked via git trees, collapsed to repo node.
            // Single branch scanned, so the use-site verdict is complete
            // (capped at RS_FILES_CAP with a digest note when partial).
            // Zero references nullifies (phantom dep).
            if walked_present && let Some(set) = &tree_set {
                // Fetch member manifests (cap MEMBER_MANIFEST_CAP to bound API cost).
                let member_files: Vec<(&String, &'static collect::ManifestKind)> = set
                    .iter()
                    .filter_map(|p| collect::manifest_kind_for_member(p).map(|k| (p, k)))
                    .take(MEMBER_MANIFEST_CAP)
                    .collect();
                for (mf, kind) in member_files {
                    if let Ok(Some(mt)) = client.file_content(&repo.full_name, mf, &b_name).await
                        && let Ok(mm) = (kind.parse)(&mt)
                    {
                        for (k, v) in mm.deps {
                            manifest_union.deps.entry(k).or_insert(v);
                        }
                    }
                }
                // Rust use-site check: .rs files capped. Partial coverage is
                // fail-open (keep the edge): a capped scan that finds zero
                // refs proves nothing, so unscanned verdicts stay absent
                // (build_manifest_edges treats absent as match). Only a FULL
                // scan with zero refs nullifies (phantom dep).
                {
                    let total_rs = set.iter().filter(|p| p.ends_with(".rs")).count();
                    let rs_files: Vec<_> = set
                        .iter()
                        .filter(|p| p.ends_with(".rs"))
                        .take(RS_FILES_CAP)
                        .cloned()
                        .collect();
                    let partial = total_rs > rs_files.len();
                    if partial {
                        digest.push(format!(
                                "{}@{}: .rs file list capped at {RS_FILES_CAP} of {total_rs} — use-site verdict is partial (fail-open, edges kept)",
                                repo.full_name, b_name
                            ));
                    }
                    if !rs_files.is_empty() {
                        let mut contents: Vec<String> = vec![];
                        for f in rs_files.iter() {
                            if let Ok(Some(c)) =
                                client.file_content(&repo.full_name, f, &b_name).await
                            {
                                contents.push(c);
                            }
                        }
                        for pkg in manifest_union.deps.keys().cloned().collect::<Vec<_>>() {
                            let used = contents
                                .iter()
                                .any(|src| collect::rust_crate_used(&pkg, src));
                            if used {
                                use_sites
                                    .entry(pkg.clone())
                                    .and_modify(|e| *e = *e || used)
                                    .or_insert(used);
                            } else if !partial {
                                use_sites
                                    .entry(pkg.clone())
                                    .and_modify(|e| *e = *e || used)
                                    .or_insert(used);
                                digest.push(format!(
                                        "{}@{}: phantom-nullified `{}` (zero use-sites in {RS_FILES_CAP} scanned files, full coverage)",
                                        repo.full_name, b_name, pkg
                                    ));
                            }
                            // partial + unused = fail-open: no entry (match),
                            // no phantom line — the cap note above covers it.
                        }
                    }
                }
            }
            // Lockfile gate: presence confirms, absence-with-lockfile deletes.
            // Tree-gated: only fetched when the lockfile is in the tree.
            // Versions feed Edge.version (briefing fact, never scored).
            for lock in ["Cargo.lock", "package-lock.json"] {
                if !has(lock) {
                    continue;
                }
                if let Ok(Some(text)) = client.file_content(&repo.full_name, lock, &b_name).await
                    && let Some((names, versions)) = collect::parse_lockfile_names(lock, &text)
                {
                    saw_lockfile = true;
                    lock_union.extend(names);
                    for (k, v) in versions {
                        lock_versions.entry(k).or_insert(v);
                    }
                }
            }

            // Submodules: .gitmodules -> owner/repo pins (1.0 flat).
            // Tree-gated: skipped when absent from the tree.
            if has(".gitmodules")
                && let Ok(Some(text)) = client
                    .file_content(&repo.full_name, ".gitmodules", &b_name)
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
                    .list_dir(&repo.full_name, ".github/workflows", &b_name)
                    .await
            {
                for f in files.iter().take(WORKFLOW_FILES_CAP) {
                    if let Ok(Some(text)) = client.file_content(&repo.full_name, f, &b_name).await {
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
                if let Ok(Some(text)) = client.file_content(&repo.full_name, doc, &b_name).await {
                    for link in collect::scan_docs_links(&text) {
                        if portfolio.contains(&link) && link != repo.full_name {
                            docs_targets.insert(link);
                        }
                    }
                }
            }
        }

        // Manifest edges (one per target, default branch).
        let lock_gate: Option<BTreeSet<String>> =
            if saw_lockfile { Some(lock_union) } else { None };
        let mut ambiguities: Vec<(String, Vec<String>)> = vec![];
        let mut edges = collect::build_manifest_edges(
            &repo.full_name,
            &branch_recency,
            &manifest_union,
            &aliases,
            &portfolio,
            lock_gate.as_ref(),
            &use_sites,
            &lock_versions,
            &mut ambiguities,
        );
        for (pkg, candidates) in ambiguities {
            digest.push(format!(
                "{}: ambiguous dep `{pkg}` provided by {} — no edge (not asserting a wrong one)",
                repo.full_name,
                candidates.join(", ")
            ));
        }
        // Submodule / workflow / docs associations (displayed only).
        for t in submodule_targets {
            edges.push(Edge {
                source: repo.full_name.clone(),
                target: t,
                kind: EdgeKind::Submodule,
                declaring_branches: branch_names.clone(),
                recency: None,
                version: None,
            });
        }
        for t in workflow_targets {
            edges.push(Edge {
                source: repo.full_name.clone(),
                target: t,
                kind: EdgeKind::Workflow,
                declaring_branches: branch_names.clone(),
                recency: None,
                version: None,
            });
        }
        for t in docs_targets {
            edges.push(Edge {
                source: repo.full_name.clone(),
                target: t,
                kind: EdgeKind::Docs,
                declaring_branches: branch_names.clone(),
                recency: None,
                version: None,
            });
        }
        // Registry (reverse-deps) + xrefs land in milestone 1.1 — same shape.
        let n_new = all_edges.len();
        all_edges.extend(edges);
        eprintln!(
            "    -> {} new edges ({} total), {} manifest deps",
            all_edges.len() - n_new,
            all_edges.len(),
            manifest_union.deps.len()
        );

        nodes.push(Node {
            id: repo.full_name.clone(),
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            fork: repo.fork,
            private: repo.private,
            upstream: repo.parent_full_name.clone(),
            ahead_by,
            pushed_at: repo.pushed_at,
            description: repo.description.clone(),
            topics: repo.topics.clone(),
            language: repo.language.clone(),
            homepage: repo.homepage.clone(),
            stars: repo.stars,
            forks_count: repo.forks_count,
            open_issues: repo.open_issues,
            license: repo.license.clone(),
            archived: repo.archived,
            disabled: repo.disabled,
            size_kb: repo.size_kb,
            has_readme,
            has_llms_txt,
            has_agents_md,
            has_license_file,
            commits_5d,
            commits_30d,
            commit_heat,
            focus,
            last_commit_at,
            branches: branch_names,
            visible: false, // computed below
            visible_reason: None,
            layer: graph::Layer::Isolated, // computed below via apply_layers
        });
    }

    // Pristine forks are never edge targets (`ahead_by == Some(0)` is
    // positive evidence of zero contributions). Dropped before visibility
    // so a phantom edge can't bootstrap load-bearing fan-in. `None`
    // (unknown) fails open.
    {
        let pristine_names: Vec<String> = nodes
            .iter()
            .filter(|n| n.fork && n.ahead_by == Some(0))
            .map(|n| n.id.clone())
            .collect();
        let dropped = cut_pristine_fork_edges(&nodes, &mut all_edges);
        if dropped > 0 {
            digest.push(format!(
                "pristine-fork cut: dropped {dropped} edge(s) to {} (ahead_by=0, not own work)",
                pristine_names.join(", ")
            ));
        }
    }
    graph::apply_visibility(&mut nodes, &all_edges);
    graph::apply_layers(&mut nodes, &all_edges);
    let g = Graph::new(nodes, all_edges);
    eprintln!(
        "done: {} repos, {} nodes, {} edges",
        repos.len(),
        g.nodes.len(),
        g.edges.len()
    );

    if !digest.is_empty() {
        eprintln!("-- briefing digest ({} notes) --", digest.len());
        for line in digest.iter().take(50) {
            eprintln!("  {line}");
        }
    }
    Ok(g)
}

/// Drop edges targeting pristine forks (`ahead_by == Some(0)`).
/// Positive evidence of zero contributions: not own work, never a
/// proximity. Returns the number dropped. `None` (unknown) fails open.
pub fn cut_pristine_fork_edges(nodes: &[Node], all_edges: &mut Vec<Edge>) -> usize {
    use std::collections::BTreeSet;
    let pristine: BTreeSet<&str> = nodes
        .iter()
        .filter(|n| n.fork && n.ahead_by == Some(0))
        .map(|n| n.id.as_str())
        .collect();
    if pristine.is_empty() {
        return 0;
    }
    let before = all_edges.len();
    all_edges.retain(|e| !pristine.contains(e.target.as_str()));
    before - all_edges.len()
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
                pushed_at: None,
                description: None,
                topics: vec![],
                language: None,
                homepage: None,
                stars: 0,
                forks_count: 0,
                open_issues: 0,
                license: None,
                archived: false,
                disabled: false,
                size_kb: 0,
                has_readme: false,
                has_llms_txt: false,
                has_agents_md: false,
                has_license_file: false,
                commits_5d: 0,
                commits_30d: 0,
                commit_heat: None,
                focus: None,
                last_commit_at: None,
                branches: vec!["main".into()],
                visible: true,
                visible_reason: None,
                layer: graph::Layer::Isolated,
            },
            Node {
                id: "me/secret".into(),
                owner: "me".into(),
                name: "secret".into(),
                fork: false,
                private: true,
                upstream: None,
                ahead_by: None,
                pushed_at: None,
                description: None,
                topics: vec![],
                language: None,
                homepage: None,
                stars: 0,
                forks_count: 0,
                open_issues: 0,
                license: None,
                archived: false,
                disabled: false,
                size_kb: 0,
                has_readme: false,
                has_llms_txt: false,
                has_agents_md: false,
                has_license_file: false,
                commits_5d: 0,
                commits_30d: 0,
                commit_heat: None,
                focus: None,
                last_commit_at: None,
                branches: vec!["main".into()],
                visible: false,
                visible_reason: None,
                layer: graph::Layer::Isolated,
            },
        ];
        let edges = vec![Edge {
            source: "me/pub".into(),
            target: "me/secret".into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into()],
            recency: None,
            version: None,
        }];
        graph::apply_visibility(&mut nodes, &edges);
        graph::apply_layers(&mut nodes, &edges);
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

    #[test]
    fn graph_json_roundtrips_across_crate_boundary() {
        // Simulates downstream crate use: serialize here, parse back via
        // the library entry point (schema-checked).
        let g = test_graph_with_private();
        let text = crate::output::render_json(&g).unwrap();
        let back = Graph::from_json(&text).unwrap();
        assert_eq!(back.schema_version, crate::graph::SCHEMA_VERSION);
        assert_eq!(back.nodes.len(), 2);
    }

    fn cut_node(id: &str, fork: bool, ahead_by: Option<u32>) -> Node {
        Node {
            id: id.into(),
            owner: "me".into(),
            name: id.rsplit('/').next().unwrap_or(id).into(),
            fork,
            private: false,
            upstream: None,
            ahead_by,
            pushed_at: None,
            description: None,
            topics: vec![],
            language: None,
            homepage: None,
            stars: 0,
            forks_count: 0,
            open_issues: 0,
            license: None,
            archived: false,
            disabled: false,
            size_kb: 0,
            has_readme: false,
            has_llms_txt: false,
            has_agents_md: false,
            has_license_file: false,
            commits_5d: 0,
            commits_30d: 0,
            commit_heat: None,
            focus: None,
            last_commit_at: None,
            branches: vec!["main".into()],
            visible: false,
            visible_reason: None,
            layer: graph::Layer::Isolated,
        }
    }

    fn cut_edge(source: &str, target: &str) -> Edge {
        Edge {
            source: source.into(),
            target: target.into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into()],
            recency: None,
            version: None,
        }
    }

    #[test]
    fn pristine_forks_are_never_edge_targets() {
        // The ratatui case: pristine fork (ahead_by 0) must not anchor.
        let nodes = vec![
            cut_node("me/app", false, None),
            cut_node("me/pristine-fork", true, Some(0)),
            cut_node("me/diverged-fork", true, Some(3)),
            cut_node("me/unknown-fork", true, None),
        ];
        let mut edges = vec![
            cut_edge("me/app", "me/pristine-fork"),
            cut_edge("me/app", "me/diverged-fork"),
            cut_edge("me/app", "me/unknown-fork"),
        ];
        let dropped = cut_pristine_fork_edges(&nodes, &mut edges);
        assert_eq!(dropped, 1);
        assert!(edges.iter().all(|e| e.target != "me/pristine-fork"));
        assert!(edges.iter().any(|e| e.target == "me/diverged-fork"));
        assert!(edges.iter().any(|e| e.target == "me/unknown-fork"));
    }
}
