use crate::graph::Graph;

/// JSON-first: stable schema, jq-queryable from day one.
/// Documented recipes (README): who-depends-on-X, orphans,
/// heaviest edges, what-changed (diff yesterday vs today).
pub fn render_json(graph: &Graph) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(graph)?)
}

/// `--format table`: ten-second value for strangers.
/// Human-readable summary to stdout, not a render input.
pub fn render_table(graph: &Graph) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "jzombie-graph: {} nodes ({} visible), {} edges [schema v{}]\n",
        graph.nodes.len(),
        graph.nodes.iter().filter(|n| n.visible).count(),
        graph.edges.len(),
        graph.schema_version,
    ));
    s.push_str("--- nodes (weight-desc) ---\n");
    for (n, w) in graph.nodes_by_weight() {
        let vis = if n.visible { "vis" } else { "hid" };
        let fork = if n.fork { " fork" } else { "" };
        let privm = if n.private { " private" } else { "" };
        s.push_str(&format!(
            "{n_id:30} w={w:6.1} {vis}{fork}{privm}\n",
            n_id = n.id
        ));
    }
    s.push_str("--- heaviest edges (top 20) ---\n");
    let mut edges = graph.edges.clone();
    edges.sort_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for e in edges.iter().take(20) {
        s.push_str(&format!(
            "{:30} -> {:30} {:9} w={:.1}\n",
            e.source,
            e.target,
            format!("{:?}", e.kind).to_lowercase(),
            e.weight
        ));
    }
    s
}

/// `--format dot`: dev-time eyeballing only. Graphviz never enters
/// the render pipeline. Invisible nodes render dashed.
pub fn render_dot(graph: &Graph) -> String {
    let mut s = String::new();
    s.push_str("digraph portfolio {\n  rankdir=LR;\n");
    for n in &graph.nodes {
        let style = if n.visible { "" } else { ", style=dashed" };
        let shape = if n.fork { "box" } else { "ellipse" };
        s.push_str(&format!("  \"{}\" [shape={}{}];\n", n.id, shape, style));
    }
    for e in &graph.edges {
        let kind = format!("{:?}", e.kind).to_lowercase();
        s.push_str(&format!(
            "  \"{}\" -> \"{}\" [label=\"{} {:.1}\"];\n",
            e.source, e.target, kind, e.weight
        ));
    }
    s.push_str("}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Edge, EdgeKind, Node};

    fn fixture() -> Graph {
        let nodes = vec![
            Node {
                id: "me/a".into(),
                owner: "me".into(),
                name: "a".into(),
                fork: false,
                private: false,
                upstream: None,
                ahead_by: None,
                branches: vec!["main".into()],
                visible: true,
                visible_reason: Some("tier3: >=1 portfolio edge".into()),
            },
            Node {
                id: "me/b".into(),
                owner: "me".into(),
                name: "b".into(),
                fork: false,
                private: false,
                upstream: None,
                ahead_by: None,
                branches: vec!["main".into()],
                visible: true,
                visible_reason: Some("tier3: >=1 portfolio edge".into()),
            },
        ];
        let edges = vec![Edge {
            source: "me/a".into(),
            target: "me/b".into(),
            kind: EdgeKind::Manifest,
            declaring_branches: vec!["main".into()],
            recency: None,
            weight: 1.0,
            version: Some("1.0.0".into()),
            note: None,
        }];
        Graph::new(nodes, edges)
    }

    #[test]
    fn json_roundtrips_schema_version() {
        let g = fixture();
        let text = render_json(&g).unwrap();
        let back: Graph = serde_json::from_str(&text).unwrap();
        assert_eq!(back.schema_version, crate::graph::SCHEMA_VERSION);
        assert_eq!(back.edges.len(), 1);
    }

    #[test]
    fn table_and_dot_contain_ids() {
        let g = fixture();
        assert!(render_table(&g).contains("me/a"));
        let dot = render_dot(&g);
        assert!(dot.contains("digraph") && dot.contains("me/a"));
    }
}
