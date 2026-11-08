use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Stable schema version. Bump only with a documented migration.
pub const SCHEMA_VERSION: u32 = 4;

/// Mechanically collected edge kinds. Dependence kinds count toward fan-in;
/// association kinds display only (see plan: workflow/docs/xrefs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    Manifest,
    Submodule,
    Registry,
    Workflow,
    Docs,
    Xrefs,
}

impl EdgeKind {
    /// Per spec: manifest/submodule/registry are dependence (count toward
    /// fan-in); lockfile is a gate (not a kind); workflow/docs/xrefs are
    /// associations (displayed only).
    pub fn is_dependence(self) -> bool {
        matches!(
            self,
            EdgeKind::Manifest | EdgeKind::Submodule | EdgeKind::Registry
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Canonical id: `owner/repo` full name.
    pub id: String,
    pub owner: String,
    pub name: String,
    /// API-native fork flag. Rides along as fact, not verdict.
    pub fork: bool,
    pub private: bool,
    /// Upstream full name when `fork == true` (if known).
    pub upstream: Option<String>,
    /// Commits ahead of upstream (`compare` API). Rule (a): divergence re-admits forks.
    pub ahead_by: Option<u32>,
    /// Last push to the repo (API `pushed_at`). Display fact only, never scored.
    /// Free in the enumerate/get responses — staleness/heartbeat signal.
    pub pushed_at: Option<DateTime<Utc>>,
    /// Repo-about facts (API-native, free in the same responses).
    /// Display/classification only, never scored. Stripped with private nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Branches observed during collection.
    pub branches: Vec<String>,
    /// Computed visibility: renderers filter blindly on this, never re-decide.
    pub visible: bool,
    /// Why visible / hidden — machine-readable for the briefing digest.
    pub visible_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
    /// Every branch whose manifest (or equivalent) declares this edge.
    pub declaring_branches: Vec<String>,
    /// Recency of the most recent declaring branch (RFC3339), if known.
    pub recency: Option<DateTime<Utc>>,
    /// Resolved version as briefing fact (lockfile), never scored.
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    pub schema_version: u32,
    pub generated_at: DateTime<Utc>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Graph {
    pub fn new(nodes: Vec<Node>, mut edges: Vec<Edge>) -> Self {
        // Deterministic output: sort nodes by id, edges by (source, target, kind).
        let mut nodes = nodes;
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        edges.sort_by(|a, b| (&a.source, &a.target, a.kind).cmp(&(&b.source, &b.target, b.kind)));
        Self {
            schema_version: SCHEMA_VERSION,
            generated_at: Utc::now(),
            nodes,
            edges,
        }
    }

    /// Node fan-in: count of incoming *dependence* edges (how many portfolio
    /// repos depend on this node). Associations never contribute.
    /// This is the "most depended-upon" ranking — no scope, no counting
    /// declaration sites.
    pub fn node_fan_in(&self) -> BTreeMap<String, u32> {
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for n in &self.nodes {
            counts.entry(n.id.clone()).or_insert(0);
        }
        for e in &self.edges {
            if e.kind.is_dependence() {
                *counts.entry(e.target.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Ordering: fan-in desc, stability tiebreak (id asc). Pure helper for renderers.
    pub fn nodes_by_fan_in(&self) -> Vec<(&Node, u32)> {
        let counts = self.node_fan_in();
        let mut out: Vec<(&Node, u32)> = self
            .nodes
            .iter()
            .map(|n| (n, *counts.get(&n.id).unwrap_or(&0)))
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
        out
    }
}

/// Compute `visible` for every node per the locked rules:
///
/// - Private repos: `visible = false` always (they compute as transit nodes,
///   stripped before render — no nodes, stubs, links, counts, or deltas).
/// - Forks: excluded by default; re-admitted by (a) divergence (`ahead_by > 0`)
///   or (b) load-bearing fan-in (a portfolio repo depends on the fork).
/// - Tier 3 isolation: >= 1 portfolio edge (in or out, dependence *or*
///   association) or omit. Study-forks pointing at other people's repos
///   self-omit here because their only edges leave the portfolio.
pub fn apply_visibility(nodes: &mut [Node], edges: &[Edge]) {
    let portfolio: BTreeSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();

    // Portfolio-internal edges only (both endpoints in the collected set).
    // Private nodes still count as transit for fan-in purposes.
    let mut has_portfolio_edge: BTreeSet<String> = BTreeSet::new();
    let mut has_fanin: BTreeSet<String> = BTreeSet::new();
    for e in edges {
        let internal =
            portfolio.contains(e.source.as_str()) && portfolio.contains(e.target.as_str());
        if !internal {
            continue;
        }
        has_portfolio_edge.insert(e.source.clone());
        has_portfolio_edge.insert(e.target.clone());
        // Fan-in: someone in the portfolio depends *on* this node.
        // Any edge kind counts as structural fan-in for the fork rule;
        // the fan-in *count* (scoring) uses dependence edges only.
        has_fanin.insert(e.target.clone());
    }

    for n in nodes.iter_mut() {
        if n.private {
            n.visible = false;
            n.visible_reason = Some("private-transit: computes, stripped before render".into());
            continue;
        }
        if n.fork {
            let diverged = n.ahead_by.unwrap_or(0) > 0;
            let load_bearing = has_fanin.contains(n.id.as_str());
            if !diverged && !load_bearing {
                // Forks without divergence or fan-in are hidden even if they
                // would otherwise pass Tier 3 (e.g. a study fork whose only
                // portfolio edge is its upstream link out — not own work).
                // They still need a reason string for the digest.
                n.visible = false;
                n.visible_reason = Some("fork: neither diverged nor load-bearing".into());
                continue;
            }
            // Re-admitted forks still pass through Tier 3 below (labeled fork).
        }
        if has_portfolio_edge.contains(n.id.as_str()) {
            n.visible = true;
            n.visible_reason = Some(if n.fork {
                if n.ahead_by.unwrap_or(0) > 0 {
                    "fork: diverged (ahead of upstream), has portfolio edge".into()
                } else {
                    "fork: load-bearing fan-in, has portfolio edge".into()
                }
            } else {
                "tier3: >=1 portfolio edge".into()
            });
        } else {
            n.visible = false;
            n.visible_reason = Some("tier3: isolated, no portfolio edge".into());
        }
    }
}

// Helper removed: ids are canonical (`owner/repo`), sets now own their Strings.

/// Public subgraph: both endpoints public. Scoring sees the true graph;
/// all surfaces show this. Private nodes vanish entirely (no stubs).
pub fn public_subgraph(graph: &Graph) -> Graph {
    let public: BTreeSet<&str> = graph
        .nodes
        .iter()
        .filter(|n| !n.private)
        .map(|n| n.id.as_str())
        .collect();
    let nodes: Vec<Node> = graph.nodes.iter().filter(|n| !n.private).cloned().collect();
    let edges: Vec<Edge> = graph
        .edges
        .iter()
        .filter(|e| public.contains(e.source.as_str()) && public.contains(e.target.as_str()))
        .cloned()
        .collect();
    Graph::new(nodes, edges)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            branches: vec!["main".into()],
            visible: false,
            visible_reason: None,
        }
    }

    fn edge(source: &str, target: &str, kind: EdgeKind) -> Edge {
        Edge {
            source: source.into(),
            target: target.into(),
            kind,
            declaring_branches: vec!["main".into()],
            recency: None,
            version: None,
        }
    }

    #[test]
    fn fork_excluded_by_default_diverged_readmitted() {
        let mut nodes = vec![
            node("me/a", false, false, None),
            node("me/study-fork", true, false, Some(0)),
            node("me/diverged-fork", true, false, Some(7)),
        ];
        let edges = vec![
            edge("me/a", "me/diverged-fork", EdgeKind::Manifest),
            // study-fork points at someone else's repo: no portfolio edge.
        ];
        apply_visibility(&mut nodes, &edges);
        let v: BTreeMap<_, _> = nodes.iter().map(|n| (n.id.as_str(), n.visible)).collect();
        assert!(v["me/a"]);
        assert!(!v["me/study-fork"]);
        // diverged fork with a portfolio edge (fan-in from me/a) is visible.
        assert!(v["me/diverged-fork"]);
    }

    #[test]
    fn load_bearing_fork_admitted_without_divergence() {
        let mut nodes = vec![
            node("me/app", false, false, None),
            node("me/load-bearing-fork", true, false, Some(0)),
        ];
        let edges = vec![edge("me/app", "me/load-bearing-fork", EdgeKind::Manifest)];
        apply_visibility(&mut nodes, &edges);
        assert!(
            nodes
                .iter()
                .find(|n| n.id == "me/load-bearing-fork")
                .unwrap()
                .visible
        );
    }

    #[test]
    fn tier3_isolates_edgeless_nodes() {
        let mut nodes = vec![
            node("me/a", false, false, None),
            node("me/lonely", false, false, None),
        ];
        let edges = vec![];
        apply_visibility(&mut nodes, &edges);
        assert!(!nodes.iter().find(|n| n.id == "me/lonely").unwrap().visible);
        assert!(!nodes.iter().find(|n| n.id == "me/a").unwrap().visible);
    }

    #[test]
    fn private_nodes_are_transit_but_stripped() {
        let mut nodes = vec![
            node("me/pub", false, false, None),
            node("me/secret", false, true, None),
        ];
        let edges = vec![edge("me/pub", "me/secret", EdgeKind::Manifest)];
        apply_visibility(&mut nodes, &edges);
        // Private computes (edge exists in the true graph) but is never visible.
        assert!(!nodes.iter().find(|n| n.id == "me/secret").unwrap().visible);
        let g = Graph::new(nodes, edges);
        let pub_g = public_subgraph(&g);
        assert!(pub_g.nodes.iter().all(|n| !n.private));
        assert!(pub_g.edges.is_empty(), "edge to private node must vanish");
    }

    #[test]
    fn fan_in_counts_dependence_only() {
        let g = Graph::new(
            vec![
                node("me/a", false, false, None),
                node("me/b", false, false, None),
            ],
            vec![
                edge("me/a", "me/b", EdgeKind::Manifest),
                edge("me/a", "me/b", EdgeKind::Workflow),
                edge("me/a", "me/b", EdgeKind::Docs),
            ],
        );
        assert_eq!(g.node_fan_in()["me/b"], 1);
    }
}
