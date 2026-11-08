//! Cathedral renderer for `jzombie-graph` JSON.
//!
//! Reads the collector's schema via `Graph::from_json` (schema-checked, so
//! old files fail loudly instead of rendering stale shapes) and renders:
//!
//! - an arch: crown, spire courses, bus bearings, same-row bearings,
//!   convergence, bedrock banner naming the drawn foundation stones plus
//!   a crypt row of load-bearing forks. Box-drawing characters keep the
//!   geometry; every stone is a link, labels are abbreviated;
//! - an elevation table where every stone is a real Markdown link, with
//!   fan-in and versioned bearings;
//! - side chapels: the docs scriptorium, detached islands, lone stones.
//!
//! Forks appear only where they bear load: the crypt row and the crypt
//! table rows — nowhere else. Only dependence edges
//! (`manifest`/`submodule`/`registry`) count toward structure.

use jzombie_graph::graph::{EdgeKind, Graph, Layer};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// Render the cathedral doc for a collected graph.
pub fn render_cathedral(graph: &Graph) -> String {
    let fan_in = graph.node_fan_in();
    let is_fork: BTreeMap<&str, bool> = graph
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.fork))
        .collect();
    let origin = |id: &str| -> bool { !is_fork.get(id).copied().unwrap_or(false) };

    // Origin-only dependence map (with pinned versions for the table).
    let mut depends: BTreeMap<&str, Vec<(&str, Option<&str>)>> = BTreeMap::new();
    for e in &graph.edges {
        if e.kind.is_dependence() && origin(e.source.as_str()) && origin(e.target.as_str()) {
            depends
                .entry(e.source.as_str())
                .or_default()
                .push((e.target.as_str(), e.version.as_deref()));
        }
    }
    for v in depends.values_mut() {
        v.sort();
        v.dedup();
    }

    // Main connected component: the load-bearing core.
    let comps = components(graph, &depends);
    let core: &BTreeSet<String> = match comps.first() {
        Some(c) if c.iter().any(|id| graph.nodes.iter().any(|n| &n.id == id)) => c,
        _ => return no_structure(graph),
    };
    let in_core = |id: &str| -> bool { core.contains(id) };

    // Courses: core stones grouped by stored layer, fan-in desc, id asc.
    let mut courses: Vec<Vec<&str>> = Vec::new();
    for layer in [Layer::Application, Layer::Middleware, Layer::Foundation] {
        let mut members: Vec<&str> = graph
            .nodes
            .iter()
            .filter(|n| n.layer == layer && origin(n.id.as_str()) && in_core(n.id.as_str()))
            .map(|n| n.id.as_str())
            .collect();
        members.sort_by(|a, b| {
            fan_in
                .get(*b)
                .copied()
                .unwrap_or(0)
                .cmp(&fan_in.get(*a).copied().unwrap_or(0))
                .then_with(|| a.cmp(b))
        });
        if !members.is_empty() {
            courses.push(members);
        }
    }
    if courses.is_empty() {
        return no_structure(graph);
    }

    let labels = abbreviations(graph, &|_| true);
    let mut s = String::new();
    s.push_str("# The Zenosmosis Cathedral\n\n");
    let _ = writeln!(
        s,
        "Generated from `graph.json` (schema v{}). Forks bear load only in the crypt.\n",
        graph.schema_version,
    );
    s.push_str(&elevation(graph, &courses, &depends, &labels));
    s.push_str(&elevation_table(
        graph, &courses, &depends, &labels, &fan_in,
    ));
    s.push_str(&chapels(graph, &depends, &labels, &core));
    s
}

fn no_structure(graph: &Graph) -> String {
    let mut s = String::new();
    s.push_str("# The Zenosmosis Cathedral\n\n");
    let _ = writeln!(
        s,
        "Generated from `graph.json` (schema v{}). Forks bear load only in the crypt.\n",
        graph.schema_version,
    );
    s.push_str("(no load-bearing structure)\n");
    s
}

/// All connected components (undirected) over origin-only dependence
/// edges, largest first: multi-node wins, then size, then smallest id.
fn components(
    graph: &Graph,
    depends: &BTreeMap<&str, Vec<(&str, Option<&str>)>>,
) -> Vec<BTreeSet<String>> {
    let mut adj: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (src, tgts) in depends {
        for (tgt, _) in tgts {
            adj.entry(src).or_default().insert(tgt);
            adj.entry(tgt).or_default().insert(src);
        }
    }
    for n in &graph.nodes {
        if !n.fork {
            adj.entry(n.id.as_str()).or_default();
        }
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut comps: Vec<BTreeSet<String>> = Vec::new();
    for start in adj.keys().copied().collect::<Vec<_>>() {
        if !seen.insert(start) {
            continue;
        }
        let mut comp: BTreeSet<String> = BTreeSet::new();
        let mut stack = vec![start];
        while let Some(cur) = stack.pop() {
            if !comp.insert(cur.to_string()) {
                continue;
            }
            if let Some(next) = adj.get(cur) {
                for nb in next {
                    if seen.insert(nb) {
                        stack.push(nb);
                    }
                }
            }
        }
        comps.push(comp);
    }
    comps.sort_by(|a, b| {
        (b.len() > 1, b.len(), std::cmp::Reverse(b.iter().next())).cmp(&(
            a.len() > 1,
            a.len(),
            std::cmp::Reverse(a.iter().next()),
        ))
    });
    comps
}

/// Order each row's boxes to maximize drawn same-row bearings
/// (brute force; rows are tiny). Deterministic.
fn order_row(row: &mut Vec<&str>, depends: &BTreeMap<&str, Vec<(&str, Option<&str>)>>) {
    if row.len() < 3 {
        return;
    }
    let mut best = row.clone();
    let mut best_n = adjacencies(&best, depends);
    permute(row, 0, &mut |p| {
        let n = adjacencies(p, depends);
        if n > best_n {
            best_n = n;
            best = p.to_vec();
        }
    });
    *row = best;
}

fn adjacencies(row: &[&str], depends: &BTreeMap<&str, Vec<(&str, Option<&str>)>>) -> usize {
    let pos: BTreeMap<&str, usize> = row.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    depends
        .iter()
        .filter(|(s, _)| pos.contains_key(*s))
        .flat_map(|(_, tgts)| tgts.iter())
        .filter(|(t, _)| pos.contains_key(*t))
        .count()
}

fn permute<T: Clone>(xs: &mut Vec<T>, i: usize, f: &mut impl FnMut(&[T])) {
    if i >= xs.len() {
        f(xs);
        return;
    }
    for j in i..xs.len() {
        xs.swap(i, j);
        permute(xs, i + 1, f);
        xs.swap(i, j);
    }
}

/// The arch: crown, spire courses with box-drawing bearings, convergence,
/// bedrock banner plus the crypt of load-bearing forks. Box labels are
/// links; geometry is computed on visible text so alignment holds.
fn elevation(
    graph: &Graph,
    courses: &[Vec<&str>],
    depends: &BTreeMap<&str, Vec<(&str, Option<&str>)>>,
    labels: &BTreeMap<&str, String>,
) -> String {
    let lab =
        |id: &str| -> String { format!("[{}]", labels.get(id).map(String::as_str).unwrap_or(id)) };
    // One centered row per course, ordered for adjacency.
    let mut rows: Vec<Vec<&str>> = Vec::new();
    for course in courses {
        let mut ordered = course.to_vec();
        order_row(&mut ordered, depends);
        rows.push(ordered);
    }
    let widths: Vec<Vec<usize>> = rows
        .iter()
        .map(|r| r.iter().map(|id| lab(id).len()).collect())
        .collect();
    let row_w: Vec<usize> = widths
        .iter()
        .map(|w| w.iter().sum::<usize>() + 2 * w.len().saturating_sub(1))
        .collect();
    let canvas_w = row_w.iter().copied().max().unwrap_or(0).max(3);
    // Box row, stub row, bus row per course row.
    let mut canvas: Vec<Vec<char>> = vec![vec![' '; canvas_w]; rows.len() * 3 - 1];
    // (row, x) -> url for link spans, keyed by visible span.
    let mut links: BTreeMap<(usize, usize, usize), String> = BTreeMap::new();
    let mut center: BTreeMap<&str, usize> = BTreeMap::new();
    for (li, row) in rows.iter().enumerate() {
        let r = li * 3;
        let offset = (canvas_w - row_w[li]) / 2;
        let mut x = offset;
        for (i, id) in row.iter().enumerate() {
            if i > 0 {
                x += 2;
            }
            let l = lab(id);
            for (j, c) in l.chars().enumerate() {
                canvas[r][x + j] = c;
            }
            // Link span covers the label text between the brackets.
            links.insert((r, x + 1, x + l.len() - 1), url_of(id));
            center.insert(*id, x + l.len() / 2);
            x += l.len();
        }
    }
    // Stub + bus between consecutive courses. Bus spans only the boxes
    // that actually bear on the next course.
    for (li, row) in rows.iter().enumerate() {
        if li + 1 >= rows.len() {
            break;
        }
        let mut involved: Vec<usize> = Vec::new();
        for id in row {
            if let Some(tgts) = depends.get(id) {
                for (tgt, _) in tgts {
                    if rows[li + 1].contains(tgt) {
                        if let (Some(&sx), Some(&tx)) = (center.get(*id), center.get(*tgt)) {
                            put(&mut canvas[li * 3 + 1], sx, '│');
                            put(&mut canvas[li * 3 + 1], tx, '│');
                            involved.push(sx);
                            involved.push(tx);
                        }
                    }
                }
            }
        }
        if !involved.is_empty() {
            involved.sort();
            involved.dedup();
            let (mn, mx) = (involved[0], involved[involved.len() - 1]);
            for x in mn..=mx {
                put(&mut canvas[li * 3 + 2], x, '─');
            }
            for x in involved {
                put(&mut canvas[li * 3 + 2], x, '┼');
            }
        }
    }
    // Same-row bearings between adjacent boxes.
    for (li, row) in rows.iter().enumerate() {
        let r = li * 3;
        let offset = (canvas_w - row_w[li]) / 2;
        let mut starts: Vec<usize> = Vec::new();
        let mut x = offset;
        for (i, _id) in row.iter().enumerate() {
            if i > 0 {
                x += 2;
            }
            starts.push(x);
            x += widths[li][i];
        }
        for (i, id) in row.iter().enumerate() {
            let Some(tgts) = depends.get(id) else {
                continue;
            };
            for (tgt, _) in tgts {
                if let Some(j) = row.iter().position(|o| o == tgt) {
                    if j == i + 1 {
                        let gx = starts[i] + widths[li][i];
                        put(&mut canvas[r], gx, '─');
                        put(&mut canvas[r], gx + 1, '─');
                    }
                }
            }
        }
    }

    let mut d = String::from("<pre>\n");
    let crown_w = canvas_w.max(3);
    d.push_str(&centered("▲", crown_w));
    d.push('\n');
    d.push_str(&centered("/ \\", crown_w));
    d.push('\n');
    for (r, line) in canvas.iter().enumerate() {
        let text: String = line.iter().collect();
        d.push_str(&apply_links(text.trim_end(), r, &links));
        d.push('\n');
    }
    // Convergence into the bedrock banner, guide-style: slanted
    // shoulders running in to two drops.
    let mut conv: Vec<char> = vec![' '; canvas_w.max(1)];
    if canvas_w > 12 {
        let (lc, rc) = (canvas_w / 3, 2 * canvas_w / 3);
        put(&mut conv, 0, '\\');
        for x in 1..lc {
            put(&mut conv, x, '─');
        }
        put(&mut conv, lc, '┐');
        put(&mut conv, rc, '┌');
        for x in (rc + 1)..canvas_w.saturating_sub(1) {
            put(&mut conv, x, '─');
        }
        put(&mut conv, canvas_w - 1, '/');
        let mut drops: Vec<char> = vec![' '; canvas_w.max(1)];
        put(&mut drops, lc, '▼');
        put(&mut drops, rc, '▼');
        d.push_str(&conv.iter().collect::<String>().trim_end().to_string());
        d.push('\n');
        d.push_str(&drops.iter().collect::<String>().trim_end().to_string());
        d.push('\n');
    } else if canvas_w > 1 {
        let mut drops: Vec<char> = vec![' '; canvas_w.max(1)];
        put(&mut drops, canvas_w / 2, '▼');
        d.push_str(&drops.iter().collect::<String>().trim_end().to_string());
        d.push('\n');
    }
    // Bedrock banner names every drawn foundation stone.
    let mut names: Vec<String> = rows
        .iter()
        .flat_map(|r| r.iter())
        .filter(|id| {
            graph
                .nodes
                .iter()
                .find(|n| n.id == **id)
                .is_some_and(|n| n.layer == Layer::Foundation)
        })
        .map(|id| {
            labels
                .get(*id)
                .map(String::as_str)
                .unwrap_or(*id)
                .to_string()
        })
        .collect();
    names.sort();
    names.dedup();
    // Crypt: load-bearing forks with their incoming counts.
    let mut crypt: Vec<(String, u32)> = graph
        .nodes
        .iter()
        .filter(|n| n.fork)
        .map(|n| {
            (
                n.id.as_str(),
                graph.node_fan_in().get(&n.id).copied().unwrap_or(0),
            )
        })
        .filter(|(_, c)| *c > 0)
        .map(|(id, c)| {
            (
                labels.get(id).map(String::as_str).unwrap_or(id).to_string(),
                c,
            )
        })
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect();
    crypt.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let banner = if names.is_empty() {
        "FOUNDATION".to_string()
    } else {
        format!("FOUNDATION: {}", names.join(" & "))
    };
    let bar = "=".repeat(banner.len().max(canvas_w));
    d.push_str(&bar);
    d.push('\n');
    d.push_str(&banner);
    d.push('\n');
    if !crypt.is_empty() {
        let line = format!(
            "CRYPT (LOAD-BEARING FORKS): {}",
            crypt
                .iter()
                .map(|(n, c)| format!("{n} ({c} incoming)"))
                .collect::<Vec<_>>()
                .join("  ·  ")
        );
        d.push_str(&line);
        d.push('\n');
    }
    d.push_str(&bar);
    d.push('\n');
    d.push_str("</pre>\n");
    d
}

/// Elevation table: one row per drawn stone plus one per crypt fork.
/// Level, element, node link, and load — all clickable, all minable.
fn elevation_table(
    graph: &Graph,
    courses: &[Vec<&str>],
    depends: &BTreeMap<&str, Vec<(&str, Option<&str>)>>,
    labels: &BTreeMap<&str, String>,
    fan_in: &BTreeMap<String, u32>,
) -> String {
    let course_name = |layer: &Layer| -> &'static str {
        match layer {
            Layer::Application => "**Spires**",
            Layer::Middleware => "**Vaults**",
            Layer::Foundation => "**Buttresses**",
            Layer::Isolated => "**Cloister**",
        }
    };
    let mut s = String::from("### Architectural Elevation\n\n");
    s.push_str("| Level | Element | Node | Load |\n");
    s.push_str("| --- | --- | --- | --- |\n");
    for course in courses {
        if course.is_empty() {
            continue;
        }
        let layer = graph
            .nodes
            .iter()
            .find(|n| course.first().is_some_and(|id| n.id == **id))
            .map(|n| n.layer)
            .unwrap_or(Layer::Isolated);
        for id in course {
            let node = graph.nodes.iter().find(|n| n.id == *id);
            let desc = node
                .and_then(|n| n.description.as_deref())
                .unwrap_or("")
                .trim()
                .replace('\n', " ");
            let element = if desc.is_empty() {
                "—".to_string()
            } else {
                desc
            };
            let home = node
                .and_then(|n| n.homepage.as_deref())
                .map(|h| format!(" · [docs]({h})"))
                .unwrap_or_default();
            let fi = fan_in.get(*id).copied().unwrap_or(0);
            let mut load = format!("fan-in {fi}");
            if let Some(deps) = depends.get(id) {
                if !deps.is_empty() {
                    let rest = deps
                        .iter()
                        .map(|(t, v)| {
                            let name = labels.get(t).map(String::as_str).unwrap_or(t);
                            match v {
                                Some(v) => format!("{name} ({v})"),
                                None => name.to_string(),
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    load.push_str(&format!(" · rests on {rest}"));
                }
            }
            let lab = labels.get(id).map(String::as_str).unwrap_or(id);
            let _ = writeln!(
                s,
                "| {} | {} | [`{lab}`]({url}){home} | {load} |",
                course_name(&layer),
                element,
                url = url_of(id)
            );
        }
    }
    // Crypt rows: load-bearing forks, heaviest first.
    let mut crypt: Vec<(&str, u32)> = graph
        .nodes
        .iter()
        .filter(|n| n.fork)
        .map(|n| (n.id.as_str(), fan_in.get(&n.id).copied().unwrap_or(0)))
        .filter(|(_, c)| *c > 0)
        .collect();
    crypt.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (id, c) in crypt {
        let node = graph.nodes.iter().find(|n| n.id == id);
        let desc = node
            .and_then(|n| n.description.as_deref())
            .unwrap_or("")
            .trim()
            .replace('\n', " ");
        let element = if desc.is_empty() {
            "—".to_string()
        } else {
            desc
        };
        let home = node
            .and_then(|n| n.homepage.as_deref())
            .map(|h| format!(" · [docs]({h})"))
            .unwrap_or_default();
        let lab = labels.get(id).map(String::as_str).unwrap_or(id);
        let _ = writeln!(
            s,
            "| **Crypt** | {element} | [`{lab}`]({url}){home} | fan-in {c} |",
            url = url_of(id)
        );
    }
    s.push('\n');
    s
}

/// Side chapels: the docs scriptorium, detached islands, lone stones.
/// All derived from edge kinds — no curation, no hand lists.
fn chapels(
    graph: &Graph,
    depends: &BTreeMap<&str, Vec<(&str, Option<&str>)>>,
    labels: &BTreeMap<&str, String>,
    core: &BTreeSet<String>,
) -> String {
    // Scriptorium: workflow sinks with no dependence edges, plus the
    // nodes whose workflow edges point at them.
    let mut houses: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in &graph.edges {
        if e.kind != EdgeKind::Workflow {
            continue;
        }
        let src_dep = graph
            .edges
            .iter()
            .any(|o| o.kind.is_dependence() && (o.source == e.source || o.target == e.source));
        let tgt_dep = graph
            .edges
            .iter()
            .any(|o| o.kind.is_dependence() && (o.source == e.target || o.target == e.target));
        if tgt_dep {
            continue;
        }
        let _ = src_dep;
        houses
            .entry(e.target.as_str())
            .or_default()
            .push(e.source.as_str());
    }
    for writers in houses.values_mut() {
        writers.sort();
        writers.dedup();
    }
    // Detached islands: non-core multi-node components + lone stones.
    let mut adj: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (src, tgts) in depends {
        for (tgt, _) in tgts {
            adj.entry(src).or_default().insert(*tgt);
            adj.entry(tgt).or_default().insert(*src);
        }
    }
    for n in &graph.nodes {
        if !n.fork {
            adj.entry(n.id.as_str()).or_default();
        }
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut islands: Vec<Vec<&str>> = Vec::new();
    let mut lone: Vec<&str> = Vec::new();
    for start in adj.keys().copied().collect::<Vec<_>>() {
        if !seen.insert(start) {
            continue;
        }
        let mut comp: BTreeSet<&str> = BTreeSet::new();
        let mut stack = vec![start];
        while let Some(cur) = stack.pop() {
            if !comp.insert(cur) {
                continue;
            }
            if let Some(next) = adj.get(cur) {
                for nb in next {
                    if seen.insert(nb) {
                        stack.push(nb);
                    }
                }
            }
        }
        if comp.len() == 1 {
            let only = comp.iter().next().unwrap();
            let layered = graph
                .nodes
                .iter()
                .find(|n| n.id == *only)
                .is_some_and(|n| n.layer != Layer::Isolated);
            if layered && !core.contains(*only) {
                lone.push(only);
            }
            continue;
        }
        let mut members: Vec<&str> = comp.into_iter().collect();
        members.sort();
        islands.push(members);
    }
    // Drop the main component — it is the drawing; keep the rest.
    islands.retain(|c| !c.iter().any(|id| core.contains(*id)));
    islands.sort();
    lone.sort();
    if houses.is_empty() && islands.is_empty() && lone.is_empty() {
        return String::new();
    }
    let lab = |id: &str| -> String {
        labels
            .get(id)
            .map(String::as_str)
            .unwrap_or(short(id))
            .to_string()
    };
    let mut s = String::from("### Side Chapels & Workshops\n\n");
    for (house, writers) in &houses {
        let _ = writeln!(
            s,
            "* **The Documentation Scriptorium:** [`{}`]({})",
            lab(house),
            url_of(house)
        );
        let _ = writeln!(
            s,
            "*Workflow ink from:* {}",
            writers
                .iter()
                .map(|w| format!("[`{}`]({})", lab(w), url_of(w)))
                .collect::<Vec<_>>()
                .join(" · ")
        );
    }
    for isl in &islands {
        // Chain display: root = member nothing in-island depends on.
        let roots: Vec<&&str> = isl
            .iter()
            .filter(|id| {
                !depends
                    .iter()
                    .any(|(src, tgts)| isl.contains(src) && tgts.iter().any(|(t, _)| t == *id))
            })
            .collect();
        let chain: Vec<&str> = if roots.is_empty() {
            isl.clone()
        } else {
            let mut out: Vec<&str> = vec![roots[0]];
            let mut cur = roots[0];
            while let Some(next) = depends.get(cur).and_then(|v| v.first()).map(|(t, _)| t) {
                if !isl.contains(next) || out.contains(next) {
                    break;
                }
                out.push(next);
                cur = next;
            }
            out
        };
        let _ = writeln!(
            s,
            "* **Detached chapel:** {}",
            chain
                .iter()
                .map(|id| format!("[`{}`]({})", lab(id), url_of(id)))
                .collect::<Vec<_>>()
                .join(" → ")
        );
    }
    if !lone.is_empty() {
        s.push_str("* **Lone stones:** ");
        s.push_str(
            &lone
                .iter()
                .map(|id| format!("[`{}`]({})", lab(id), url_of(id)))
                .collect::<Vec<_>>()
                .join(" · "),
        );
        s.push('\n');
    }
    s.push('\n');
    s
}

/// Splice `<a>` links into one canvas row at the recorded visible spans.
/// Geometry was computed on visible text, so tags (zero-width) keep alignment.
fn apply_links(text: &str, row: usize, links: &BTreeMap<(usize, usize, usize), String>) -> String {
    let mut spans: Vec<(usize, usize, &String)> = links
        .iter()
        .filter(|((r, _, _), _)| *r == row)
        .map(|((_, s, e), u)| (*s, *e, u))
        .collect();
    spans.sort();
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut x = 0;
    for (s, e, url) in spans {
        for c in chars.iter().take(s.min(chars.len())).skip(x) {
            push_esc(&mut out, *c);
        }
        out.push_str(&format!("<a href=\"{url}\">", url = esc_attr(url)));
        for c in chars.iter().take(e.min(chars.len())).skip(s) {
            push_esc(&mut out, *c);
        }
        out.push_str("</a>");
        x = e;
    }
    for c in chars.iter().skip(x) {
        push_esc(&mut out, *c);
    }
    out
}

fn push_esc(s: &mut String, c: char) {
    match c {
        '&' => s.push_str("&amp;"),
        '<' => s.push_str("&lt;"),
        '>' => s.push_str("&gt;"),
        _ => s.push(c),
    }
}

fn esc_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

/// Combine connector strokes: crossings become `+`; box-drawing passes through.
fn put(line: &mut [char], x: usize, c: char) {
    if x >= line.len() {
        return;
    }
    line[x] = match (line[x], c) {
        (' ', c) => c,
        ('-', '|') | ('|', '-') | ('-', '+') | ('+', '-') | ('|', '+') | ('+', '|') => '+',
        ('─', '│') | ('│', '─') | ('─', '┼') | ('┼', '─') | ('│', '┼') | ('┼', '│') => {
            '┼'
        }
        ('─', '┌') | ('┌', '─') | ('─', '┐') | ('┐', '─') => c,
        (cur, _) => cur,
    };
}

fn centered(text: &str, width: usize) -> String {
    if text.len() >= width {
        return text.to_string();
    }
    let pad = (width - text.len()) / 2;
    format!("{}{}", " ".repeat(pad), text)
}

fn abbreviations<'a>(
    graph: &'a Graph,
    in_drawn: &dyn Fn(&str) -> bool,
) -> BTreeMap<&'a str, String> {
    let ids: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| in_drawn(n.id.as_str()))
        .map(|n| n.id.as_str())
        .collect();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for id in &ids {
        *counts.entry(candidate(id)).or_default() += 1;
    }
    ids.into_iter()
        .map(|id| {
            let c = candidate(id);
            let label = if counts.get(&c).copied().unwrap_or(0) > 1 {
                short(id).to_string()
            } else {
                c
            };
            (id, label)
        })
        .collect()
}

fn candidate(id: &str) -> String {
    short(id)
        .strip_prefix("rust-")
        .unwrap_or(short(id))
        .to_string()
}

/// GitHub URL for a portfolio id.
fn url_of(id: &str) -> String {
    format!("https://github.com/{id}")
}

/// Short label: repo name without the owner.
fn short(id: &str) -> &str {
    id.split('/').nth(1).unwrap_or(id)
}
