//! End-to-end: fixture JSON in, pins family out, snapshots pinned.
//!
//! The family renders top-9 by stars (forks, then name, break ties);
//! focus scores never matter. Cards carry a heat band plus a proof
//! footer; the fragment flows as one centered paragraph.

use jzombie_graph::graph::{BranchHeat, Graph, Layer, Node};

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

fn eleven() -> Vec<Node> {
    let descs = [
        "Async Rust client for SEC EDGAR: fetch filings, fund holdings, XBRL data, and IPO registrations with compliant rate limiting",
        "Nightly map of my GitHub portfolio",
        "Composable data sampling primitives",
        "Spatial Terminal Desktop Environment for Remote Workspaces",
        "Managed caches, with optional eviction, for Rust project artifacts",
        "Zero-copy access, schema-less, append-only, SIMD-optimized binary storage engine for Rust",
        "Seventh repo that must not render",
        "Eighth repo that makes the cut",
        "Ninth repo squeaking in",
        "Tenth repo just outside",
        "Eleventh repo, also out",
    ];
    // Stars drive the order; r1/r2 tie breaks on forks. Focus scores
    // run the other way to prove they no longer matter.
    let stats = [
        (30u32, 0u32, 0.0f32),
        (25, 9, 1.0),
        (25, 3, 2.0),
        (20, 1, 3.0),
        (15, 4, 4.0),
        (10, 2, 5.0),
        (1, 0, 99.0),
        (9, 0, 6.0),
        (8, 1, 7.0),
        (7, 0, 8.0),
        (2, 0, 9.0),
    ];
    // r5 has no language: the footer must skip the dot, not print
    // "Unspecified".
    let langs = [
        Some("Rust"),
        Some("Rust"),
        Some("Rust"),
        Some("Rust"),
        Some("Rust"),
        None,
        Some("Go"),
        Some("Go"),
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
            (i as u32 + 1) * 5,
        ));
    }
    nodes
}

#[test]
fn pins_bundle_one_card_per_repo_each_linking_out() {
    let mut graph = Graph::new(eleven(), vec![]);
    // Pin the sweep timestamp: asset URLs carry it as the cache-bust
    // token, so the fragment snapshot stays deterministic.
    graph.generated_at = "2026-11-14T23:03:00Z".parse().expect("fixed time");
    let g = jzombie_graph::output::render_json(&graph).unwrap();
    let bundle = charts::render_pins_bundle(&g).unwrap();
    // Nine cards plus the fragment that embeds them.
    assert_eq!(bundle.len(), 10);
    let paths: Vec<&str> = bundle.iter().map(|(p, _)| p.as_str()).collect();
    assert!(paths.contains(&"proof/dashboard/charts/portfolio-graph-pin-r0.svg"));
    assert!(paths.contains(&"proof/fragments/pins.md"));
    // r10 (2 stars) and r6 (1 star) fall outside the top-9 cut.
    assert!(!paths.iter().any(|p| p.contains("r10")), "cut tail");
    assert!(!paths.iter().any(|p| p.contains("pin-r6")), "cut tail");
    // Stars desc, forks break the r1/r2 tie, focus ignored.
    let first = &bundle[0].1;
    assert!(
        first.contains("https://github.com/me/r0"),
        "top card links out"
    );
    assert!(first.contains("fill=\"#000000\""), "black card");
    assert!(first.contains("#dea584"), "Rust dot");
    // r5 has no language: no language dot — but the heat dot still
    // renders, so exactly one circle, in the dormant band color.
    let r5 = &bundle[5].1;
    assert!(!r5.contains("Unspecified"), "no language, no label");
    assert_eq!(r5.matches("<circle").count(), 1, "heat dot only");
    assert!(r5.contains("#BBBBBB"), "dormant heat dot");
    // The fragment flows as one centered paragraph, rank order.
    let frag = &bundle[9].1;
    assert!(frag.contains("## Popular repositories"));
    assert!(frag.contains("<div align=\"center\">"));
    let r0 = frag.find("me/r0").unwrap();
    let r1 = frag.find("me/r1").unwrap();
    let r2 = frag.find("me/r2").unwrap();
    assert!(r0 < r1 && r1 < r2, "fragment rank order");
    assert!(!frag.contains("r10"), "tail stays out of the fragment");
    assert!(!frag.contains("me/r6\""), "tail stays out of the fragment");
    insta::assert_snapshot!("pins_card", first);
    insta::assert_snapshot!("pins_frag", frag);
}

#[test]
fn pins_empty_when_no_commits() {
    let nodes = vec![node("quiet", None, None, 0, 0, None, 0)];
    let g = Graph::new(nodes, vec![]);
    let json = jzombie_graph::output::render_json(&g).unwrap();
    let bundle = charts::render_pins_bundle(&json).unwrap();
    assert_eq!(bundle.len(), 1);
    assert_eq!(bundle[0].0, "proof/fragments/pins.md");
    assert!(bundle[0].1.contains("No commits in trailing 30d"));
    insta::assert_snapshot!("pins_empty", bundle[0].1);
}
