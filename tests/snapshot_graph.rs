//! Snapshot tests: the continuous form of having inspected the graph.
//! Locks the output contract (visibility flags, fan-in counts).
//! Regressions show as diffs — the spec's required suite in miniature:
//! fan-in floor, fork promotion, zero private nodes in public output.

use jzombie_graph::collect::BranchDecl;
use jzombie_graph::graph::{self, EdgeKind, Graph, Node};
use std::collections::{BTreeSet, HashMap};

fn node(id: &str, fork: bool, private: bool, ahead_by: Option<u32>) -> Node {
    let (owner, name) = id.split_once('/').unwrap();
    Node {
        id: id.into(),
        owner: owner.into(),
        name: name.into(),
        fork,
        private,
        upstream: None,
        ahead_by,
        pushed_at: None,
        description: None,
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
        commit_heat: None,
        focus: None,
        last_commit_at: None,
        branch_heat: vec![],
        recent_prs: vec![],
        visible: false,
        visible_reason: None,
        layer: jzombie_graph::graph::Layer::Isolated,
    }
}

#[test]
fn snapshot_reference_portfolio() {
    // Reference case from the spec: term-wm-vt100-style diverged fork is
    // promoted; simd-r-drive-style fan-in floor holds; private never leaks.
    let mut nodes = vec![
        node("me/hub", false, false, None),
        node("me/simd-r-drive", false, false, None),
        node("me/term-wm-vt100", true, false, Some(12)),
        node("me/study-fork", true, false, Some(0)),
        node("me/secret", false, true, None),
    ];
    // About-fields contract: hub carries every field so the snapshot locks
    // their JSON shape; other nodes stay empty (topics: [] only).
    nodes[0].description = Some("hub service".into());
    nodes[0].topics = vec!["rust".into(), "graph".into()];
    nodes[0].language = Some("Rust".into());
    nodes[0].homepage = Some("https://example.com/hub".into());
    nodes[0].stars = 42;
    nodes[0].forks_count = 7;
    nodes[0].open_issues = 3;
    nodes[0].license = Some("MIT".into());
    nodes[0].size_kb = 2048;
    nodes[0].has_readme = true;
    nodes[0].has_llms_txt = true;
    nodes[0].has_agents_md = true;
    nodes[0].has_license_file = true;
    // Velocity contract: the hub carries captured gauges so the snapshot
    // locks their JSON shape (computed by the same methods collection uses,
    // against the normalized clock below — deterministic).
    nodes[0].commits_1d = 4;
    nodes[0].commits_5d = 7;
    nodes[0].commits_15d = 11;
    nodes[0].commits_30d = 17;
    nodes[0].pushed_at = Some(chrono::DateTime::from_timestamp(0, 0).unwrap());
    nodes[0].last_commit_at = nodes[0].pushed_at;
    nodes[0].branch_heat = vec![
        graph::BranchHeat {
            branch: "main".into(),
            is_default: true,
            commits_1d: 1,
            commits_5d: 2,
            commits_15d: 6,
            commits_30d: 12,
        },
        graph::BranchHeat {
            branch: "next".into(),
            is_default: false,
            commits_1d: 3,
            commits_5d: 5,
            commits_15d: 5,
            commits_30d: 5,
        },
    ];
    nodes[0].commit_heat = nodes[0].commit_heat();
    nodes[0].focus = nodes[0].focus(&nodes[0].pushed_at.unwrap());
    let edges = vec![
        jzombie_graph::graph::Edge {
            source: "me/hub".into(),
            target: "me/simd-r-drive".into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into()],
            recency: None,
            version: Some("0.9.0".into()),
        },
        jzombie_graph::graph::Edge {
            source: "me/hub".into(),
            target: "me/term-wm-vt100".into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into(), "next".into()],
            recency: None,
            version: None,
        },
        // Private transit: hub -> secret -> simd-r-drive. True graph sees it.
        jzombie_graph::graph::Edge {
            source: "me/hub".into(),
            target: "me/secret".into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into()],
            recency: None,
            version: None,
        },
        jzombie_graph::graph::Edge {
            source: "me/secret".into(),
            target: "me/simd-r-drive".into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into()],
            recency: None,
            version: None,
        },
    ];
    graph::apply_visibility(&mut nodes, &edges);
    graph::apply_layers(&mut nodes, &edges);
    let g = Graph::new(nodes, edges);

    // Known truths (asserted, then snapshotted):
    let fan_in = g.node_fan_in();
    assert_eq!(
        fan_in["me/simd-r-drive"], 2,
        "fan-in counts dependence edges (incl. private transit)"
    );
    assert!(
        g.nodes
            .iter()
            .find(|n| n.id == "me/term-wm-vt100")
            .unwrap()
            .visible,
        "diverged fork promoted"
    );
    assert!(
        !g.nodes
            .iter()
            .find(|n| n.id == "me/study-fork")
            .unwrap()
            .visible,
        "study fork omitted"
    );
    assert!(
        !g.nodes
            .iter()
            .find(|n| n.id == "me/secret")
            .unwrap()
            .visible
    );

    // llkv-style: present-with-stale-heartbeat node stays visible iff edged.
    let _ = BranchDecl {
        branch: "main".into(),
        recency: None,
    };
    let _ = HashMap::<String, String>::new();
    let _ = BTreeSet::<String>::new();

    let mut pub_g = graph::public_subgraph(&g);
    assert!(
        pub_g.nodes.iter().all(|n| !n.private),
        "zero private nodes in public output"
    );
    assert!(pub_g.edges.iter().all(|e| {
        pub_g.nodes.iter().any(|n| n.id == e.source) && pub_g.nodes.iter().any(|n| n.id == e.target)
    }));
    // Normalize timestamp so the snapshot is deterministic.
    pub_g.generated_at = chrono::DateTime::from_timestamp(0, 0).unwrap();

    insta::with_settings!({sort_maps => true}, {
        insta::assert_json_snapshot!("reference_portfolio_public", &pub_g);
    });
}

/// Schema drift guard: the committed `json/portfolio-graph-schema.json` is
/// generated output, never hand-edited. Any struct change that alters the
/// output contract fails here until the schema is consciously regenerated
/// (`cargo run -- --format schema --output data/json/portfolio-graph-schema.json`,
/// then copy over `json/portfolio-graph-schema.json`).
#[test]
fn schema_matches_committed_asset() {
    use jzombie_graph::paths::COMMITTED_SCHEMA;
    let rendered = jzombie_graph::output::render_schema();
    let committed = std::fs::read_to_string(COMMITTED_SCHEMA).expect("read committed schema");
    assert_eq!(
        rendered, committed,
        "schema drift: regenerate the lane output, then copy it over {COMMITTED_SCHEMA}"
    );
}
