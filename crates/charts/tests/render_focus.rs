//! End-to-end: fixture JSON in, focus-bars SVG out, snapshot-pinned.
//! Snapshots lock only on approval — see the pie chart flow.

use jzombie_graph::graph::{Graph, Layer, Node};

fn node(name: &str, focus: Option<f32>) -> Node {
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
        commits_5d: 0,
        commits_30d: 0,
        commit_heat: None,
        focus,
        last_commit_at: None,
        branches: vec!["main".into()],
        visible: true,
        visible_reason: None,
        layer: Layer::Isolated,
    }
}

fn json_of(scores: &[(&str, Option<f32>)]) -> String {
    let nodes = scores.iter().map(|(n, f)| node(n, *f)).collect();
    jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap()
}

#[test]
fn focus_bars_diverging_snapshot() {
    let g = json_of(&[
        ("argtuner", Some(3.00)),
        ("oxdock", Some(0.43)),
        ("windows-headless", Some(0.00)),
        ("term-wm-vt100", Some(-0.10)),
        ("lab-qemu-portable", Some(-0.20)),
        ("ancient", Some(-54.83)),
        ("mystery", None),
    ]);
    let svg = charts::render_focus_svg(&g).unwrap();
    assert!(svg.starts_with("<svg "));
    assert!(svg.ends_with("</svg>"));
    // Six scored rows render; the unknown one is excluded from the scale.
    // Each row draws one bar rect plus one clip rect (two-pass ink).
    assert_eq!(svg.matches("<clipPath ").count(), 6);
    assert_eq!(svg.matches("<rect ").count(), 12);
    insta::assert_snapshot!("focus_bars", svg);
}

#[test]
fn focus_bars_empty_when_unknown() {
    let g = json_of(&[("mystery", None)]);
    let svg = charts::render_focus_svg(&g).unwrap();
    assert!(svg.contains("No focus data"));
    insta::assert_snapshot!("focus_bars_empty", svg);
}
