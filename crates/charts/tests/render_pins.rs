//! End-to-end: fixture JSON in, pins SVG out, snapshot-pinned.

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

#[test]
fn pins_top6_by_focus_snapshot() {
    let descs = [
        "Async Rust client for SEC EDGAR: fetch filings, fund holdings, XBRL data, and IPO registrations with compliant rate limiting",
        "Nightly map of my GitHub portfolio",
        "Composable data sampling primitives",
        "Spatial Terminal Desktop Environment for Remote Workspaces",
        "Managed caches, with optional eviction, for Rust project artifacts",
        "Zero-copy access, schema-less, append-only, SIMD-optimized binary storage engine for Rust",
        "Seventh repo that must not render",
    ];
    let langs = ["Rust", "Rust", "Rust", "Rust", "Rust", "Rust", "Go"];
    let mut nodes = vec![];
    for (i, name) in ["r0", "r1", "r2", "r3", "r4", "r5", "r6"].iter().enumerate() {
        nodes.push(node(
            name,
            Some(descs[i]),
            Some(langs[i]),
            (i as u32 + 1) * 3,
            i as u32,
            Some(6.0 - i as f32),
            (i as u32 + 1) * 5,
        ));
    }
    let g = jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap();
    let svg = charts::render_pins_svg(&g).unwrap();
    assert!(svg.starts_with("<svg "));
    assert!(svg.ends_with("</svg>"));
    // Focus order, hottest first; the seventh repo stays out.
    let top = svg.find("r0").unwrap();
    let second = svg.find("r1").unwrap();
    assert!(top < second, "focus desc");
    assert!(!svg.contains("r6"), "top-6 cut");
    assert!(svg.contains("#dea584"), "Rust dot");
    assert!(svg.contains("https://github.com/me/r0"), "cards link out");
    // Six bordered cards in a 2-col grid.
    assert_eq!(svg.matches("rx=\"4.5\"").count(), 6);
    insta::assert_snapshot!("pins", svg);
}

#[test]
fn pins_empty_when_no_commits() {
    let nodes = vec![node("quiet", None, None, 0, 0, None, 0)];
    let g = jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap();
    let svg = charts::render_pins_svg(&g).unwrap();
    assert!(svg.contains("No commits in trailing 30d"));
    insta::assert_snapshot!("pins_empty", svg);
}
