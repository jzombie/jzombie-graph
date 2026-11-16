//! Cluster tests: proximity from real edges, links intact, portfolio
//! filter (own-work forks only, no stale), no tables or tier language.

use cartographer::render_cartographer;
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
        commits_1d: 0,
        commits_5d: 0,
        commits_15d: 0,
        commits_30d: 0,
        issues_open_1d: 0,
        issues_open_5d: 0,
        issues_open_15d: 0,
        issues_open_30d: 0,
        issues_closed_1d: 0,
        issues_closed_5d: 0,
        issues_closed_15d: 0,
        issues_closed_30d: 0,
        issue_comments_1d: 0,
        issue_comments_5d: 0,
        issue_comments_15d: 0,
        issue_comments_30d: 0,
        issue_labels: Default::default(),
        recent_issues: vec![],
        issue_heat: None,
        commit_heat: None,
        focus: None,
        last_commit_at: None,
        branch_heat: vec![],
        recent_prs: vec![],
        visible: true,
        visible_reason: None,
        layer: std::collections::BTreeSet::from([Layer::Isolated]),
        has_default_binary: false,
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
fn cluster_draws_linked_nodes_with_braille_traces() {
    let md = render_cartographer(&fixture());
    // Header derives from data: owner title, never brand strings.
    assert!(md.contains("me portfolio cluster"), "data-driven title");
    assert!(
        md.contains("Repos are in this chart through interconnectedness"),
        "inclusion sentence, no counts"
    );
    assert!(!md.contains("zenOSmosis"), "no hardcoded brand");
    assert!(!md.contains("Zenosmosis"), "old casing gone");
    assert!(
        md.contains("<pre aria-hidden=\"true\">"),
        "projection block"
    );
    assert!(md.contains("</pre>"), "projection closes");
    assert!(
        md.contains("Screen readers:"),
        "screen-reader prelude present"
    );
    assert!(
        md.contains("<a href=\"https://github.com/me/app\">app</a>"),
        "nodes clickable"
    );
    assert!(md.contains(">mid</a>"), "mid linked");
    assert!(md.contains(">lib</a>"), "lib linked");
    assert!(
        md.chars().any(|c| ('\u{2800}'..='\u{28FF}').contains(&c)),
        "braille traces drawn"
    );
}

#[test]
fn no_tables_or_tier_language() {
    let md = render_cartographer(&fixture());
    assert!(!md.contains("| Level |"), "no elevation table");
    assert!(!md.contains("Spires"), "no tier names");
    assert!(!md.contains("Vaults"), "no tier names");
    assert!(!md.contains("Crypt"), "no crypt");
    assert!(!md.contains("FOUNDATION"), "no bedrock banner");
    assert!(!md.contains("Side Chapels"), "no detached-note lists");
    assert!(!md.contains("Lone stones"), "no detached-note lists");
}

#[test]
fn docs_edges_draw_same_braille_trace() {
    // app -> lib is docs-only: lighter gravity in layout, same braille
    // glyphs in the drawing. Workflow-only edges attach nothing.
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
    let md = render_cartographer(&Graph::new(nodes, edges));
    assert!(
        md.contains(">a</a>") && md.contains(">c</a>"),
        "nodes drawn"
    );
    assert!(
        md.chars().any(|c| ('\u{2800}'..='\u{28FF}').contains(&c)),
        "docs reference drawn in braille"
    );
}

#[test]
fn docs_satellite_with_prose_but_no_language_stays_out() {
    // Description + topics but no code: still a bucket, stays out even if
    // someone writes it a nice readme blurb later.
    let asset = Node {
        language: None,
        ..node("me/blobs", false, false)
    };
    let mut nodes = vec![
        node("me/a", false, false),
        node("me/b", false, false),
        asset,
    ];
    let edges = vec![
        edge("me/a", "me/b", EdgeKind::Manifest),
        edge("me/a", "me/blobs", EdgeKind::Docs),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cartographer(&Graph::new(nodes, edges));
    assert!(!md.contains("blobs"), "prose-only bucket stays out");
}

#[test]
fn docs_satellite_with_language_is_kept() {
    let tool = Node {
        language: Some("Shell".into()),
        ..node("me/tool", false, false)
    };
    let mut nodes = vec![node("me/a", false, false), node("me/b", false, false), tool];
    let edges = vec![
        edge("me/a", "me/b", EdgeKind::Manifest),
        edge("me/a", "me/tool", EdgeKind::Docs),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cartographer(&Graph::new(nodes, edges));
    assert!(md.contains("tool"), "code satellite kept");
}

#[test]
fn workflow_edges_attach_nothing() {
    let mut nodes = vec![node("me/a", false, false), node("me/tool", false, false)];
    let edges = vec![edge("me/a", "me/tool", EdgeKind::Workflow)];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cartographer(&Graph::new(nodes, edges));
    assert!(!md.contains("tool"), "workflow-only node stays out");
}

#[test]
fn docs_satellite_without_project_signals_stays_out() {
    // Asset bucket: no description, language, or topics — not graphed even
    // with a direct docs edge to the core.
    let asset = Node {
        description: None,
        language: None,
        topics: vec![],
        ..node("me/blobs", false, false)
    };
    let mut nodes = vec![
        node("me/a", false, false),
        node("me/b", false, false),
        asset,
    ];
    let edges = vec![
        edge("me/a", "me/b", EdgeKind::Manifest),
        edge("me/a", "me/blobs", EdgeKind::Docs),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cartographer(&Graph::new(nodes, edges));
    assert!(!md.contains("blobs"), "asset bucket stays out");
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
    let md = render_cartographer(&Graph::new(nodes, edges));
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
    let md = render_cartographer(&Graph::new(nodes, edges));
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
    let md = render_cartographer(&Graph::new(nodes, edges));
    assert!(md.contains("term-wm-vt100"), "renamed fork kept");
}

#[test]
fn stale_dependent_is_omitted() {
    // old depends on app (stale user, not a dependency): stays out.
    let mut nodes = vec![node("me/app", false, false), node("me/old", false, false)];
    nodes[1].pushed_at = Some(Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).single().unwrap());
    let edges = vec![edge("me/old", "me/app", EdgeKind::Manifest)];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let mut g = Graph::new(nodes, edges);
    g.generated_at = Utc.with_ymd_and_hms(2026, 11, 8, 0, 0, 0).single().unwrap();
    let md = render_cartographer(&g);
    assert!(!md.contains("old"), "stale dependent omitted");
}

#[test]
fn stale_dependency_is_kept() {
    // app depends on old (stale library): pulled back in.
    let mut nodes = vec![node("me/app", false, false), node("me/old", false, false)];
    nodes[1].pushed_at = Some(Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).single().unwrap());
    let edges = vec![edge("me/app", "me/old", EdgeKind::Manifest)];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let mut g = Graph::new(nodes, edges);
    g.generated_at = Utc.with_ymd_and_hms(2026, 11, 8, 0, 0, 0).single().unwrap();
    let md = render_cartographer(&g);
    assert!(md.contains("old"), "stale dependency kept");
}

#[test]
fn stale_dependency_chain_is_kept_recursively() {
    // app -> mid (stale) -> old (stale): both pulled back in.
    let mut nodes = vec![
        node("me/app", false, false),
        node("me/mid", false, false),
        node("me/old", false, false),
    ];
    let stale_at = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).single().unwrap();
    nodes[1].pushed_at = Some(stale_at);
    nodes[2].pushed_at = Some(stale_at);
    let edges = vec![
        edge("me/app", "me/mid", EdgeKind::Manifest),
        edge("me/mid", "me/old", EdgeKind::Manifest),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let mut g = Graph::new(nodes, edges);
    g.generated_at = Utc.with_ymd_and_hms(2026, 11, 8, 0, 0, 0).single().unwrap();
    let md = render_cartographer(&g);
    assert!(md.contains("mid"), "stale direct dependency kept");
    assert!(md.contains("old"), "stale transitive dependency kept");
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
    let md = render_cartographer(&Graph::new(nodes, edges));
    assert!(md.contains(">widget</a>"), "abbreviated label linked");
    assert!(!md.contains("rust-widget</a>"), "no long labels");
}

#[test]
fn header_sentence_states_inclusion_with_no_details() {
    let md = render_cartographer(&fixture());
    assert!(
        md.contains("Repos are in this chart through interconnectedness: each one builds on, is used by, or documents another repo here."),
        "inclusion sentence verbatim:\n{md}"
    );
    assert!(!md.contains("repos connect"), "no connect-counts phrasing");
    assert!(!md.contains("links between"), "no edge counts");
    assert!(!md.contains("foundation"), "no layer census");
    assert!(!md.contains("middleware"), "no layer census");
    assert!(!md.contains("application"), "no layer census");
    assert!(!md.contains("isolated"), "no layer census");
    assert!(!md.contains("Busiest right now"), "no busiest clause");
    assert!(!md.contains("most reused"), "no fan-in clause");
    assert!(
        md.contains("left out unless something current uses them"),
        "omission rule stated"
    );
}

#[test]
fn empty_graph_renders() {
    let g = Graph::new(vec![], vec![]);
    let md = render_cartographer(&g);
    assert!(md.contains("portfolio cluster"));
    assert!(md.contains("(no connected cluster)"));
}

#[test]
fn profile_repo_with_docs_to_everything_stays_out() {
    // `me/me` is the GitHub profile README: it documents the portfolio but
    // is never a project node, even with detected language and docs edges
    // to the core.
    let profile = Node {
        language: Some("Go Template".into()),
        ..node("me/me", false, false)
    };
    let mut nodes = vec![
        node("me/a", false, false),
        node("me/b", false, false),
        profile,
    ];
    let edges = vec![
        edge("me/a", "me/b", EdgeKind::Manifest),
        edge("me/me", "me/a", EdgeKind::Docs),
        edge("me/me", "me/b", EdgeKind::Docs),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cartographer(&Graph::new(nodes, edges));
    assert!(md.contains(">a</a>"), "core kept");
    assert!(
        !md.contains("github.com/me/me"),
        "profile README excluded:\n{md}"
    );
}

#[test]
fn profile_repo_cannot_drag_isolated_nodes_in() {
    // Isolated `lonely` documents nothing but the profile hub: with the hub
    // excluded there is no anchor left, so it stays out too (no cascade).
    let profile = Node {
        language: Some("Go Template".into()),
        ..node("me/me", false, false)
    };
    let lonely = Node {
        language: Some("Rust".into()),
        ..node("me/lonely", false, false)
    };
    let mut nodes = vec![
        node("me/a", false, false),
        node("me/b", false, false),
        profile,
        lonely,
    ];
    let edges = vec![
        edge("me/a", "me/b", EdgeKind::Manifest),
        edge("me/me", "me/a", EdgeKind::Docs),
        edge("me/lonely", "me/me", EdgeKind::Docs),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cartographer(&Graph::new(nodes, edges));
    assert!(
        !md.contains("github.com/me/lonely"),
        "satellite of an excluded hub stays out:\n{md}"
    );
}

#[test]
fn docs_hub_with_many_edges_stays_out() {
    // A documentation index (many docs edges) is not runtime coupling,
    // even when it has detected language and touches the core.
    let mut nodes = vec![node("me/a", false, false), node("me/b", false, false)];
    let hub = Node {
        language: Some("Rust".into()),
        ..node("me/hub", false, false)
    };
    nodes.push(hub);
    for i in 0..9 {
        nodes.push(node(&format!("me/dummy{i}"), false, false));
    }
    let mut edges = vec![
        edge("me/a", "me/b", EdgeKind::Manifest),
        edge("me/hub", "me/a", EdgeKind::Docs),
    ];
    for i in 0..9 {
        edges.push(edge("me/hub", &format!("me/dummy{i}"), EdgeKind::Docs));
    }
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cartographer(&Graph::new(nodes, edges));
    assert!(
        !md.contains("github.com/me/hub"),
        "docs hub excluded:\n{md}"
    );
    assert!(md.contains(">a</a>"), "core kept");
}

#[test]
fn satellite_cannot_anchor_another_satellite() {
    // `mid` attaches to the core via docs; `zebra` only documents `mid`.
    // Edge sort order would have chained them under a mutating core, so
    // this pins the snapshot-anchor behavior.
    let mid = Node {
        language: Some("Shell".into()),
        ..node("me/mid", false, false)
    };
    let zebra = Node {
        language: Some("Shell".into()),
        ..node("me/zebra", false, false)
    };
    let mut nodes = vec![
        node("me/a", false, false),
        node("me/b", false, false),
        mid,
        zebra,
    ];
    let edges = vec![
        edge("me/a", "me/b", EdgeKind::Manifest),
        edge("me/mid", "me/a", EdgeKind::Docs),
        edge("me/zebra", "me/mid", EdgeKind::Docs),
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let md = render_cartographer(&Graph::new(nodes, edges));
    assert!(md.contains("mid"), "direct satellite kept");
    assert!(
        !md.contains("zebra"),
        "satellite of a satellite stays out:\n{md}"
    );
}
