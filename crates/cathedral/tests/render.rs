//! Fixture tests: the whole core renders as one clickable arch —
//! crown, courses, convergence, bedrock banner. No lists, no tables,
//! no fences-as-text, no unwired names.

use cathedral::render_cathedral;
use jzombie_graph::graph::{self, EdgeKind, Graph, Layer, Node};

fn node(id: &str, fork: bool, private: bool) -> Node {
    let (owner, name) = id.split_once('/').unwrap();
    Node {
        id: id.into(),
        owner: owner.into(),
        name: name.into(),
        fork,
        private,
        upstream: None,
        ahead_by: None,
        pushed_at: None,
        description: Some(format!("about {name}")),
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
        branches: vec!["main".into()],
        visible: true,
        visible_reason: None,
        layer: Layer::Isolated,
    }
}

fn edge(source: &str, target: &str, kind: EdgeKind) -> jzombie_graph::graph::Edge {
    jzombie_graph::graph::Edge {
        source: source.into(),
        target: target.into(),
        kind,
        declaring_branches: vec!["main".into()],
        recency: None,
        version: None,
    }
}

fn fixture() -> Graph {
    // lib <- mid <- {app, app2}: every origin node load-bearing.
    let mut nodes = vec![
        node("me/lib", false, false),
        node("me/mid", false, false),
        node("me/app", false, false),
        node("me/app2", false, false),
    ];
    let edges = vec![
        edge("me/mid", "me/lib", EdgeKind::Manifest),
        edge("me/app", "me/mid", EdgeKind::Manifest),
        edge("me/app2", "me/mid", EdgeKind::Manifest),
        edge("me/app", "me/lib", EdgeKind::Docs),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    Graph::new(nodes, edges)
}

fn arch(md: &str) -> &str {
    md.split("<pre>")
        .nth(1)
        .unwrap()
        .split("</pre>")
        .next()
        .unwrap()
}

#[test]
fn doc_is_only_the_arch() {
    let md = render_cathedral(&fixture());
    assert!(md.contains("<pre>"), "whitespace-preserved block");
    assert!(md.contains("▲"), "crown");
    assert!(md.contains("FOUNDATION:"), "bedrock banner");
    assert!(md.contains(">app</a>"), "spire drawn and linked");
    assert!(md.contains(">app2</a>"), "whole canopy drawn");
    assert!(md.contains(">mid</a>"), "course drawn and linked");
    assert!(md.contains(">lib</a>"), "bedrock drawn and linked");
    assert!(md.contains('|'), "bearings drawn");
    assert!(!md.contains("## "), "no sections");
    assert!(!md.contains("- ["), "no markdown lists");
    assert!(!md.contains("| ---") && !md.contains("--- |"), "no tables");
    assert!(!md.contains("```"), "no code fences");
    assert!(!md.contains("Index:"), "no index lists");
    assert!(!md.contains("Detached"), "no detached lists");
}

#[test]
fn forks_appear_nowhere() {
    let mut nodes = vec![
        node("me/app", false, false),
        node("me/lib", false, false),
        node("me/vendor", true, false),
    ];
    let edges = vec![
        edge("me/app", "me/lib", EdgeKind::Manifest),
        edge("me/app", "me/vendor", EdgeKind::Manifest),
        edge("me/lib", "me/vendor", EdgeKind::Manifest),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cathedral(&Graph::new(nodes, edges));
    assert!(!md.contains("vendor"), "no fork anywhere");
}

#[test]
fn labels_abbreviate() {
    let mut nodes = vec![
        node("me/rust-widget", false, false),
        node("me/lib", false, false),
    ];
    let edges = vec![edge("me/rust-widget", "me/lib", EdgeKind::Manifest)];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cathedral(&Graph::new(nodes, edges));
    assert!(md.contains(">widget</a>"), "abbreviated label linked");
    assert!(!md.contains("rust-widget</a>"), "no long labels");
}

#[test]
fn empty_graph_renders() {
    let g = Graph::new(vec![], vec![]);
    let md = render_cathedral(&g);
    assert!(md.contains("# Cathedral"));
    assert!(md.contains("(no load-bearing structure)"));
}
