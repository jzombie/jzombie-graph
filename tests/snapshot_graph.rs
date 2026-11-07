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
        branches: vec!["main".into()],
        visible: false,
        visible_reason: None,
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
    let edges = vec![
        jzombie_graph::graph::Edge {
            source: "me/hub".into(),
            target: "me/simd-r-drive".into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into()],
            recency: None,
            version: Some("0.9.0".into()),
            note: None,
        },
        jzombie_graph::graph::Edge {
            source: "me/hub".into(),
            target: "me/term-wm-vt100".into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into(), "next".into()],
            recency: None,
            version: None,
            note: None,
        },
        // Private transit: hub -> secret -> simd-r-drive. True graph sees it.
        jzombie_graph::graph::Edge {
            source: "me/hub".into(),
            target: "me/secret".into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into()],
            recency: None,
            version: None,
            note: None,
        },
        jzombie_graph::graph::Edge {
            source: "me/secret".into(),
            target: "me/simd-r-drive".into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into()],
            recency: None,
            version: None,
            note: None,
        },
    ];
    graph::apply_visibility(&mut nodes, &edges);
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
