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
    let comps = components(graph, &depends);
    let core = comps.first().cloned().unwrap_or_default();
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
    s.push_str(&elevation(graph, &rows, &depends, &labels, &placed, &core));
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
/// same-row bearings, convergence, bedrock banner — then side chapels
/// for every load-bearing node outside the drawn cone. Node labels are
/// links; geometry is computed on visible text so alignment holds.
fn elevation(
    graph: &Graph,
    rows: &[Vec<&str>],
    depends: &BTreeMap<&str, Vec<&str>>,
    labels: &BTreeMap<&str, String>,
    placed: &BTreeSet<&str>,
    core: &BTreeSet<String>,
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
    // Box rows: row 0 at canvas 0, then a one-line diagonal fan, then
    // two-line stub+bus gaps. r(li) = 0, 2, 5, 8, ...
    let row_at = |li: usize| -> usize { if li == 0 { 0 } else { li * 3 - 1 } };
    let canvas_h = if rows.is_empty() {
        0
    } else {
        row_at(rows.len() - 1) + 1
    };
    let mut canvas: Vec<Vec<char>> = vec![vec![' '; canvas_w]; canvas_h];
    // (row, x) -> url for link spans, keyed by visible span.
    let mut links: BTreeMap<(usize, usize, usize), String> = BTreeMap::new();
    let mut center: BTreeMap<&str, usize> = BTreeMap::new();
    for (li, row) in rows.iter().enumerate() {
        let r = row_at(li);
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
    // Apex fan: one diagonal line straight from the spire to its course.
    if rows.len() > 1 {
        if let Some(apex) = rows[0].first() {
            if let Some(&ax) = center.get(apex) {
                for tgt in &rows[1] {
                    if let Some(&tx) = center.get(tgt) {
                        put(
                            &mut canvas[row_at(0) + 1],
                            tx,
                            if tx < ax {
                                '/'
                            } else if tx > ax {
                                '\\'
                            } else {
                                '|'
                            },
                        );
                    }
                }
            }
        }
    }
    // Cross-row bearings between adjacent rows (bus below the fan).
    for (src, tgts) in depends {
        let Some(si) = at_row(src) else {
            continue;
        };
        if si == 0 {
            continue; // apex bearings ride the fan line
        }
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
            put(&mut canvas[row_at(si) + 1], sx, '|');
            put(&mut canvas[row_at(si) + 1], tx, '|');
            if sx == tx {
                put(&mut canvas[row_at(si) + 2], sx, '|');
            } else {
                let (lo, hi) = (sx.min(tx), sx.max(tx));
                for x in lo..=hi {
                    put(&mut canvas[row_at(si) + 2], x, '-');
                }
                put(&mut canvas[row_at(si) + 2], sx, '+');
                put(&mut canvas[row_at(si) + 2], tx, '+');
            }
        }
    }
    // Same-row bearings between adjacent boxes.
    for (li, row) in rows.iter().enumerate() {
        let r = row_at(li);
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
    // Convergence into the bedrock banner, guide-style: slanted
    // shoulders running in to two drops.
    let mut conv: Vec<char> = vec![' '; canvas_w.max(1)];
    if canvas_w > 12 {
        let (lc, rc) = (canvas_w / 3, 2 * canvas_w / 3);
        put(&mut conv, 0, '\\');
        for x in 1..lc {
            put(&mut conv, x, '-');
        }
        put(&mut conv, lc, '+');
        put(&mut conv, rc, '+');
        for x in (rc + 1)..canvas_w.saturating_sub(1) {
            put(&mut conv, x, '-');
        }
        put(&mut conv, canvas_w - 1, '/');
        let mut drops: Vec<char> = vec![' '; canvas_w.max(1)];
        put(&mut drops, lc, 'v');
        put(&mut drops, rc, 'v');
        d.push_str(&conv.iter().collect::<String>().trim_end().to_string());
        d.push('\n');
        d.push_str(&drops.iter().collect::<String>().trim_end().to_string());
        d.push('\n');
    } else if canvas_w > 1 {
        let mut drops: Vec<char> = vec![' '; canvas_w.max(1)];
        put(&mut drops, canvas_w / 2, 'v');
        d.push_str(&drops.iter().collect::<String>().trim_end().to_string());
        d.push('\n');
    }
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
    d.push_str(&chapels(graph, depends, labels, placed, core));
    d.push_str("</pre>\n");
    d
}

/// Side chapels: one small tree per load-bearing node outside the drawn
/// cone — off-cone core stacks, detached islands, lone stones. Same
/// branch glyphs as the register would use, inline links.
fn chapels(
    graph: &Graph,
    depends: &BTreeMap<&str, Vec<&str>>,
    labels: &BTreeMap<&str, String>,
    placed: &BTreeSet<&str>,
    core: &BTreeSet<String>,
) -> String {
    // Off-cone core applications root their own cones, pruned at drawn stone.
    let mut roots: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| {
            n.layer == Layer::Application
                && core.contains(n.id.as_str())
                && !placed.contains(n.id.as_str())
        })
        .map(|n| n.id.as_str())
        .collect();
    roots.sort_by(|a, b| {
        depends
            .get(b)
            .map(Vec::len)
            .unwrap_or(0)
            .cmp(&depends.get(a).map(Vec::len).unwrap_or(0))
            .then_with(|| a.cmp(b))
    });
    // Detached islands (non-core components with edges) root at the
    // members nothing else in the island bears on.
    let mut island_members: BTreeSet<&str> = BTreeSet::new();
    let mut lone: Vec<&str> = Vec::new();
    for n in &graph.nodes {
        if n.fork || core.contains(n.id.as_str()) {
            continue;
        }
        let has_edges = depends.contains_key(n.id.as_str())
            || depends.values().any(|v| v.contains(&n.id.as_str()));
        if !has_edges {
            if n.layer != Layer::Isolated {
                lone.push(n.id.as_str());
            }
            continue;
        }
        island_members.insert(n.id.as_str());
    }
    let mut island_roots: Vec<&str> = Vec::new();
    for id in island_members.iter() {
        let depended_on = depends
            .iter()
            .any(|(s, tgts)| island_members.contains(*s) && tgts.iter().any(|t| t == id));
        if !depended_on {
            island_roots.push(id);
        }
    }
    island_roots.sort();
    lone.sort();

    if roots.is_empty() && island_roots.is_empty() && lone.is_empty() {
        return String::new();
    }
    let mut d = String::from("-- side chapels --\n");
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for root in roots {
        seen.insert(root);
        let lab = labels.get(root).map(String::as_str).unwrap_or(root);
        d.push_str(&format!("[{lab}]\n", lab = linked_inline(lab, root)));
        chapel_tree(&mut d, depends, labels, placed, root, "", &mut seen);
    }
    for root in island_roots {
        seen.insert(root);
        let lab = labels.get(root).map(String::as_str).unwrap_or(root);
        d.push_str(&format!("[{lab}]\n", lab = linked_inline(lab, root)));
        chapel_tree(&mut d, depends, labels, placed, root, "", &mut seen);
    }
    for id in lone {
        let lab = labels.get(id).map(String::as_str).unwrap_or(id);
        d.push_str(&format!("[{lab}]\n", lab = linked_inline(lab, id)));
    }
    d
}

fn linked_inline(lab: &str, id: &str) -> String {
    format!("<a href=\"{url}\">{lab}</a>", url = url_of(id))
}

/// One chapel tree: supports of `id`, pruned at drawn stone with an
/// `(above)` marker, deduped within the whole elevation.
fn chapel_tree<'a>(
    s: &mut String,
    depends: &BTreeMap<&str, Vec<&'a str>>,
    labels: &BTreeMap<&'a str, String>,
    placed: &BTreeSet<&str>,
    id: &'a str,
    prefix: &str,
    seen: &mut BTreeSet<&'a str>,
) {
    let mut kids: Vec<&str> = depends.get(id).map(|v| v.to_vec()).unwrap_or_default();
    kids.sort();
    for (i, kid) in kids.iter().enumerate() {
        let last = i + 1 == kids.len();
        let (branch, ext) = if last {
            ("└── ", "    ")
        } else {
            ("├── ", "│   ")
        };
        let lab = labels.get(kid).map(String::as_str).unwrap_or(kid);
        if placed.contains(kid) {
            let _ = writeln!(
                s,
                "{prefix}{branch}[{lab}] (above)",
                lab = linked_inline(lab, kid)
            );
            continue;
        }
        if !seen.insert(kid) {
            let _ = writeln!(
                s,
                "{prefix}{branch}[{lab}] (above)",
                lab = linked_inline(lab, kid)
            );
            continue;
        }
        let _ = writeln!(s, "{prefix}{branch}[{lab}]", lab = linked_inline(lab, kid));
        chapel_tree(
            s,
            depends,
            labels,
            placed,
            kid,
            &format!("{prefix}{ext}"),
            seen,
        );
    }
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

/// All connected components (undirected) over origin-only dependence
/// edges, largest first: multi-node beats singleton, then size, then
/// smallest member id.
fn components(graph: &Graph, depends: &BTreeMap<&str, Vec<&str>>) -> Vec<BTreeSet<String>> {
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
                    if !seen.contains(nb) {
                        seen.insert(nb);
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
