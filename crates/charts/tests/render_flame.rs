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
        commit_heat: None,
        focus: None,
        last_commit_at: None,
        branch_heat: branches,
        visible: true,
        visible_reason: None,
        layer: Layer::Isolated,
    }
}

#[test]
fn flame_pro_rata_snapshot() {
    let nodes = vec![
        node(
            "big",
            6,
            55,
            64,
            81,
            vec![branch("main", true, 0, 1, 10, 27)],
        ),
        node("small", 0, 0, 0, 3, vec![branch("main", true, 0, 0, 0, 3)]),
        node("quiet", 0, 0, 0, 0, vec![]),
    ];
    let g = jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap();
    let svg = charts::render_flame_svg(&g).unwrap();
    assert!(svg.starts_with("<svg "));
    assert!(svg.ends_with("</svg>"));
    assert!(svg.contains("portfolio · 84 commits / 30d"));
    assert!(svg.contains("recent embers"));
    assert!(!svg.contains("quiet"));
    assert_eq!(svg.matches("class=\"bar pulse\"").count(), 1);
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
