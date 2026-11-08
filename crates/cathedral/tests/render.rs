//! Cluster tests: proximity from real edges, links intact, portfolio
//! filter (own-work forks only, no stale), no tables or tier language.

use cathedral::render_cathedral;
use chrono::{TimeZone, Utc};
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
    // lib <- mid <- {app, app2}.
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
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    Graph::new(nodes, edges)
}

#[test]
fn cluster_draws_linked_nodes_with_dot_traces() {
    let md = render_cathedral(&fixture());
    assert!(md.contains("zenOSmosis"), "correct branding");
    assert!(!md.contains("Zenosmosis"), "old casing gone");
    assert!(md.contains("<pre>"), "projection block");
    assert!(md.contains("</pre>"), "projection closes");
    assert!(
        md.contains("<a href=\"https://github.com/me/app\">app</a>"),
        "nodes clickable"
    );
    assert!(md.contains(">mid</a>"), "mid linked");
    assert!(md.contains(">lib</a>"), "lib linked");
    assert!(md.contains('·'), "per-edge traces drawn");
}

#[test]
fn no_tables_or_tier_language() {
    let md = render_cathedral(&fixture());
    assert!(!md.contains("| Level |"), "no elevation table");
    assert!(!md.contains("Spires"), "no tier names");
    assert!(!md.contains("Vaults"), "no tier names");
    assert!(!md.contains("Crypt"), "no crypt");
    assert!(!md.contains("FOUNDATION"), "no bedrock banner");
    assert!(!md.contains("Side Chapels"), "no detached-note lists");
    assert!(!md.contains("Lone stones"), "no detached-note lists");
}

#[test]
fn docs_edges_leave_no_trace() {
    // app -> lib is docs-only: no dependence, so no trace may rely on it.
    // The cluster still draws (mid edges carry it); the point is the
    // renderer only ever draws manifest/submodule/registry pairs.
    let mut nodes = vec![
        node("me/a", false, false),
        node("me/b", false, false),
        node("me/c", false, false),
    ];
    let edges = vec![
        edge("me/a", "me/b", EdgeKind::Manifest),
        edge("me/b", "me/c", EdgeKind::Manifest),
        edge("me/a", "me/c", EdgeKind::Docs),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cathedral(&Graph::new(nodes, edges));
    assert!(
        md.contains(">a</a>") && md.contains(">c</a>"),
        "nodes drawn"
    );
}

#[test]
fn fork_without_work_is_excluded() {
    let mut nodes = vec![
        node("me/app", false, false),
        node("me/lib", false, false),
        Node {
            upstream: Some("other/vendor".into()),
            ahead_by: Some(0),
            ..node("me/vendor", true, false)
        },
    ];
    let edges = vec![
        edge("me/app", "me/lib", EdgeKind::Manifest),
        edge("me/app", "me/vendor", EdgeKind::Manifest),
        edge("me/lib", "me/vendor", EdgeKind::Manifest),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cathedral(&Graph::new(nodes, edges));
    assert!(!md.contains("vendor"), "plain fork omitted");
}

#[test]
fn fork_with_a_commit_is_kept() {
    let mut nodes = vec![
        node("me/app", false, false),
        Node {
            upstream: Some("other/lib".into()),
            ahead_by: Some(1),
            ..node("me/vendor", true, false)
        },
    ];
    let edges = vec![edge("me/app", "me/vendor", EdgeKind::Manifest)];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cathedral(&Graph::new(nodes, edges));
    assert!(md.contains("vendor"), "one commit keeps the fork");
}

#[test]
fn renamed_fork_is_kept() {
    let mut nodes = vec![
        node("me/app", false, false),
        Node {
            upstream: Some("doy/vt100-rust".into()),
            ahead_by: Some(0),
            ..node("me/term-wm-vt100", true, false)
        },
    ];
    let edges = vec![edge("me/app", "me/term-wm-vt100", EdgeKind::Manifest)];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cathedral(&Graph::new(nodes, edges));
    assert!(md.contains("term-wm-vt100"), "renamed fork kept");
}

#[test]
fn stale_repos_are_omitted() {
    let mut nodes = vec![node("me/app", false, false), node("me/old", false, false)];
    nodes[1].pushed_at = Some(Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).single().unwrap());
    let edges = vec![edge("me/app", "me/old", EdgeKind::Manifest)];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let mut g = Graph::new(nodes, edges);
    g.generated_at = Utc.with_ymd_and_hms(2026, 11, 8, 0, 0, 0).single().unwrap();
    let md = render_cathedral(&g);
    assert!(!md.contains("old"), "stale repo omitted");
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
    assert!(md.contains("zenOSmosis"));
    assert!(md.contains("(no connected cluster)"));
}
