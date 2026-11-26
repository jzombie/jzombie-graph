//! End-to-end: fixture JSON in, languages strip out. Assert-based
//! (no snapshots): counts, ordering, exclusions, bust token.

use jzombie_graph::graph::{Graph, Layer, Node};

#[allow(clippy::too_many_arguments)]
fn node(name: &str, lang: Option<&str>, fork: bool, private: bool) -> Node {
    Node {
        id: format!("me/{name}"),
        owner: "me".into(),
        name: name.into(),
        fork,
        private,
        upstream: None,
        ahead_by: None,
        pushed_at: None,
        description: Some(format!("about {name}")),
        topics: vec![],
        language: lang.map(|l| l.into()),
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
        commits_1d: 0,
        commits_5d: 0,
        commits_15d: 0,
        commits_30d: 5,
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
        focus: None,
        last_commit_at: None,
        branch_heat: vec![],
        recent_prs: vec![],
        visible: true,
        visible_reason: None,
        layer: std::collections::BTreeSet::from([Layer::Isolated]),
        has_default_binary: false,
        ci_usage_30d: Default::default(),
    }
}

fn fixture() -> String {
    let mut nodes = vec![];
    for (i, name) in ["r1", "r2", "r3", "r4"].iter().enumerate() {
        let mut n = node(name, Some("Rust"), false, false);
        n.description = Some(format!("repo {i}"));
        nodes.push(n);
    }
    for name in ["s1", "s2"] {
        nodes.push(node(name, Some("Shell"), false, false));
    }
    // Python/Shell tie breaks by name: Python first.
    for name in ["p1", "p2"] {
        nodes.push(node(name, Some("Python"), false, false));
    }
    for (name, lang) in [
        ("d1", "Dockerfile"),
        ("g1", "Go"),
        ("j1", "JavaScript"),
        ("l1", "Lua"),
        ("t1", "TypeScript"),
    ] {
        nodes.push(node(name, Some(lang), false, false));
    }
    // Excluded: unknown language, private, plain fork.
    nodes.push(node("u1", None, false, false));
    nodes.push(node("priv", Some("Rust"), false, true));
    nodes.push(node("fork", Some("Rust"), true, false));
    let mut g = Graph::new(nodes, vec![]);
    // Pin the sweep timestamp: asset URLs carry it as the cache-bust token.
    g.generated_at = "2026-11-14T23:03:00Z".parse().expect("fixed time");
    jzombie_graph::output::render_json(&g).unwrap()
}

#[test]
fn languages_rank_counts_and_roll_up_other() {
    let svg = charts::render_languages_svg(&fixture()).unwrap();
    assert!(svg.starts_with("<svg "));
    assert!(svg.ends_with("</svg>"));
    // 13 in scope: top-6 plus an Other pair (Lua + TypeScript).
    for entry in [
        "Rust 30.8%",
        "Python 15.4%",
        "Shell 15.4%",
        "Dockerfile 7.7%",
        "Go 7.7%",
        "JavaScript 7.7%",
        "Other 15.4%",
    ] {
        assert!(svg.contains(entry), "legend carries {entry}");
    }
    // Rank order: counts desc, ties break by name (Python < Shell).
    let pos = |s: &str| svg.find(s).unwrap();
    assert!(pos("Rust") < pos("Python"));
    assert!(pos("Python") < pos("Shell"));
    assert!(pos("Shell") < pos("Dockerfile"));
    assert!(pos("Other") > pos("JavaScript"));
    // No Unknown slot, no private/fork leakage into the bar.
    assert!(!svg.contains("Unknown"), "unknown languages stay out");
    assert!(!svg.contains("priv"), "private repos stay out");
    assert!(!svg.contains("fork"), "plain forks stay out");
    assert!(
        !svg.contains("?v="),
        "bust lives in the fragment, not the svg"
    );
}

#[test]
fn languages_fragment_links_and_busts() {
    let md = charts::render_languages_md(&fixture()).unwrap();
    assert!(md.contains("## Top languages"));
    assert!(
        md.contains("charts/portfolio-graph-languages.svg?v=202611142303"),
        "bust token from sweep:\n{md}"
    );
    assert!(md.contains("https://github.com/jzombie?tab=repositories"));
}

#[test]
fn languages_empty_when_no_known_languages() {
    let nodes = vec![node("quiet", None, false, false)];
    let g = Graph::new(nodes, vec![]);
    let json = jzombie_graph::output::render_json(&g).unwrap();
    let svg = charts::render_languages_svg(&json).unwrap();
    assert!(svg.contains("No language data in scope"));
}
