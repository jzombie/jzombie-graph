//! zenOSmosis cluster renderer for `jzombie-graph` JSON.
//!
//! Reads the collector's schema via `Graph::from_json` (schema-checked) and
//! renders jzombie's portfolio as a 2D projection of the dependence graph:
//! nodes that share edges pull together, unconnected nodes push apart.
//! No buses, no stubs, no intermediate glyphs — one braille trace per real
//! edge: cargo-level dependence (`manifest`/`submodule`/`registry`) plus
//! `docs` references (runtime coupling the manifest can't see, e.g.
//! precompiled binaries the main project consumes). Docs edges pull with
//! lighter gravity in layout but render with the same glyphs.
//! Every node label is a link; tags are zero-width so
//! geometry computed on visible text stays aligned.
//!
//! Portfolio filter (renderer-side, mechanical — no hand lists):
//! - private nodes never render;
//! - forks render only when they are own work: `ahead_by > 0` (any commit)
//!   or renamed from the parent (`name` differs from the upstream repo name);
//! - stale nodes (`pushed_at` more than 365 days before `generated_at`)
//!   are omitted unless something current depends on them (transitive
//!   dependence pulls a stale dependency back in; a stale dependent that
//!   nothing uses stays out);
//! - only the largest connected component over dependence edges is drawn,
//!   so detached scraps never appear as footnote lists.

use jzombie_graph::graph::{EdgeKind, Graph};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// Grid size for the projection.
const WIDTH: usize = 78;
/// Height tuned so the real graph fits without wrapping labels.
const HEIGHT: usize = 26;
/// Staleness cutoff: pushes older than this before generation are omitted.
const STALE_DAYS: i64 = 365;

/// Render the cluster doc for a collected graph.
pub fn render_cathedral(graph: &Graph) -> String {
    let mut s = String::new();
    s.push_str("# The zenOSmosis Cluster\n\n");
    s.push_str("jzombie's portfolio as a 2D projection — proximity is edge strength.\n\n");
    let _ = writeln!(
        s,
        "Generated from `graph.json` (schema v{}). Braille traces show cargo dependence plus docs runtime coupling; every label links to GitHub.\n",
        graph.schema_version,
    );
    let Some(draw) = select_cluster(graph) else {
        s.push_str("(no connected cluster)\n");
        return s;
    };
    s.push_str(
        "<sub>Diagram: a 2D projection of these repos — closeness means shared dependencies. \
Screen readers: the block below uses braille characters for lines and is noisy, so it is hidden; \
the same edges are machine-readable in `graph.json`.</sub>\n\n",
    );
    s.push_str(&render_pre(graph, &draw));
    s.push_str(
        "Proximity reflects dependence strength: nodes sharing an edge attract, \
unconnected nodes repel. Forks appear only when diverged or renamed; \
repos untouched for over a year are omitted unless something current depends on them.\n",
    );
    s
}

/// Nodes + edges selected for the drawing. Strong edges are cargo-level
/// dependence (`manifest`/`submodule`/`registry`); weak edges are `docs`
/// references (runtime coupling the manifest can't see, e.g.
/// `windows-headless` ↔ `lab-qemu-portable`). Workflow stays out as tooling.
struct Cluster {
    /// Cluster member ids, sorted.
    nodes: Vec<String>,
    /// Undirected dependence pairs within the cluster, deduped.
    edges: Vec<(String, String)>,
    /// Undirected docs-reference pairs within the cluster, deduped.
    weak: Vec<(String, String)>,
    /// Edge multiplicity (parallel dependence edges) per undirected pair.
    weight: BTreeMap<(String, String), f64>,
}

fn select_cluster(graph: &Graph) -> Option<Cluster> {
    // Mechanical portfolio filter. Staleness has one exception: a stale
    // node stays if it is a (transitive) dependence of current work.
    let mut eligible: BTreeSet<&str> = BTreeSet::new();
    let mut fresh: BTreeSet<&str> = BTreeSet::new();
    for n in &graph.nodes {
        if n.private {
            continue;
        }
        if n.fork && !is_own_work(n) {
            continue;
        }
        eligible.insert(n.id.as_str());
        if !is_stale(graph, n) {
            fresh.insert(n.id.as_str());
        }
    }
    // Dependence edges among eligible nodes (stale or not) drive the closure.
    let mut dep_edges: Vec<(&str, &str)> = Vec::new();
    for e in &graph.edges {
        if e.kind != EdgeKind::Manifest
            && e.kind != EdgeKind::Submodule
            && e.kind != EdgeKind::Registry
        {
            continue;
        }
        if eligible.contains(e.source.as_str()) && eligible.contains(e.target.as_str()) {
            dep_edges.push((e.source.as_str(), e.target.as_str()));
        }
    }
    // Pull stale dependencies back in: from every kept node, follow
    // source -> target (dependent -> dependency) transitively.
    let mut candidates: BTreeSet<&str> = fresh.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for (src, tgt) in &dep_edges {
            if candidates.contains(src) && !candidates.contains(tgt) {
                candidates.insert(tgt);
                changed = true;
            }
        }
    }
    // Dependence adjacency among candidates.
    let mut adj: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut pairs: BTreeMap<(String, String), f64> = BTreeMap::new();
    for e in &graph.edges {
        if e.kind != EdgeKind::Manifest
            && e.kind != EdgeKind::Submodule
            && e.kind != EdgeKind::Registry
        {
            continue;
        }
        if !candidates.contains(e.source.as_str()) || !candidates.contains(e.target.as_str()) {
            continue;
        }
        adj.entry(e.source.as_str())
            .or_default()
            .insert(e.target.as_str());
        adj.entry(e.target.as_str())
            .or_default()
            .insert(e.source.as_str());
        let a = e.source.clone();
        let b = e.target.clone();
        let key = if a <= b { (a, b) } else { (b, a) };
        *pairs.entry(key).or_default() += 1.0;
    }
    for id in &candidates {
        adj.entry(id).or_default();
    }
    // Largest connected component (multi-node wins, then size, then id).
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
        if comp.len() > 1 {
            comps.push(comp);
        }
    }
    comps.sort_by(|a, b| {
        (b.len(), std::cmp::Reverse(b.iter().next()))
            .cmp(&(a.len(), std::cmp::Reverse(a.iter().next())))
    });
    let mut core: BTreeSet<String> = comps.into_iter().next()?;
    // Docs satellites: fresh eligible nodes with a direct docs edge to the
    // dependence core (runtime coupling, e.g. portable QEMU builds the VM
    // host consumes but doesn't declare as a cargo dep). The satellite must
    // contain code (detected `language`) — prose alone (description/topics)
    // never qualifies, so an asset bucket stays out even if it gains a
    // readme blurb later. Workflow stays out as tooling.
    for e in &graph.edges {
        if e.kind != EdgeKind::Docs {
            continue;
        }
        let (s, t) = (e.source.as_str(), e.target.as_str());
        if !eligible.contains(s) || !eligible.contains(t) {
            continue;
        }
        let other = if core.contains(s) && fresh.contains(t) {
            Some(t)
        } else if core.contains(t) && fresh.contains(s) {
            Some(s)
        } else {
            None
        };
        if let Some(id) = other {
            let is_code = graph
                .nodes
                .iter()
                .find(|n| n.id == id)
                .is_some_and(|n| n.language.is_some());
            if is_code {
                core.insert(id.to_string());
            }
        }
    }
    let nodes: Vec<String> = core.iter().cloned().collect();
    let in_core = |id: &str| core.contains(id);
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut weight: BTreeMap<(String, String), f64> = BTreeMap::new();
    for (k, w) in pairs {
        if in_core(&k.0) && in_core(&k.1) {
            edges.push(k.clone());
            weight.insert(k, w);
        }
    }
    edges.sort();
    let mut weak: Vec<(String, String)> = Vec::new();
    for e in &graph.edges {
        if e.kind != EdgeKind::Docs {
            continue;
        }
        let (mut a, mut b) = (e.source.clone(), e.target.clone());
        if !in_core(&a) || !in_core(&b) {
            continue;
        }
        if a > b {
            std::mem::swap(&mut a, &mut b);
        }
        if !weak.contains(&(a.clone(), b.clone())) {
            weak.push((a.clone(), b.clone()));
        }
        // Each directed docs edge adds pull; a mutual pair (both
        // directions, like the runtime-coupling pairs here) lands at 0.6 —
        // still clearly lighter than a cargo edge at 1.0+.
        *weight.entry((a, b)).or_insert(0.0) += 0.3;
    }
    weak.sort();
    Some(Cluster {
        nodes,
        edges,
        weak,
        weight,
    })
}

fn is_own_work(n: &jzombie_graph::graph::Node) -> bool {
    if n.ahead_by.unwrap_or(0) > 0 {
        return true;
    }
    let Some(up) = n.upstream.as_deref() else {
        return false;
    };
    let parent = up.split('/').nth(1).unwrap_or(up);
    !parent.eq_ignore_ascii_case(&n.name)
}

fn is_stale(graph: &Graph, n: &jzombie_graph::graph::Node) -> bool {
    let (Some(pushed),) = (n.pushed_at,) else {
        return false;
    };
    (graph.generated_at - pushed).num_days() > STALE_DAYS
}

fn render_pre(graph: &Graph, cluster: &Cluster) -> String {
    let labels = abbreviations(graph, &cluster.nodes);
    let fan_in = graph.node_fan_in();
    // Deterministic force-directed layout over all edges (docs keeps
    // lighter gravity). Weak-only satellites would still orbit far on
    // light pull, so pin each just outside its anchor instead — adjacent
    // still means connected, and they stop stretching the normalization
    // that scrunches the core.
    let mut pos = layout(cluster);
    let strong_members: BTreeSet<&str> = cluster
        .edges
        .iter()
        .flat_map(|(a, b)| [a.as_str(), b.as_str()])
        .collect();
    if !strong_members.is_empty() {
        let (mut cx, mut cy) = (0.0, 0.0);
        for id in &strong_members {
            if let Some((x, y)) = pos.get(*id) {
                cx += x;
                cy += y;
            }
        }
        cx /= strong_members.len() as f64;
        cy /= strong_members.len() as f64;
        for id in &cluster.nodes {
            if strong_members.contains(id.as_str()) {
                continue;
            }
            let mut anchors: Vec<&str> = cluster
                .weak
                .iter()
                .filter_map(|(a, b)| {
                    if a == id {
                        Some(b.as_str())
                    } else if b == id {
                        Some(a.as_str())
                    } else {
                        None
                    }
                })
                .collect();
            anchors.sort();
            let anchor = anchors
                .iter()
                .find(|a| strong_members.contains(*a))
                .or(anchors.first())
                .copied();
            let Some(anchor) = anchor else { continue };
            let Some(&(ax, ay)) = pos.get(anchor) else {
                continue;
            };
            let (mut dx, mut dy) = (ax - cx, ay - cy);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 0.01 {
                dx = 1.0;
                dy = 0.0;
            } else {
                dx /= len;
                dy /= len;
            }
            pos.insert(
                id.clone(),
                (
                    (ax + dx * 0.09).clamp(0.0, 1.0),
                    (ay + dy * 0.09).clamp(0.0, 1.0),
                ),
            );
        }
    }
    // Map to grid with padding for label widths.
    let mut placed: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new(); // id -> (x, y, w)
    let mut order: Vec<&String> = cluster.nodes.iter().collect();
    order.sort_by(|a, b| {
        fan_in
            .get(*b)
            .copied()
            .unwrap_or(0)
            .cmp(&fan_in.get(*a).copied().unwrap_or(0))
            .then_with(|| a.cmp(b))
    });
    let mut boxes: Vec<(usize, usize, usize, usize)> = Vec::new(); // x0,y,w,h=1
    for id in &order {
        let label = labels.get(id.as_str()).map(String::as_str).unwrap_or(id);
        let w = label.len() + 2; // [label]
        let (fx, fy) = pos.get(id.as_str()).copied().unwrap_or((0.0, 0.0));
        let mut x = (fx * (WIDTH as f64 - w as f64 - 2.0) + 1.0).round() as isize;
        let mut y = (fy * (HEIGHT as f64 - 3.0) + 1.0).round() as isize;
        x = x.clamp(0, WIDTH as isize - w as isize);
        y = y.clamp(0, HEIGHT as isize - 1);
        // Nudge down/right until free (deterministic, bounded).
        for _ in 0..HEIGHT {
            if boxes.iter().all(|(ox, oy, ow, _)| {
                (x as usize + w + 1 <= *ox) || (*ox + *ow + 1 <= x as usize) || (y as usize != *oy)
            }) {
                break;
            }
            y += 1;
            if y as usize >= HEIGHT {
                y = 0;
                x = (x + 3).min(WIDTH as isize - w as isize);
            }
        }
        let (xu, yu) = (x as usize, y as usize);
        boxes.push((xu, yu, w, 1));
        placed.insert(id.as_str(), (xu, yu, w));
    }
    // Braille dot-grid for traces: 2×4 dots per character cell. Strong
    // (cargo) and weak (docs runtime coupling) edges share the same glyphs;
    // docs keeps lighter gravity in layout only. Nodes mask the dots beneath.
    let dw = WIDTH * 2;
    let dh = HEIGHT * 4;
    let mut dots: Vec<Vec<bool>> = vec![vec![false; dw]; dh];
    for (a, b) in cluster.edges.iter().chain(cluster.weak.iter()) {
        let Some(&(ax, ay, aw)) = placed.get(a.as_str()) else {
            continue;
        };
        let Some(&(bx, by, bw)) = placed.get(b.as_str()) else {
            continue;
        };
        let (x0, y0) = (ax as isize * 2 + aw as isize, ay as isize * 4 + 2);
        let (x1, y1) = (bx as isize * 2 + bw as isize, by as isize * 4 + 2);
        for (x, y) in bresenham(x0, y0, x1, y1) {
            if x < 0 || y < 0 || x >= dw as isize || y >= dh as isize {
                continue;
            }
            dots[y as usize][x as usize] = true;
        }
    }
    // Mask dots hidden under node boxes.
    for (bx, by, bw, _) in &boxes {
        for dy in (by * 4)..((by + 1) * 4).min(dh) {
            for dx in (bx * 2)..((bx + bw) * 2).min(dw) {
                dots[dy][dx] = false;
            }
        }
    }
    let mut grid: Vec<Vec<char>> = vec![vec![' '; WIDTH]; HEIGHT];
    for cy in 0..HEIGHT {
        for cx in 0..WIDTH {
            let mut bits = 0u8;
            // Braille bit order: ⠁⠂⡀ ⠈⠐⢀ ⡈⢈ (see `braille_of`).
            if dots[cy * 4][cx * 2] {
                bits |= 0x01;
            }
            if dots[cy * 4 + 1][cx * 2] {
                bits |= 0x02;
            }
            if dots[cy * 4 + 2][cx * 2] {
                bits |= 0x04;
            }
            if dots[cy * 4][cx * 2 + 1] {
                bits |= 0x08;
            }
            if dots[cy * 4 + 1][cx * 2 + 1] {
                bits |= 0x10;
            }
            if dots[cy * 4 + 2][cx * 2 + 1] {
                bits |= 0x20;
            }
            if dots[cy * 4 + 3][cx * 2] {
                bits |= 0x40;
            }
            if dots[cy * 4 + 3][cx * 2 + 1] {
                bits |= 0x80;
            }
            grid[cy][cx] = braille_of(bits);
        }
    }
    // Nodes on top; record link spans on visible text.
    let mut links: BTreeMap<(usize, usize, usize), String> = BTreeMap::new();
    for id in &cluster.nodes {
        let (x, y, w) = placed[id.as_str()];
        let label = labels.get(id.as_str()).map(String::as_str).unwrap_or(id);
        let text = format!("[{label}]");
        for (i, c) in text.chars().enumerate() {
            if x + i < WIDTH {
                grid[y][x + i] = c;
            }
        }
        links.insert((y, x + 1, x + w - 1), url_of(id));
        let _ = w;
    }
    let mut d = String::from("<pre aria-hidden=\"true\">\n");
    for (r, line) in grid.iter().enumerate() {
        let text: String = line.iter().collect();
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            d.push('\n');
            continue;
        }
        d.push_str(&apply_links(trimmed, r, &links));
        d.push('\n');
    }
    d.push_str("</pre>\n");
    d
}

/// Deterministic Fruchterman–Reingold: connected nodes attract (scaled by
/// edge multiplicity), all pairs repel. Output normalized to [0,1]².
fn layout(cluster: &Cluster) -> BTreeMap<String, (f64, f64)> {
    let n = cluster.nodes.len();
    let mut idx: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, id) in cluster.nodes.iter().enumerate() {
        idx.insert(id.as_str(), i);
    }
    let mut p: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let a = 2.0 * std::f64::consts::PI * i as f64 / n.max(1) as f64;
            (10.0 * a.cos(), 10.0 * a.sin())
        })
        .collect();
    let area = 100.0 * 100.0;
    let k = (area / n.max(1) as f64).sqrt().max(1.0);
    let mut temp = 5.0;
    // Adjacency with weights (strong + weak docs references).
    let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for (a, b) in cluster.edges.iter().chain(cluster.weak.iter()) {
        let (Some(&ia), Some(&ib)) = (idx.get(a.as_str()), idx.get(b.as_str())) else {
            continue;
        };
        let key = if *a <= *b {
            (a.clone(), b.clone())
        } else {
            (b.clone(), a.clone())
        };
        let w = cluster.weight.get(&key).copied().unwrap_or(1.0);
        adj[ia].push((ib, w));
        adj[ib].push((ia, w));
    }
    for _ in 0..160 {
        let mut disp = vec![(0.0, 0.0); n];
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = p[j].0 - p[i].0;
                let dy = p[j].1 - p[i].1;
                let dist = (dx * dx + dy * dy).sqrt().max(0.01);
                let f = k * k / dist;
                let ux = dx / dist;
                let uy = dy / dist;
                disp[i].0 -= ux * f;
                disp[i].1 -= uy * f;
                disp[j].0 += ux * f;
                disp[j].1 += uy * f;
            }
        }
        for i in 0..n {
            for (j, w) in &adj[i] {
                let dx = p[*j].0 - p[i].0;
                let dy = p[*j].1 - p[i].1;
                let dist = (dx * dx + dy * dy).sqrt().max(0.01);
                let f = dist * dist / k * w.min(3.0);
                disp[i].0 += dx / dist * f;
                disp[i].1 += dy / dist * f;
            }
        }
        for i in 0..n {
            let len = (disp[i].0 * disp[i].0 + disp[i].1 * disp[i].1)
                .sqrt()
                .max(0.01);
            let step = len.min(temp);
            p[i].0 += disp[i].0 / len * step;
            p[i].1 += disp[i].1 / len * step;
        }
        temp *= 0.95;
    }
    // Normalize to [0,1]².
    let (mut minx, mut maxx) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut miny, mut maxy) = (f64::INFINITY, f64::NEG_INFINITY);
    for (x, y) in &p {
        minx = minx.min(*x);
        maxx = maxx.max(*x);
        miny = miny.min(*y);
        maxy = maxy.max(*y);
    }
    let dx = (maxx - minx).max(0.01);
    let dy = (maxy - miny).max(0.01);
    cluster
        .nodes
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), ((p[i].0 - minx) / dx, (p[i].1 - miny) / dy)))
        .collect()
}

fn bresenham(x0: isize, y0: isize, x1: isize, y1: isize) -> Vec<(isize, isize)> {
    let mut out = Vec::new();
    let (mut x, mut y) = (x0, y0);
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let mut err = dx + dy;
    loop {
        out.push((x, y));
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
    out
}

/// Empty cell stays a space (trims cleanly); any dots become U+2800 + bits.
fn braille_of(bits: u8) -> char {
    if bits == 0 {
        return ' ';
    }
    char::from_u32(0x2800 + bits as u32).unwrap_or('·')
}

/// Splice `<a>` links into one grid row at the recorded visible spans.
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

fn abbreviations<'a>(graph: &'a Graph, wanted: &[String]) -> BTreeMap<&'a str, String> {
    let want: BTreeSet<&str> = wanted.iter().map(String::as_str).collect();
    let ids: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| want.contains(n.id.as_str()))
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
