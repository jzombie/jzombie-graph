//! Fixture tests: the arch draws the apex cone, forks never appear,
//! labels abbreviate, every drawn label links.

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
    // app -> mid -> lib, plus app -> lib docs (association, not drawn).
    // lib is foundation, mid middleware, app application.
    let mut nodes = vec![
        node("me/lib", false, false),
        node("me/mid", false, false),
        node("me/app", false, false),
        node("me/loner", false, false),
    ];
    let edges = vec![
        edge("me/mid", "me/lib", EdgeKind::Manifest),
        edge("me/app", "me/mid", EdgeKind::Manifest),
        edge("me/app", "me/lib", EdgeKind::Docs),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    Graph::new(nodes, edges)
}

#[test]
fn elevation_is_an_arch_with_clickable_stones() {
    let md = render_cathedral(&fixture());
    let elev = md
        .split("## Elevation")
        .nth(1)
        .unwrap()
        .split("\nIndex:")
        .next()
        .unwrap();
    assert!(elev.contains("<pre>"), "whitespace-preserved block");
    assert!(elev.contains("▲"), "crown");
    assert!(elev.contains("FOUNDATION:"), "bedrock banner");
    assert!(
        elev.contains("[app]") || elev.contains(">app</a>"),
        "apex spire drawn"
    );
    assert!(
        elev.contains("[mid]") || elev.contains(">mid</a>"),
        "course drawn"
    );
    assert!(
        elev.contains("[lib]") || elev.contains(">lib</a>"),
        "bedrock drawn"
    );
    assert!(elev.contains('|'), "bearings drawn");
    // Stones are links inside the drawing.
    assert!(
        elev.contains("<a href=\"https://github.com/me/lib\">lib</a>"),
        "nodes clickable in the art"
    );
    // And the index below maps every drawn label.
    let index = md.split("\nIndex:").nth(1).unwrap();
    assert!(
        index.contains("[lib](https://github.com/me/lib)"),
        "index links every label"
    );
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
    assert!(
        md.contains("[widget](https://github.com/me/rust-widget)"),
        "index uses the short label"
    );
}

#[test]
fn detached_pairs_are_named_not_drawn() {
    // island-a -> island-b: connected, but detached from the main core.
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
        edge("me/island-a", "me/island-b", EdgeKind::Manifest),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cathedral(&Graph::new(nodes, edges));
    let elev = md.split("## Elevation").nth(1).unwrap();
    let fence = elev
        .split("<pre>")
        .nth(1)
        .unwrap()
        .split("</pre>")
        .next()
        .unwrap();
    assert!(!fence.contains("island"), "islands not drawn");
    assert!(
        md.contains("Detached from the load-bearing core"),
        "detached named"
    );
    assert!(md.contains("island-a"), "island reachable from the index");
}

#[test]
fn empty_graph_renders() {
    let g = Graph::new(vec![], vec![]);
    let md = render_cathedral(&g);
    assert!(md.contains("# Cathedral"));
    assert!(md.contains("(no load-bearing structure)"));
}
