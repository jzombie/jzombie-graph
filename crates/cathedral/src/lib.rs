//! Markdown renderer for `jzombie-graph` JSON.
//!
//! Reads the collector's schema via `Graph::from_json` (schema-checked, so
//! old files fail loudly instead of rendering stale shapes) and writes a
//! deterministic doc grouped by the stored `Node.layer`:
//! foundation, middleware, application, then isolated under `<details>`.
//! Only dependence edges (`manifest`/`submodule`/`registry`) count toward
//! the tallies; associations render as lists, never scores.

use jzombie_graph::graph::{EdgeKind, Graph, Layer};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Render the full Markdown doc for a collected graph.
pub fn render_cathedral(graph: &Graph) -> String {
    let fan_in = graph.node_fan_in();
    let fan_out = graph.node_fan_out();

    // depends-on (dependence only), sorted for determinism.
    let mut depends_on: BTreeMap<&str, Vec<(&str, Option<&str>)>> = BTreeMap::new();
    let mut used_by: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut assoc_out: BTreeMap<&str, Vec<(&str, EdgeKind)>> = BTreeMap::new();
    for e in &graph.edges {
        if e.kind.is_dependence() {
            depends_on
                .entry(e.source.as_str())
                .or_default()
                .push((e.target.as_str(), e.version.as_deref()));
            used_by
                .entry(e.target.as_str())
                .or_default()
                .push(e.source.as_str());
        } else {
            assoc_out
                .entry(e.source.as_str())
                .or_default()
                .push((e.target.as_str(), e.kind));
        }
    }
    for v in depends_on.values_mut() {
        v.sort();
    }
    for v in used_by.values_mut() {
        v.sort();
    }
    for v in assoc_out.values_mut() {
        v.sort();
    }

    let mut s = String::new();
    let visible = graph.nodes.iter().filter(|n| n.visible).count();
    s.push_str("# Portfolio architecture\n\n");
    let _ = writeln!(
        s,
        "Generated from `graph.json` (schema v{}, {} nodes, {} visible, {} edges, generated_at={}).",
        graph.schema_version,
        graph.nodes.len(),
        visible,
        graph.edges.len(),
        graph.generated_at.to_rfc3339(),
    );

    // Summary counts by stored layer and edge kind.
    let mut layer_counts: BTreeMap<Layer, usize> = BTreeMap::new();
    for n in &graph.nodes {
        *layer_counts.entry(layer_of(n.layer)).or_default() += 1;
    }
    let mut kind_counts: BTreeMap<EdgeKind, usize> = BTreeMap::new();
    for e in &graph.edges {
        *kind_counts.entry(e.kind).or_default() += 1;
    }
    s.push_str("\n## Summary\n\n");
    s.push_str("| Layer | Nodes |\n|---|---|\n");
    for layer in [
        Layer::Foundation,
        Layer::Middleware,
        Layer::Application,
        Layer::Isolated,
    ] {
        let c = layer_counts.get(&layer).copied().unwrap_or(0);
        let _ = writeln!(s, "| {layer} | {c} |");
    }
    s.push_str("\n| Edge kind | Edges | Scores |\n|---|---|---|\n");
    for kind in [
        EdgeKind::Manifest,
        EdgeKind::Submodule,
        EdgeKind::Registry,
        EdgeKind::Workflow,
        EdgeKind::Docs,
        EdgeKind::Xrefs,
    ] {
        let c = kind_counts.get(&kind).copied().unwrap_or(0);
        if c == 0 {
            continue;
        }
        let scores = if kind.is_dependence() { "yes" } else { "no" };
        let _ = writeln!(
            s,
            "| {} | {c} | {scores} |",
            format!("{kind:?}").to_lowercase()
        );
    }

    // Layer sections, nodes ordered by fan-in desc (stable: id asc).
    let mut by_fan_in: Vec<(&jzombie_graph::graph::Node, u32)> = graph
        .nodes
        .iter()
        .map(|n| (n, fan_in.get(&n.id).copied().unwrap_or(0)))
        .collect();
    by_fan_in.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));

    for layer in [Layer::Foundation, Layer::Middleware, Layer::Application] {
        s.push_str(&format!("\n## {layer}\n\n"));
        let mut any = false;
        for (n, fi) in &by_fan_in {
            if layer_of(n.layer) != layer {
                continue;
            }
            any = true;
            let fo = fan_out.get(&n.id).copied().unwrap_or(0);
            s.push_str(&format!("### {}\n\n", linked(&n.id)));
            if let Some(d) = n.description.as_deref() {
                s.push_str(d.trim());
                s.push('\n');
            }
            s.push('\n');
            let _ = writeln!(
                s,
                "- visible: {} ({})",
                n.visible,
                n.visible_reason.as_deref().unwrap_or("-")
            );
            let _ = writeln!(s, "- dependence fan-in: {fi}, fan-out: {fo}");
            let _ = writeln!(
                s,
                "- signals: stars={} open_issues={} size_kb={} pushed_at={}",
                n.stars,
                n.open_issues,
                n.size_kb,
                n.pushed_at.map(|t| t.to_rfc3339()).unwrap_or("-".into()),
            );
            if let Some(h) = n.homepage.as_deref() {
                let _ = writeln!(s, "- homepage: <{h}>");
            }
            if !n.topics.is_empty() {
                let _ = writeln!(s, "- topics: {}", n.topics.join(", "));
            }
            match depends_on.get(n.id.as_str()) {
                Some(deps) if !deps.is_empty() => {
                    let list = deps
                        .iter()
                        .map(|(t, v)| match v {
                            Some(v) => format!("{} ({v})", linked(t)),
                            None => linked(t),
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(s, "- depends on: {list}");
                }
                _ => s.push_str("- depends on: (none)\n"),
            }
            match used_by.get(n.id.as_str()) {
                Some(users) if !users.is_empty() => {
                    let list = users
                        .iter()
                        .map(|u| linked(u))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(s, "- used by: {list}");
                }
                _ => s.push_str("- used by: (none)\n"),
            }
            match assoc_out.get(n.id.as_str()) {
                Some(assocs) if !assocs.is_empty() => {
                    let list = assocs
                        .iter()
                        .map(|(t, k)| {
                            format!("{} [{}]", linked(t), format!("{k:?}").to_lowercase())
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = writeln!(s, "- associations: {list}");
                }
                _ => {}
            }
            s.push('\n');
        }
        if !any {
            s.push_str("(none)\n");
        }
    }

    // Isolated last, collapsed so it never dominates the doc.
    s.push_str("\n## Isolated\n\n<details>\n<summary>Nodes with no dependence edges</summary>\n\n");
    let mut any_isolated = false;
    for (n, _) in &by_fan_in {
        if layer_of(n.layer) != Layer::Isolated {
            continue;
        }
        any_isolated = true;
        let _ = writeln!(
            s,
            "- {} — {}{}",
            linked(&n.id),
            if n.fork { "fork" } else { "origin" },
            match n.visible_reason.as_deref() {
                Some(r) => format!(" ({r})"),
                None => String::new(),
            }
        );
    }
    if !any_isolated {
        s.push_str("(none)\n");
    }
    s.push_str("\n</details>\n");
    s
}

/// GitHub deep link for a portfolio id (`owner/repo` are canonical).
fn linked(id: &str) -> String {
    format!("[`{id}`](https://github.com/{id})")
}

/// Copy helper: `Layer` is `Copy`, this keeps call sites readable.
fn layer_of(l: Layer) -> Layer {
    l
}
