//! End-to-end: fixture JSON in, SVG bytes out, snapshot-pinned.
//! This runs the real embedded OxDock script (LOAD_JSON, FOR, APPEND,
//! ASSERTs) — DSL regressions surface here, not in production.

use jzombie_graph::graph::{Graph, Layer, Node};

fn node(name: &str, commits_5d: u32) -> Node {
    Node {
        id: format!("me/{name}"),
        owner: "me".into(),
        name: name.into(),
        fork: false,
        private: false,
        upstream: None,
        ahead_by: None,
        pushed_at: None,
        description: None,
        topics: vec![],
        language: Some("Rust".into()),
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
        commits_5d,
        commits_30d: commits_5d * 6,
        commit_heat: None,
        focus: None,
        last_commit_at: None,
        branches: vec!["main".into()],
        visible: true,
        visible_reason: None,
        layer: Layer::Isolated,
    }
}

fn json_of(commits: &[(&str, u32)]) -> String {
    let nodes = commits.iter().map(|(n, c)| node(n, *c)).collect();
    jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap()
}

#[test]
fn share_pie_top_n_plus_other_snapshot() {
    let g = json_of(&[
        ("alpha", 10),
        ("beta", 7),
        ("gamma", 3),
        ("delta", 2),
        ("epsilon", 2),
        ("zeta", 1),
        ("eta", 1),
        ("theta", 1),
        ("iota", 1),
        ("kappa", 1),
        ("silent", 0),
    ]);
    let svg = charts::render_share_svg(&g).unwrap();
    // Structural guarantees alongside the snapshot pin.
    assert!(svg.starts_with("<svg "));
    assert!(svg.ends_with("</svg>"));
    assert_eq!(svg.matches("<path ").count(), 9); // top 8 + Other
    assert!(svg.contains("Other"));
    assert!(!svg.contains("silent"));
    insta::assert_snapshot!("share_pie", svg);
}

#[test]
fn share_pie_single_contributor_is_circle() {
    let g = json_of(&[("solo", 5)]);
    let svg = charts::render_share_svg(&g).unwrap();
    assert_eq!(svg.matches("<circle ").count(), 1);
    assert_eq!(svg.matches("<path ").count(), 0);
    insta::assert_snapshot!("share_pie_single", svg);
}

#[test]
fn share_pie_empty_window_is_bare_canvas() {
    let g = json_of(&[("quiet", 0)]);
    let svg = charts::render_share_svg(&g).unwrap();
    assert_eq!(svg.matches("<path ").count(), 0);
    insta::assert_snapshot!("share_pie_empty", svg);
}
