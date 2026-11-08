//! Fixture tests: the arch draws the core, the table links every stone,
//! chapels cover what the arch doesn't. No lists-as-structure, no tables
//! of contents — just the building, its register, and its chapels.

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
    // lib <- mid <- {app, app2}; island-a -> island-b detached.
    let mut nodes = vec![
        node("me/lib", false, false),
        node("me/mid", false, false),
        node("me/app", false, false),
        node("me/app2", false, false),
        node("me/island-a", false, false),
        node("me/island-b", false, false),
    ];
    let edges = vec![
        edge("me/mid", "me/lib", EdgeKind::Manifest),
        edge("me/app", "me/mid", EdgeKind::Manifest),
        edge("me/app2", "me/mid", EdgeKind::Manifest),
        edge("me/app", "me/lib", EdgeKind::Docs),
        edge("me/island-a", "me/island-b", EdgeKind::Manifest),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    Graph::new(nodes, edges)
}

#[test]
fn arch_draws_core_with_clickable_boxes() {
    let md = render_cathedral(&fixture());
    assert!(md.contains("<pre>"), "whitespace-preserved block");
    assert!(md.contains("▲"), "crown");
    assert!(md.contains("FOUNDATION:"), "bedrock banner");
    assert!(
        md.contains("<a href=\"https://github.com/me/app\">app</a>"),
        "apex stone linked"
    );
    assert!(md.contains(">mid</a>"), "course stone linked");
    assert!(md.contains(">lib</a>"), "bedrock stone linked");
    assert!(md.contains('|'), "bearings drawn");
}

#[test]
fn table_links_every_stone() {
    let md = render_cathedral(&fixture());
    assert!(md.contains("| Level |"), "register header");
    assert!(
        md.contains("[`app`](https://github.com/me/app)"),
        "table links core nodes"
    );
    assert!(
        md.contains("rests on mid"),
        "table shows bearings with versions"
    );
}

#[test]
fn chapels_cover_the_detached() {
    let md = render_cathedral(&fixture());
    assert!(md.contains("Side Chapels"), "chapels section exists");
    assert!(md.contains("island-a"), "detached island drawn");
    assert!(md.contains("island-b"), "island bearer drawn");
}

#[test]
fn forks_appear_only_where_they_bear_load() {
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
    // vendor bears load (fan-in 2): named in the crypt ...
    assert!(md.contains("vendor"), "load-bearing fork named in crypt");
    // ... but never drawn as a stone.
    let art = md
        .split("<pre>")
        .nth(1)
        .unwrap()
        .split("</pre>")
        .next()
        .unwrap();
    assert!(!art.contains("[vendor]"), "no fork boxes in the arch");
}

#[test]
fn load_less_forks_appear_nowhere() {
    let mut nodes = vec![
        node("me/app", false, false),
        node("me/lib", false, false),
        node("me/leaf", true, false),
    ];
    let edges = vec![
        edge("me/app", "me/lib", EdgeKind::Manifest),
        edge("me/leaf", "me/lib", EdgeKind::Manifest),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cathedral(&Graph::new(nodes, edges));
    assert!(!md.contains("leaf"), "unbearing fork omitted entirely");
}

#[test]
fn labels_abbreviate_rust_prefix() {
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
    assert!(md.contains("# The Zenosmosis Cathedral"));
    assert!(md.contains("(no load-bearing structure)"));
}
