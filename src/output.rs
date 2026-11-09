use crate::graph::Graph;
use anyhow::Context;

/// JSON-first: stable schema, jq-queryable from day one.
/// Documented recipes (README): who-depends-on-X, orphans,
/// most depended-upon, what-changed (diff yesterday vs today).
pub fn render_json(graph: &Graph) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(graph)?)
}

/// One write path for file assets: creates missing parent directories
/// (`mkdir -p` semantics — fresh checkouts never have `data/` since it's
/// gitignored), then writes the bytes verbatim.
pub fn write_file(path: &std::path::Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}
/// `--format schema`: the JSON Schema for the graph contract, derived from
/// the Rust types themselves via `schemars` — never hand-maintained, so
/// there is no second copy of the contract to drift. Static: no graph data,
/// no API, no selector.
///
/// This is the source of the committed `json/portfolio-graph-schema.json`
/// (see the drift test): whatever this returns is what ships. Trailing
/// newline included — it is a file asset, and the bytes written are the
/// bytes compared.
pub fn render_schema() -> String {
    let schema = schemars::schema_for!(Graph);
    format!(
        "{}\n",
        serde_json::to_string_pretty(&schema).expect("schema serializes")
    )
}

/// `--format focus`: the focus ranking, hottest first. Same `Node::focus`
/// formula as the dump lane — commit heat vs own 30d baseline minus one
/// point per idle month — so the number never lives in two places.
/// Offline: pure function of the stored graph + `now`.
pub fn render_focus(graph: &Graph, now: &chrono::DateTime<chrono::Utc>) -> String {
    let mut rows: Vec<(&crate::graph::Node, Option<f32>)> =
        graph.nodes.iter().map(|n| (n, n.focus(now))).collect();
    rows.sort_by(|a, b| match (&a.1, &b.1) {
        (Some(fa), Some(fb)) => fb
            .partial_cmp(fa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.id.cmp(&b.0.id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.id.cmp(&b.0.id),
    });
    let mut s = String::new();
    for (n, focus) in rows {
        s.push_str(&format!(
            "{} focus={} heat={} idle={}d 5d={} 30d={} issues={}\n",
            n.id,
            focus.map(|f| format!("{f:.2}")).unwrap_or("-".into()),
            n.commit_heat()
                .map(|h| format!("{h:.2}"))
                .unwrap_or("-".into()),
            n.idle_days(now)
                .map(|d| d.to_string())
                .unwrap_or("-".into()),
            n.commits_5d,
            n.commits_30d,
            n.open_issues,
        ));
    }
    s
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
    // Per-node adjacency for the vertical blocks (all kinds render in the
    // in/out lists). Tallies and the stored `layer` are dependence-only.
    let mut incoming: BTreeMap<&str, Vec<&crate::graph::Edge>> = BTreeMap::new();
    let mut outgoing: BTreeMap<&str, Vec<&crate::graph::Edge>> = BTreeMap::new();
    for e in &graph.edges {
        incoming.entry(e.target.as_str()).or_default().push(e);
        outgoing.entry(e.source.as_str()).or_default().push(e);
    }
    let dep_fan_in = graph.node_fan_in();
    let dep_fan_out = graph.node_fan_out();
    s.push_str(&format!(
        "--- nodes ({}, fan-in desc) ---\n",
        graph.nodes.len()
    ));
    for (i, (n, fan_in)) in graph.nodes_by_fan_in().iter().enumerate() {
        let fan_out = dep_fan_out.get(n.id.as_str()).copied().unwrap_or(0);
        let dep_in = dep_fan_in.get(n.id.as_str()).copied().unwrap_or(0);
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
        let heat: Vec<String> = n
            .branch_heat
            .iter()
            .map(|b| {
                format!(
                    "{}{}:{}",
                    b.branch,
                    if b.is_default { "*" } else { "" },
                    b.commits_30d
                )
            })
            .collect();
        s.push_str(&format!("  branches: [{}]\n", heat.join(", ")));
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
        s.push_str(&format!(
            "  fan-in: {fan_in} fan-out: {fan_out} layer: {} (dep-in: {dep_in})\n",
            n.layer
        ));
        s.push_str(&format!(
            "  focus: commits_5d={} commits_30d={} commit_heat={} idle={}d focus={} open_issues={}\n",
            n.commits_5d,
            n.commits_30d,
            n.commit_heat().map(|h| format!("{h:.2}")).unwrap_or("-".into()),
            n.idle_days(&graph.generated_at)
                .map(|d| d.to_string())
                .unwrap_or("-".into()),
            n.focus(&graph.generated_at)
                .map(|f| format!("{f:.2}"))
                .unwrap_or("-".into()),
            n.open_issues,
        ));
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
                commits_1d: 0,
                commits_5d: 0,
                commits_15d: 0,
                commits_30d: 0,
                commit_heat: None,
                focus: None,
                last_commit_at: None,
                branch_heat: vec![],
                recent_prs: vec![],
                visible: true,
                visible_reason: Some("tier3: >=1 portfolio edge".into()),
                layer: crate::graph::Layer::Isolated,
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
                commits_1d: 0,
                commits_5d: 0,
                commits_15d: 0,
                commits_30d: 0,
                commit_heat: None,
                focus: None,
                last_commit_at: None,
                branch_heat: vec![],
                visible: true,
                recent_prs: vec![],
                visible_reason: Some("tier3: >=1 portfolio edge".into()),
                layer: crate::graph::Layer::Isolated,
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

    #[test]
    fn focus_ranks_hottest_first_unknown_last() {
        use chrono::TimeZone;
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap();
        let mut g = fixture();
        // a: flat baseline, pushed now → focus 1.00.
        g.nodes[0].commits_1d = 1;
        g.nodes[0].commits_5d = 5;
        g.nodes[0].commits_15d = 15;
        g.nodes[0].commits_30d = 30;
        g.nodes[0].pushed_at = Some(now);
        // b: dormant 250 days → deep negative.
        g.nodes[1].commits_5d = 0;
        g.nodes[1].commits_30d = 0;
        g.nodes[1].pushed_at = Some(now - chrono::Duration::days(250));
        let text = render_focus(&g, &now);
        let ids: Vec<&str> = text
            .lines()
            .map(|l| l.split_whitespace().next().unwrap())
            .collect();
        assert_eq!(ids, vec!["me/a", "me/b"]);
        assert!(text.lines().next().unwrap().contains("focus=1.00"));
        // Unknown push sorts last.
        g.nodes[1].pushed_at = None;
        let text2 = render_focus(&g, &now);
        let ids2: Vec<&str> = text2
            .lines()
            .map(|l| l.split_whitespace().next().unwrap())
            .collect();
        assert_eq!(ids2, vec!["me/a", "me/b"]);
        assert!(text2.lines().last().unwrap().contains("focus=-"));
    }

    #[test]
    fn write_file_creates_missing_parents() {
        // Fresh checkouts never have `data/` (gitignored): the write path
        // must create parents itself. Regression test for the CI failure
        // where collect died with "No such file or directory" on a clean
        // runner (publish run 42516260566).
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("data").join("json").join("out.json");
        write_file(&nested, "{\"ok\":true}").unwrap();
        assert_eq!(std::fs::read_to_string(&nested).unwrap(), "{\"ok\":true}");
        // Bare filename (no parent) still writes to cwd-relative path.
        let cwd_file = dir.path().join("flat.txt");
        write_file(&cwd_file, "flat").unwrap();
        assert_eq!(std::fs::read_to_string(&cwd_file).unwrap(), "flat");
    }
}
