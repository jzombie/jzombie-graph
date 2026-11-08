//! Fixture tests: the doc is one clickable arch — crown, spire,
//! courses, convergence, bedrock banner. No lists, no tables.

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
    // lib <- mid <- app (apex by fan-out), plus a detached island.
    let mut nodes = vec![
        node("me/lib", false, false),
        node("me/mid", false, false),
        node("me/app", false, false),
        node("me/island-a", false, false),
        node("me/island-b", false, false),
    ];
    let edges = vec![
        edge("me/mid", "me/lib", EdgeKind::Manifest),
        edge("me/app", "me/mid", EdgeKind::Manifest),
        edge("me/app", "me/lib", EdgeKind::Docs),
        edge("me/island-a", "me/island-b", EdgeKind::Manifest),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    Graph::new(nodes, edges)
}

#[test]
fn doc_is_only_the_arch() {
    let md = render_cathedral(&fixture());
    assert!(md.contains("<pre>"), "whitespace-preserved block");
    assert!(md.contains("▲"), "crown");
    assert!(md.contains("FOUNDATION:"), "bedrock banner");
    assert!(md.contains(">app</a>"), "apex spire drawn and linked");
    assert!(md.contains(">mid</a>"), "course drawn and linked");
    assert!(md.contains(">lib</a>"), "bedrock drawn and linked");
    assert!(!md.contains("## "), "no sections — the arch is the doc");
    assert!(!md.contains("- ["), "no markdown lists");
    assert!(!md.contains(" | "), "no tables");
    assert!(!md.contains("```"), "no code fences");
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
fn detached_never_drawn() {
    let md = render_cathedral(&fixture());
    let art = md
        .split("<pre>")
        .nth(1)
        .unwrap()
        .split("</pre>")
        .next()
        .unwrap();
    assert!(!art.contains("island"), "islands not drawn");
}

#[test]
fn empty_graph_renders() {
    let g = Graph::new(vec![], vec![]);
    let md = render_cathedral(&g);
    assert!(md.contains("# Cathedral"));
    assert!(md.contains("(no load-bearing structure)"));
}
