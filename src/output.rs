use crate::graph::Graph;

/// JSON-first: stable schema, jq-queryable from day one.
/// Documented recipes (README): who-depends-on-X, orphans,
/// most depended-upon, what-changed (diff yesterday vs today).
pub fn render_json(graph: &Graph) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(graph)?)
}

/// `--format dump`: full debug lane. Every node + every edge, all fields,
/// vertical, no truncation, no summary metrics standing in for data.
/// JSON stays the machine lane; this is the "see what it's doing" lane.
pub fn render_dump(graph: &Graph) -> String {
    use std::collections::BTreeMap;
    let mut s = String::new();
    s.push_str(&format!(
        "jzombie-graph dump: {} nodes ({} visible), {} edges [schema v{}] generated_at={}\n",
        graph.nodes.len(),
        graph.nodes.iter().filter(|n| n.visible).count(),
        graph.edges.len(),
        graph.schema_version,
        graph.generated_at.to_rfc3339(),
    ));
    // Per-node in/out adjacency for the vertical blocks.
    let mut incoming: BTreeMap<&str, Vec<&crate::graph::Edge>> = BTreeMap::new();
    let mut outgoing: BTreeMap<&str, Vec<&crate::graph::Edge>> = BTreeMap::new();
    for e in &graph.edges {
        incoming.entry(e.target.as_str()).or_default().push(e);
        outgoing.entry(e.source.as_str()).or_default().push(e);
    }
    s.push_str(&format!(
        "--- nodes ({}, fan-in desc) ---\n",
        graph.nodes.len()
    ));
    for (i, (n, fan_in)) in graph.nodes_by_fan_in().iter().enumerate() {
        let fan_out = outgoing.get(n.id.as_str()).map(|v| v.len()).unwrap_or(0);
        s.push_str(&format!(
            "[node {}/{}] {}\n",
            i + 1,
            graph.nodes.len(),
            n.id
        ));
        s.push_str(&format!("  owner: {} name: {}\n", n.owner, n.name));
        s.push_str(&format!(
            "  fork: {} private: {} upstream: {} ahead_by: {}\n",
            n.fork,
            n.private,
            n.upstream.as_deref().unwrap_or("-"),
            n.ahead_by.map(|v| v.to_string()).unwrap_or("-".into()),
        ));
        s.push_str(&format!("  branches: [{}]\n", n.branches.join(", ")));
        s.push_str(&format!(
            "  about: desc={} lang={} topics=[{}] home={}\n",
            n.description.as_deref().unwrap_or("-"),
            n.language.as_deref().unwrap_or("-"),
            n.topics.join(", "),
            n.homepage.as_deref().unwrap_or("-"),
        ));
        s.push_str(&format!(
            "  signals: stars={} forks={} open_issues={} license={} archived={} disabled={} size_kb={}\n",
            n.stars,
            n.forks_count,
            n.open_issues,
            n.license.as_deref().unwrap_or("-"),
            n.archived,
            n.disabled,
            n.size_kb,
        ));
        s.push_str(&format!(
            "  surfaces: readme={} llms_txt={} agents_md={} license_file={}\n",
            n.has_readme, n.has_llms_txt, n.has_agents_md, n.has_license_file,
        ));
        s.push_str(&format!(
            "  visible: {} ({})\n",
            n.visible,
            n.visible_reason.as_deref().unwrap_or("-"),
        ));
        s.push_str(&format!(
            "  pushed_at: {}\n",
            n.pushed_at.map(|t| t.to_rfc3339()).unwrap_or("-".into()),
        ));
        s.push_str(&format!("  fan-in: {fan_in} fan-out: {fan_out}\n"));
        match incoming.get(n.id.as_str()) {
            Some(v) if !v.is_empty() => {
                s.push_str("  in:\n");
                for e in v {
                    s.push_str(&format!(
                        "    {} -> {} [{}] branches=[{}] version={} recency={}\n",
                        e.source,
                        e.target,
                        format!("{:?}", e.kind).to_lowercase(),
                        e.declaring_branches.join(", "),
                        e.version.as_deref().unwrap_or("-"),
                        e.recency.map(|t| t.to_rfc3339()).unwrap_or("-".into()),
                    ));
                }
            }
            _ => s.push_str("  in: (none)\n"),
        }
        match outgoing.get(n.id.as_str()) {
            Some(v) if !v.is_empty() => {
                s.push_str("  out:\n");
                for e in v {
                    s.push_str(&format!(
                        "    {} -> {} [{}] branches=[{}] version={} recency={}\n",
                        e.source,
                        e.target,
                        format!("{:?}", e.kind).to_lowercase(),
                        e.declaring_branches.join(", "),
                        e.version.as_deref().unwrap_or("-"),
                        e.recency.map(|t| t.to_rfc3339()).unwrap_or("-".into()),
                    ));
                }
            }
            _ => s.push_str("  out: (none)\n"),
        }
    }
    // Full edge list: no truncation, sorted for determinism.
    let mut edges = graph.edges.clone();
    edges.sort_by(|a, b| (&a.source, &a.target, a.kind).cmp(&(&b.source, &b.target, b.kind)));
    s.push_str(&format!("--- edges ({}, all) ---\n", edges.len()));
    for (i, e) in edges.iter().enumerate() {
        s.push_str(&format!(
            "[edge {}/{}] {} -> {} [{}]\n",
            i + 1,
            edges.len(),
            e.source,
            e.target,
            format!("{:?}", e.kind).to_lowercase(),
        ));
        s.push_str(&format!(
            "  declaring_branches: [{}]\n",
            e.declaring_branches.join(", ")
        ));
        s.push_str(&format!(
            "  recency: {}\n",
            e.recency.map(|t| t.to_rfc3339()).unwrap_or("-".into()),
        ));
        s.push_str(&format!(
            "  version: {}\n",
            e.version.as_deref().unwrap_or("-")
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
            "  \"{}\" -> \"{}\" [label=\"{}\"];\n",
            e.source, e.target, kind
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
                pushed_at: None,
                description: Some("demo a".into()),
                topics: vec!["rust".into()],
                language: Some("Rust".into()),
                homepage: None,
                stars: 12,
                forks_count: 2,
                open_issues: 3,
                license: Some("MIT".into()),
                archived: false,
                disabled: false,
                size_kb: 1234,
                has_readme: true,
                has_llms_txt: true,
                has_agents_md: false,
                has_license_file: true,
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
            version: Some("1.0.0".into()),
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
    fn dump_and_dot_contain_ids() {
        let g = fixture();
        let dump = render_dump(&g);
        assert!(dump.contains("me/a"));
        // Full-detail markers: no truncation, all fields present.
        assert!(dump.contains("pushed_at:"));
        assert!(dump.contains("fan-in:"));
        assert!(dump.contains("[edge 1/1]"));
        let dot = render_dot(&g);
        assert!(dot.contains("digraph") && dot.contains("me/a"));
    }
}
