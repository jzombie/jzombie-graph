//! End-to-end: fixture JSON in, flame SVG out, snapshot-pinned.

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

fn node(name: &str, c1: u32, c5: u32, c15: u32, c30: u32, branches: Vec<BranchHeat>) -> Node {
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
        commits_1d: c1,
        commits_5d: c5,
        commits_15d: c15,
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
        focus: None,
        last_commit_at: None,
        branch_heat: branches,
        recent_prs: vec![],
        visible: true,
        visible_reason: None,
        layer: std::collections::BTreeSet::from([Layer::Isolated]),
        has_default_binary: false,
    }
}

#[test]
fn flame_pro_rata_snapshot() {
    let mut big = node(
        "big",
        6,
        55,
        64,
        81,
        vec![branch("main", true, 0, 1, 10, 27)],
    );
    big.description = Some("Heat sketch for the portfolio map".into());
    big.topics = vec!["viz".into(), "svg".into()];
    big.language = Some("Rust".into());
    let nodes = vec![
        big,
        node("small", 0, 0, 0, 3, vec![branch("main", true, 0, 0, 0, 3)]),
        node("quiet", 0, 0, 0, 0, vec![]),
    ];
    let g = jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap();
    let svg = charts::render_flame_svg(&g).unwrap();
    assert!(svg.starts_with("<svg "));
    assert!(svg.ends_with("</svg>"));
    assert!(svg.contains("portfolio · 84 commits / 30d"));
    assert!(svg.contains("top: main (27)"));
    assert!(svg.contains("Heat sketch for the portfolio map"));
    assert!(svg.contains("#viz #svg"));
    assert!(svg.contains("Rust"));
    assert!(!svg.contains("quiet"));
    // One card per repo, heating pulse on the hot one.
    assert_eq!(svg.matches("class=\"card pulse\"").count(), 1);
    assert_eq!(svg.matches("class=\"card pulse slow\"").count(), 0);
    assert_eq!(svg.matches("<linearGradient").count(), 2);
    insta::assert_snapshot!("flame", svg);
}

#[test]
fn flame_empty_when_no_commits() {
    let nodes = vec![node("quiet", 0, 0, 0, 0, vec![])];
    let g = jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap();
    let svg = charts::render_flame_svg(&g).unwrap();
    assert!(svg.contains("No commits in trailing 30d"));
    insta::assert_snapshot!("flame_empty", svg);
}
