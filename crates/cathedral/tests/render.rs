//! Fixture tests: layers land in the right sections, fan-in orders
//! nodes, associations never score.

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
fn sections_follow_stored_layers() {
    let md = render_cathedral(&fixture());
    assert!(md.contains("## foundation"), "foundation section");
    assert!(md.contains("## middleware"), "middleware section");
    assert!(md.contains("## application"), "application section");
    assert!(md.contains("## Isolated"), "isolated section");
    let foundation = md.split("## foundation").nth(1).unwrap();
    let foundation = foundation.split("## middleware").next().unwrap();
    assert!(foundation.contains("me/lib"), "lib in foundation");
    assert!(!foundation.contains("me/app"), "app not in foundation");
    let middleware = md.split("## middleware").nth(1).unwrap();
    let middleware = middleware.split("## application").next().unwrap();
    assert!(middleware.contains("me/mid"), "mid in middleware");
    // Docs-only loner stays isolated, collapsed.
    let isolated = md.split("## Isolated").nth(1).unwrap();
    assert!(isolated.contains("me/loner"), "loner isolated");
    assert!(isolated.contains("<details>"), "isolated collapsed");
}

#[test]
fn associations_render_but_never_score() {
    let md = render_cathedral(&fixture());
    // Docs edge renders on the app block...
    let app = md.split("### [`me/app`]").nth(1).unwrap();
    assert!(app.contains("[docs]"), "association listed");
    // ...but the app block still reports dependence-only tallies.
    assert!(
        app.contains("dependence fan-in: 0, fan-out: 1"),
        "scores ignore docs, got:\n{app}"
    );
}

#[test]
fn empty_graph_renders() {
    let g = Graph::new(vec![], vec![]);
    let md = render_cathedral(&g);
    assert!(md.contains("# Portfolio architecture"));
    assert!(md.contains("(none)"));
}
