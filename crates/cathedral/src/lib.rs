//! Cathedral renderer for `jzombie-graph` JSON.
//!
//! Reads the collector's schema via `Graph::from_json` (schema-checked, so
//! old files fail loudly instead of rendering stale shapes) and renders one
//! thing: the load-bearing core as a symmetric arch — crown, spire, fanning
//! courses, convergence, bedrock banner. Every stone is a link; labels are
//! abbreviated, tooltips carry the full id.
//!
//! Forks are excluded by design: no fork node, edge, or name appears
//! anywhere. Only dependence edges (`manifest`/`submodule`/`registry`)
//! count toward structure. The drawn tree is the support cone of the top
//! application by fan-out; bearings that would cross the drawing are
//! named, not drawn.

use jzombie_graph::graph::{Graph, Layer};
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

    // Origin-only dependence map.
    let mut depends: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for e in &graph.edges {
        if e.kind.is_dependence() && origin(e.source.as_str()) && origin(e.target.as_str()) {
            depends
                .entry(e.source.as_str())
                .or_default()
                .push(e.target.as_str());
        }
    }
    for v in depends.values_mut() {
        v.sort();
        v.dedup();
    }

    let mut s = String::new();
    s.push_str("# Cathedral\n\n");
    let _ = writeln!(
        s,
        "Generated from `graph.json` (schema v{}). Forks excluded by design.\n",
        graph.schema_version,
    );

    // Apex: the core application with the most bearings on it.
    let core = main_component(graph, &depends);
    let in_tree = |id: &str| -> bool { core.contains(id) };
    let mut apps: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| n.layer == Layer::Application && in_tree(n.id.as_str()))
        .map(|n| n.id.as_str())
        .collect();
    apps.sort_by(|a, b| {
        let fa = depends.get(a).map(Vec::len).unwrap_or(0);
        let fb = depends.get(b).map(Vec::len).unwrap_or(0);
        fb.cmp(&fa).then_with(|| a.cmp(b))
    });
    let Some(apex) = apps.first().copied() else {
        s.push_str("(no load-bearing structure)\n");
        return s;
    };

    // Support cone of the apex, rows by depth.
    let mut rows: Vec<Vec<&str>> = vec![vec![apex]];
    let mut placed: BTreeSet<&str> = BTreeSet::from([apex]);
    loop {
        let mut next: Vec<&str> = rows
            .last()
            .map(|r| r.as_slice())
            .unwrap_or(&[])
            .iter()
            .flat_map(|src| depends.get(*src).map(Vec::as_slice).unwrap_or(&[]))
            .copied()
            .filter(|t| in_tree(t) && !placed.contains(t))
            .collect();
        next.sort_by(|a, b| {
            fan_in
                .get(*b)
                .copied()
                .unwrap_or(0)
                .cmp(&fan_in.get(*a).copied().unwrap_or(0))
                .then_with(|| a.cmp(b))
        });
        next.dedup();
        if next.is_empty() {
            break;
        }
        for n in &next {
            placed.insert(n);
        }
        rows.push(next);
    }
    let mut rows = rows;
    for r in rows.iter_mut().skip(1) {
        order_row(r, &depends);
    }

    let labels = abbreviations(graph, &|id| origin(id));
    s.push_str("\n## Elevation\n\n");
    s.push_str(
        "One arch over the load-bearing core: crown, spire, fanning courses, convergence, bedrock. ",
    );
    s.push_str("Every stone is a link; labels abbreviate. Tallies count the full graph.\n\n");
    s.push_str(&elevation(graph, &rows, &depends, &labels));
    s.push_str(&index(&labels, &placed));

    // Everything load-bearing but off-cone, and everything detached:
    // named with links, not drawn.
    let mut off: Vec<&str> = core
        .iter()
        .filter(|id| !placed.contains(id.as_str()))
        .map(String::as_str)
        .collect();
    off.sort_by_key(|id| short(id).to_string());
    if !off.is_empty() {
        s.push_str("\nAlso bearing, outside this elevation: ");
        for (i, id) in off.iter().enumerate() {
            if i > 0 {
                s.push_str(" · ");
            }
            let lab = labels.get(id).map(String::as_str).unwrap_or(id);
            let _ = write!(s, "[{lab}]({url})", url = url_of(id));
        }
        s.push_str(".\n");
    }
    let mut detached: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| !n.fork && n.layer != Layer::Isolated && !core.contains(n.id.as_str()))
        .map(|n| n.id.as_str())
        .collect();
    detached.sort();
    if !detached.is_empty() {
        s.push_str("\nDetached from the load-bearing core: ");
        for (i, id) in detached.iter().enumerate() {
            if i > 0 {
                s.push_str(" · ");
            }
            let lab = labels.get(id).map(String::as_str).unwrap_or(short(id));
            let _ = write!(s, "[{lab}]({url})", url = url_of(id));
        }
        s.push_str(".\n");
    }
    s
}

/// Order each row's boxes to maximize drawn same-row bearings
/// (brute force; rows are tiny). Deterministic.
fn order_row(row: &mut Vec<&str>, depends: &BTreeMap<&str, Vec<&str>>) {
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

fn adjacencies(row: &[&str], depends: &BTreeMap<&str, Vec<&str>>) -> usize {
    let pos: BTreeMap<&str, usize> = row.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    depends
        .iter()
        .filter(|(s, _)| pos.contains_key(*s))
        .flat_map(|(_, tgts)| tgts.iter())
        .filter(|t| pos.contains_key(*t))
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

/// The arch: crown, spire, fanning courses with bus bearings,
/// same-row bearings, convergence, bedrock banner. Node labels are
/// links; geometry is computed on visible text so alignment holds.
fn elevation(
    graph: &Graph,
    rows: &[Vec<&str>],
    depends: &BTreeMap<&str, Vec<&str>>,
    labels: &BTreeMap<&str, String>,
) -> String {
    let lab =
        |id: &str| -> String { format!("[{}]", labels.get(id).map(String::as_str).unwrap_or(id)) };
    // Box widths from visible text.
    let widths: Vec<Vec<usize>> = rows
        .iter()
        .map(|r| r.iter().map(|id| lab(id).len()).collect())
        .collect();
    let row_w: Vec<usize> = widths
        .iter()
        .map(|w| w.iter().sum::<usize>() + 2 * w.len().saturating_sub(1))
        .collect();
    let canvas_w = row_w.iter().copied().max().unwrap_or(0);

    // Rows at 0, 3, 6, ...: box row, stub row, bus row.
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
    let at_row = |id: &str| -> Option<usize> { rows.iter().position(|r| r.contains(&id)) };
    // Cross-row bearings between adjacent rows.
    for (src, tgts) in depends {
        let Some(si) = at_row(src) else {
            continue;
        };
        for tgt in tgts {
            let Some(ti) = at_row(tgt) else {
                continue;
            };
            if ti != si + 1 {
                continue;
            }
            let (Some(&sx), Some(&tx)) = (center.get(src), center.get(tgt)) else {
                continue;
            };
            put(&mut canvas[si * 3 + 1], sx, '|');
            put(&mut canvas[si * 3 + 1], tx, '|');
            if sx == tx {
                put(&mut canvas[si * 3 + 2], sx, '|');
            } else {
                let (lo, hi) = (sx.min(tx), sx.max(tx));
                for x in lo..=hi {
                    put(&mut canvas[si * 3 + 2], x, '-');
                }
                put(&mut canvas[si * 3 + 2], sx, '+');
                put(&mut canvas[si * 3 + 2], tx, '+');
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
            for tgt in tgts {
                if let Some(j) = row.iter().position(|o| o == tgt) {
                    if j == i + 1 {
                        let gx = starts[i] + widths[li][i];
                        put(&mut canvas[r], gx, '-');
                        put(&mut canvas[r], gx + 1, '-');
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
    // Convergence into the bedrock banner.
    let (first, last) = (0usize, canvas_w.saturating_sub(1));
    let mut conv: Vec<char> = vec![' '; canvas_w.max(1)];
    if canvas_w > 1 {
        put(&mut conv, first, '\\');
        put(&mut conv, last, '/');
        for x in (first + 1)..last {
            if x != canvas_w / 2 - 2 && x != canvas_w / 2 + 2 {
                put(&mut conv, x, '-');
            }
        }
        put(&mut conv, canvas_w / 2 - 2, '+');
        put(&mut conv, canvas_w / 2 + 2, '+');
    }
    d.push_str(&conv.iter().collect::<String>().trim_end().to_string());
    d.push('\n');
    let mut drops: Vec<char> = vec![' '; canvas_w.max(1)];
    if canvas_w > 1 {
        put(&mut drops, canvas_w / 2 - 2, 'v');
        put(&mut drops, canvas_w / 2 + 2, 'v');
    }
    d.push_str(&drops.iter().collect::<String>().trim_end().to_string());
    d.push('\n');
    // Bedrock banner names every foundation stone in the drawn cone.
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
    d.push_str(&bar);
    d.push('\n');
    d.push_str("</pre>\n");
    d
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

/// Combine connector strokes: crossings become `+`.
fn put(line: &mut [char], x: usize, c: char) {
    if x >= line.len() {
        return;
    }
    line[x] = match (line[x], c) {
        (' ', c) => c,
        ('-', '|') | ('|', '-') | ('-', '+') | ('+', '-') | ('|', '+') | ('+', '|') => '+',
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

/// Largest connected component (undirected) over origin-only dependence
/// edges. Largest wins; ties break on smallest member id.
fn main_component(graph: &Graph, depends: &BTreeMap<&str, Vec<&str>>) -> BTreeSet<String> {
    let mut adj: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (src, tgts) in depends {
        for tgt in tgts {
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
    let mut best: BTreeSet<String> = BTreeSet::new();
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
                    if !seen.contains(nb) {
                        seen.insert(nb);
                        stack.push(nb);
                    }
                }
            }
        }
        let score = |c: &BTreeSet<String>| (c.len() > 1, c.len());
        let better = score(&comp) > score(&best)
            || (score(&comp) == score(&best) && comp.iter().next() < best.iter().next());
        if better {
            best = comp;
        }
    }
    best
}

/// Clickable index: every drawn label mapped to its repo. The `<pre>`
/// above is the graphic; this is the links.
fn index(labels: &BTreeMap<&str, String>, drawn: &BTreeSet<&str>) -> String {
    let mut rows: Vec<(&str, &str)> = drawn
        .iter()
        .map(|id| (labels.get(*id).map(String::as_str).unwrap_or(*id), *id))
        .collect();
    rows.sort();
    rows.dedup();
    let mut s = String::from("\nIndex: ");
    for (i, (lab, id)) in rows.iter().enumerate() {
        if i > 0 {
            s.push_str(" · ");
        }
        let _ = write!(s, "[{lab}]({url})", url = url_of(id));
    }
    s.push_str("\n");
    s
}

/// GitHub URL for a portfolio id.
fn url_of(id: &str) -> String {
    format!("https://github.com/{id}")
}

/// Short label: repo name without the owner.
fn short(id: &str) -> &str {
    id.split('/').nth(1).unwrap_or(id)
}

/// Abbreviated display labels: owner stripped, one leading `rust-`
/// stripped, collisions fall back to the full name.
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
