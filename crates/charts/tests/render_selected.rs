//! End-to-end: fixture JSON in, selected-work fragment out, snapshot pinned.
//!
//! The list ranks by `selected_score`, not by stars: a repo the portfolio
//! depends on leads even with none of its own, a popular repo nobody
//! imports sits below it, and focus scores (which have their own chart)
//! change nothing. The profile README repo is never selected work.

use jzombie_graph::graph::{BranchHeat, Edge, EdgeKind, Graph, Layer, Node, RecentIssue, RecentPr};

fn branch(name: &str, default: bool, c1: u32, c5: u32, c15: u32, c30: u32) -> BranchHeat {
    BranchHeat {
        branch: name.into(),
        is_default: default,
        commits_1d: c1,
        commits_5d: c5,
        commits_15d: c15,
        commits_30d: c30,
    }
}

#[allow(clippy::too_many_arguments)]
fn node(
    name: &str,
    desc: Option<&str>,
    lang: Option<&str>,
    stars: u32,
    forks: u32,
    focus: Option<f32>,
    c30: u32,
) -> Node {
    Node {
        id: format!("me/{name}"),
        owner: "me".into(),
        name: name.into(),
        fork: false,
        private: false,
        upstream: None,
        ahead_by: None,
        pushed_at: None,
        description: desc.map(|d| d.into()),
        topics: vec![],
        language: lang.map(|l| l.into()),
        homepage: None,
        stars,
        forks_count: forks,
        open_issues: 0,
        license: None,
        archived: false,
        disabled: false,
        size_kb: 0,
        has_readme: false,
        has_llms_txt: false,
        has_agents_md: false,
        has_license_file: false,
        commits_1d: 0,
        commits_5d: 0,
        commits_15d: 0,
        commits_30d: c30,
        issues_open_1d: 0,
        issues_open_5d: 0,
        issues_open_15d: 0,
        issues_open_30d: 0,
        issues_closed_1d: 0,
        issues_closed_5d: 0,
        issues_closed_15d: 0,
        issues_closed_30d: 0,
        issue_heat: None,
        issue_comments_1d: 0,
        issue_comments_5d: 0,
        issue_comments_15d: 0,
        issue_comments_30d: 0,
        issue_labels: Default::default(),
        recent_issues: vec![],
        commit_heat: None,
        focus,
        last_commit_at: None,
        branch_heat: vec![branch("main", true, 0, 0, 0, c30)],
        recent_prs: vec![],
        visible: true,
        visible_reason: None,
        layer: std::collections::BTreeSet::from([Layer::Isolated]),
        has_default_binary: false,
    }
}

/// Eleven repos, identical 30d activity so only reuse, pull, and forks can
/// move the order. `r0` has no stars at all but three portfolio dependents;
/// `r1` is the most-starred repo and still lands third, behind `r2` on
/// forks. Focus runs backwards from the result to prove it is ignored.
fn eleven() -> Vec<Node> {
    let descs = [
        "Depended-on core that three other repos import",
        "Most popular repo nobody here imports",
        "Popular and forked",
        "Steady earner",
        "Forked more than it stars",
        "Mid-pack",
        "Quiet follower",
        "Small and depended upon",
        "Barely there",
        "Tail one",
        "Tail two",
    ];
    let stats = [
        (0u32, 0u32, -99.0f32),
        (30, 0, 99.0),
        (25, 3, 50.0),
        (20, 1, 5.0),
        (15, 4, 4.0),
        (10, 2, 3.0),
        (8, 1, 2.0),
        (9, 0, 1.0),
        (7, 0, 0.0),
        (2, 0, -5.0),
        (1, 0, -9.0),
    ];
    let langs = [
        Some("Rust"),
        Some("Rust"),
        Some("Rust"),
        Some("Rust"),
        Some("Rust"),
        None,
        Some("Go"),
        Some("Rust"),
        Some("Python"),
        Some("Rust"),
        Some("Rust"),
    ];
    let mut nodes = vec![];
    for (i, name) in [
        "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10",
    ]
    .iter()
    .enumerate()
    {
        let (stars, forks, focus) = stats[i];
        nodes.push(node(
            name,
            Some(descs[i]),
            langs[i],
            stars,
            forks,
            Some(focus),
            5,
        ));
    }
    // Identity data rides the graph too: `r0` carries topics plus a
    // homepage, `r1` topics only, `r2` neither — so every branch of the
    // render (topic line, line dropped) is covered.
    nodes[0].topics = vec!["core".into(), "zero-copy".into()];
    nodes[0].homepage = Some("https://crates.io/crates/r0".into());
    nodes[1].topics = vec!["popular".into()];
    // `r0` gets the full fact set so every line the renderer can emit
    // is covered: classification, thread traffic, samples.
    let n0 = &mut nodes[0];
    n0.layer = std::collections::BTreeSet::from([Layer::Middleware, Layer::Application]);
    n0.license = Some("Apache-2.0".into());
    n0.size_kb = 6334;
    n0.open_issues = 69;
    n0.has_readme = true;
    n0.has_license_file = true;
    n0.has_agents_md = true;
    n0.last_commit_at = Some("2026-11-15T01:34:48Z".parse().expect("fixed time"));
    n0.branch_heat.push(branch("feat/ui", false, 1, 2, 3, 4));
    n0.issues_open_30d = 6;
    n0.issues_closed_30d = 5;
    n0.issue_comments_30d = 9;
    n0.issue_heat = Some(1.4);
    n0.issue_labels = std::collections::BTreeMap::from([
        ("bug".to_string(), 4u32),
        ("enhancement".to_string(), 5u32),
    ]);
    n0.recent_prs = vec![
        RecentPr {
            title: "fix: Cut UI lag from mouse bursts".into(),
            branch: "fix/ui".into(),
            base: "main".into(),
            state: "MERGED".into(),
        },
        RecentPr {
            title: "Bump deps".into(),
            branch: "deps".into(),
            base: "main".into(),
            state: "OPEN".into(),
        },
        RecentPr {
            title: "draft - external script imports".into(),
            branch: "draft".into(),
            base: "main".into(),
            state: "CLOSED".into(),
        },
    ];
    n0.recent_issues = vec![
        RecentIssue {
            number: 131,
            title: "100-line app ergonomics".into(),
            state: "OPEN".into(),
            labels: vec!["enhancement".into()],
            comments: 1,
        },
        RecentIssue {
            number: 87,
            title: "MCP server for agents".into(),
            state: "OPEN".into(),
            labels: vec!["enhancement".into(), "needs research".into()],
            comments: 9,
        },
    ];
    // A diverged fork: own work, and the row says so. `r6`, not a row
    // the ordering assertions pin.
    nodes[6].fork = true;
    nodes[6].upstream = Some("other/Quiet".into());
    nodes[6].ahead_by = Some(12);
    nodes
}

/// Dependence edges into `r0` (three importers) and into `r7` (one).
fn edges() -> Vec<Edge> {
    let edge = |s: &str, t: &str| Edge {
        source: format!("me/{s}"),
        target: format!("me/{t}"),
        kind: EdgeKind::Manifest,
        declaring_branches: vec!["main".into()],
        recency: None,
        version: None,
    };
    vec![
        edge("r1", "r0"),
        edge("r2", "r0"),
        edge("r6", "r0"),
        edge("r5", "r7"),
    ]
}

#[test]
fn selected_fragment_ranks_by_score_not_stars() {
    let mut graph = Graph::new(eleven(), edges());
    // Pin the sweep timestamp so the snapshot stays deterministic even
    // though this fragment carries no cache-bust token.
    graph.generated_at = "2026-11-14T23:03:00Z".parse().expect("fixed time");
    let g = jzombie_graph::output::render_json(&graph).unwrap();
    let md = charts::render_selected_md(&g).unwrap();

    assert!(md.starts_with("## Selected work"), "heading first");
    assert!(
        md.contains("ranking of signal, not a tally of volume"),
        "the prose states the measure"
    );
    // `r0` leads on three dependents despite zero stars; `r2` outranks
    // the more-starred `r1` on forks; focus never enters the order.
    let pos = |needle: &str| {
        md.find(needle)
            .unwrap_or_else(|| panic!("missing {needle}"))
    };
    let r0 = pos("me/r0");
    let r2 = pos("me/r2");
    let r1 = pos("me/r1");
    assert!(r0 < r2 && r2 < r1, "score order, not star order");
    // Top-9 cut: the tail never renders.
    assert!(!md.contains("me/r9"), "cut tail");
    assert!(!md.contains("me/r10"), "cut tail");
    // Dependents ride as text, zeros included — this is the "show me
    // everything" pass, so nothing is suppressed for looking empty.
    assert!(md.contains("used by 3"), "fan-in spelled out");
    assert!(md.contains("used by 0"), "zeros still print");
    assert!(md.contains("uses 1"), "fan-out spelled out");
    // The heat badge is the committed indicator file, sized and centered
    // on the text — `align` is the only vertical alignment GitHub keeps.
    assert!(
        md.contains("dashboard/indicators/"),
        "badge rides the shared indicator family"
    );
    assert!(
        md.contains("width=\"24\" align=\"middle\""),
        "badge sized and centered:\n{md}"
    );
    // Classification line: positional tags, identity, tree audit.
    assert!(
        md.contains("layer: `middleware · application`"),
        "layer spelled out"
    );
    assert!(md.contains("focus -99.00"), "focus gauge");
    assert!(md.contains("last commit 2026-11-15"), "recency date");
    assert!(md.contains("2 branches"), "branch count");
    assert!(md.contains("6.2 MB"), "size in reader units");
    assert!(
        md.contains("docs: readme, license, agents.md"),
        "tree audit"
    );
    assert!(
        md.contains("fork of other/Quiet (+12 own commits)"),
        "diverged fork says so"
    );
    // Proof line: backlog and structure, then the homepage printed in
    // full — never a truncated host.
    assert!(md.contains("69 open issues"), "backlog");
    assert!(
        md.contains("[https://crates.io/crates/r0](https://crates.io/crates/r0)"),
        "homepage printed in full, linked"
    );
    // Thread traffic: window counts, gauge, label census, then the
    // threads themselves as state counts plus a sample.
    assert!(
        md.contains("issues 30d: 6 opened · 5 closed · 9 comments"),
        "issue window"
    );
    assert!(md.contains("issue heat 1.40"), "issue gauge");
    assert!(
        md.contains("label census: enhancement×5, bug×4"),
        "loudest label first"
    );
    assert!(
        md.contains("PRs (3): 1 merged · 1 open · 1 closed"),
        "PR states"
    );
    assert!(
        md.contains("\"fix: Cut UI lag from mouse bursts\" · \"Bump deps\""),
        "PR sample"
    );
    assert!(md.contains("issues (2): 2 open · 0 closed"), "issue states");
    assert!(
        md.contains("#131 \"100-line app ergonomics\" (open · 1 comment · enhancement)"),
        "issue sample"
    );
    // Topics render as user-scoped search links.
    assert!(
        md.contains(
            "[#core](https://github.com/search?q=user%3Ame+topic%3Acore&type=repositories)"
        ),
        "topic linked to a user-scoped search"
    );
    // `r2` carries neither: no topic line, no homepage in its proof.
    let r2_item = &md[pos("me/r2")..pos("me/r1")];
    assert!(!r2_item.contains("topic%3A"), "no topics, no topic line");
    assert!(!r2_item.contains("crates.io"), "no homepage, no host");
    insta::assert_snapshot!("selected_frag", md);
}

#[test]
fn selected_fragment_skips_the_profile_readme_repo() {
    let mut nodes = eleven();
    // A profile README (`owner` == `name`) with the loudest stats in the
    // graph: documentation index, never selected work.
    let mut profile = node(
        "profile",
        Some("Portfolio index"),
        None,
        100,
        100,
        Some(9.0),
        100,
    );
    profile.id = "sel/sel".into();
    profile.owner = "sel".into();
    profile.name = "sel".into();
    nodes.push(profile);
    let g = jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap();
    let md = charts::render_selected_md(&g).unwrap();
    assert!(!md.contains("sel/sel"), "profile repo excluded");
    assert!(!md.contains("Portfolio index"), "its description stays out");
}

#[test]
fn selected_empty_when_no_commits() {
    let nodes = vec![node("quiet", None, None, 0, 0, None, 0)];
    let g = jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap();
    let md = charts::render_selected_md(&g).unwrap();
    assert!(md.contains("No commits in trailing 30d"));
    insta::assert_snapshot!("selected_empty", md);
}
