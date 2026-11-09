//! Commit-share pie charts, OxDock-templated.
//!
//! Split of responsibilities (deliberate):
//! - Rust owns parsing (`Graph::from_json`, schema-checked), row math, and
//!   SVG geometry — all unit-tested, deterministic floats, no timestamps.
//! - The embedded OxDock script owns markup emission only: loop, interpolate,
//!   write. No file reads (hermetic — tests need no `data/` checkout).
//! - Path knowledge lives once, in the binary, from `paths::` constants.
//!   The script never names a data path.
//!
//! Bytes pinned by snapshot tests; the script self-checks with ASSERTs.

use oxdock::{Engine, HostModule, OxDockFn, Value, oxdock, oxdock_func, oxdock_parser};

pub mod palette;

/// Shared `<style>` block, single-sourced from `style.oxfile`.
/// The CSS lives once in the DSL; Rust only extracts the payload,
/// so no stylesheet literal can drift between charts.
const STYLE_OXFILE: &str = include_str!("../style.oxfile");

fn chart_style() -> anyhow::Result<String> {
    let line = STYLE_OXFILE
        .lines()
        .find(|l| l.trim_start().starts_with("RETURN \""))
        .ok_or_else(|| anyhow::anyhow!("CHART_STYLE payload missing in style.oxfile"))?;
    let start = line.find('"').map(|i| i + 1).unwrap_or(0);
    let end = line.rfind('"').unwrap_or(0);
    anyhow::ensure!(end > start, "CHART_STYLE payload unterminated");
    Ok(line[start..end].to_string())
}

/// OxDock string-literal transport: `#var` interpolation inlines raw text
/// into the script, so quotes/backslashes in data break parsing. Base64
/// is escape-proof by construction; host funcs decode on entry.
fn transport_in(raw: &str) -> String {
    use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
    B64.encode(raw)
}

fn transport_out(b64: &str) -> anyhow::Result<String> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
    let bytes = B64
        .decode(b64)
        .map_err(|e| anyhow::anyhow!("base64 transport: {e}"))?;
    String::from_utf8(bytes).map_err(|e| anyhow::anyhow!("transport utf8: {e}"))
}

/// Plugin function: the shared style block out. Every chart template
/// calls this and appends the result — one FUNC, four callers.
#[oxdock_func(pure, name = "CHART_STYLE")]
fn chart_style_fn() -> anyhow::Result<Value> {
    Ok(Value::string(chart_style()?))
}

/// One pie slice: everything preformatted — the script interpolates only.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct SliceRow {
    label: String,
    pct: String,
    color: String,
    /// Breathing class from the repo's heat band (`pulse`, `pulse slow`,
    /// or empty for static) — width is share, motion is heat.
    pulse: String,
    /// True when this row is the whole pie (single contributor): the script
    /// emits a `<circle>` instead of a degenerate zero-length arc path.
    circle: bool,
    /// SVG path data (`M cx,cy L ... A ... Z`), 1-decimal, deterministic.
    d: String,
    /// Legend text / swatch positions, preformatted.
    tx: String,
    ty: String,
    sx: String,
    sy: String,
}

/// Chart document: every value precomputed in Rust, keyed by name.
/// The `oxdock!` macro frontend pads bare digit runs in triple-quoted
/// blocks, so templates carry no literals except tag/attribute names —
/// even font sizes and colors ride in as interpolated values.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct PieDoc {
    header: String,
    footer: String,
    cx: String,
    cy: String,
    r: String,
    cell: String,
    fs: String,
    ink: String,
    muted: String,
    mx: String,
    my: String,
    msg: String,
    empty: bool,
    rows: Vec<SliceRow>,
}

/// Canvas geometry from the row count. Fixed pie; the canvas fits whichever
/// is taller, pie or legend — radius never exceeds the canvas, so small
/// row counts can't overflow (the single-circle case used to).
struct Layout {
    w: f64,
    h: f64,
    cx: f64,
    cy: f64,
    r: f64,
    leg_x: f64,
    title_h: f64,
    row_h: f64,
    body_h: f64,
}

fn layout(count: usize) -> Layout {
    let (pad, title_h, row_h, r) = (16.0, 44.0, 26.0, 90.0);
    let pie_h = 2.0 * r + 2.0 * pad;
    let leg_h = count as f64 * row_h + pad;
    let body_h = pie_h.max(leg_h);
    let cx = pad + r;
    Layout {
        w: cx + r + 28.0 + 220.0,
        h: title_h + body_h,
        cx,
        cy: title_h + body_h / 2.0,
        r,
        leg_x: cx + r + 28.0,
        title_h,
        row_h,
        body_h,
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn f1(x: f64) -> String {
    format!("{x:.1}")
}

/// Rows for the commit-share pie: own-work repos only, via the shared
/// `is_own_work` filter — pristine forks you never contributed to never
/// slice. Top-`n` plus an `Other` bucket, with pie geometry precomputed.
/// Zero-commit repos never slice. Empty when nothing committed in window
/// (script emits a labeled empty canvas instead).
/// Colors come from the shared [`palette`] module — same values every chart.
fn share_rows(graph_json: &str, n: usize) -> anyhow::Result<Vec<SliceRow>> {
    let graph = jzombie_graph::graph::Graph::from_json(graph_json)?;
    let mut repos: Vec<(&str, u32, Option<f32>)> = graph
        .nodes
        .iter()
        .filter(|nd| !nd.private && jzombie_graph::graph::is_own_work(nd))
        .filter(|nd| nd.commits_5d > 0)
        .map(|nd| {
            (
                nd.name.as_str(),
                nd.commits_5d,
                nd.commit_heat.or_else(|| nd.commit_heat()),
            )
        })
        .collect();
    repos.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let total: u32 = repos.iter().map(|(_, c, _)| c).sum();
    if total == 0 {
        return Ok(vec![]);
    }
    let n = n.max(1);
    let mut kept: Vec<(String, u32, Option<f32>)> = repos
        .iter()
        .take(n)
        .map(|(name, c, h)| (name.to_string(), *c, *h))
        .collect();
    let rest: u32 = repos.iter().skip(n).map(|(_, c, _)| c).sum();
    if rest > 0 {
        // The bucket aggregates repos of every band — it stays static.
        kept.push(("Other".to_string(), rest, None));
    }
    let kept_len = kept.len();
    let lay = layout(kept_len);
    let mut acc = 0.0;
    let rows = kept
        .into_iter()
        .enumerate()
        .map(|(i, (label, c, heat))| {
            let share = c as f64 / total as f64;
            let sweep = share * 360.0;
            let a0 = -90.0 + acc * 360.0;
            acc += share;
            let color = if label == "Other" {
                palette::other().to_string()
            } else {
                palette::categorical(i).to_string()
            };
            let ty = lay.title_h
                + (lay.body_h - kept_len as f64 * lay.row_h) / 2.0
                + i as f64 * lay.row_h
                + 18.0;
            SliceRow {
                label: xml_escape(&label),
                pct: format!("{:.1}%", share * 100.0),
                color,
                pulse: palette::heat_pulse(heat).to_string(),
                circle: kept_len == 1,
                d: {
                    let rad = std::f64::consts::PI / 180.0;
                    let x1 = lay.cx + lay.r * (a0 * rad).cos();
                    let y1 = lay.cy + lay.r * (a0 * rad).sin();
                    let x2 = lay.cx + lay.r * ((a0 + sweep) * rad).cos();
                    let y2 = lay.cy + lay.r * ((a0 + sweep) * rad).sin();
                    let large = if sweep > 180.0 { 1 } else { 0 };
                    format!(
                        "M {},{} L {},{} A {},{} 0 {},1 {},{} Z",
                        f1(lay.cx),
                        f1(lay.cy),
                        f1(x1),
                        f1(y1),
                        f1(lay.r),
                        f1(lay.r),
                        large,
                        f1(x2),
                        f1(y2)
                    )
                },
                tx: f1(lay.leg_x + 22.0),
                ty: f1(ty),
                sx: f1(lay.leg_x),
                sy: f1(ty - 13.0),
            }
        })
        .collect();
    Ok(rows)
}

/// One focus bar: everything preformatted — the script interpolates only.
/// `bx`/`bw` place the bar against the zero axis; name and value live in
/// fixed left columns (right-anchored). Ink splits per pixel, not per
/// element: gray text throughout, dark re-drawn clipped to the bar rect —
/// a label straddling the bar edge reads correctly on both sides.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct FocusRow {
    label: String,
    value: String,
    color: String,
    /// Breathing class from the bar's own heat (`pulse`, `pulse slow`,
    /// or empty) — only actually-hot bars move.
    pulse: String,
    bx: String,
    bw: String,
    by: String,
    tx: String,
    ty: String,
    vx: String,
    vanchor: String,
    clip: String,
}

/// Chart document: diverging bars from a zero axis. Same contract as PieDoc
/// (all values precomputed in Rust, script only interpolates).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct FocusDoc {
    header: String,
    footer: String,
    zero_x: String,
    zero_y1: String,
    zero_y2: String,
    bar_h: String,
    fs: String,
    ink: String,
    bar_ink: String,
    muted: String,
    mx: String,
    my: String,
    msg: String,
    empty: bool,
    rows: Vec<FocusRow>,
}

/// Longest repo name shown; longer names truncate with an ellipsis.
const LABEL_MAX: usize = 18;

fn truncate_name(name: &str, max: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= max {
        name.to_string()
    } else {
        chars[..max.saturating_sub(1)].iter().collect::<String>() + "…"
    }
}

/// Elbow cut for the negative side: how many leading (closest-to-zero)
/// negatives to keep. The largest consecutive drop cuts only when it dwarfs
/// the rest — bigger than one heat unit and 10x the median gap — else the
/// slope is smooth and everything stays. Fewer than 3 negatives can't elbow.
fn elbow_keep(neg: &[f32]) -> usize {
    if neg.len() < 3 {
        return neg.len();
    }
    let gaps: Vec<f32> = neg.windows(2).map(|w| w[0] - w[1]).collect();
    let mut sorted = gaps.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    let median = if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    };
    let (idx, max) = gaps
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, m)| (i, *m))
        .unwrap_or((0, 0.0));
    if max > 1.0 && max > 10.0 * median.max(1e-6) {
        idx + 1
    } else {
        neg.len()
    }
}

/// Top-`n_top` repos by focus plus the next `n_bottom` below (one smooth
/// ranking), unknown-focus repos excluded — unknowns can't sit on a scale.
/// Own-work repos only, via the shared `is_own_work` filter.
/// The negative side elbow-cuts: a sudden cliff (dormant abyss orders of
/// magnitude past the near-zero cluster) is dropped, so one scale doesn't
/// flatten the cluster that actually varies.
/// Display order stays descending. Each side scales to its own max, so heat
/// and dormancy depth (different processes, different magnitudes) each read
/// fully; exact values ride in the labels.
fn focus_rows(graph_json: &str, n_top: usize, n_bottom: usize) -> anyhow::Result<Vec<FocusRow>> {
    let graph = jzombie_graph::graph::Graph::from_json(graph_json)?;
    let now = graph.generated_at;
    let mut scored: Vec<(&str, f32)> = graph
        .nodes
        .iter()
        .filter(|nd| !nd.private && jzombie_graph::graph::is_own_work(nd))
        .filter_map(|nd| {
            nd.focus
                .or_else(|| nd.focus(&now))
                .map(|f| (nd.name.as_str(), f))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(b.0))
    });
    // Elbow: truncate the negative suffix at the cliff, if any.
    let neg_start = scored
        .iter()
        .position(|(_, f)| *f < 0.0)
        .unwrap_or(scored.len());
    let keep = neg_start
        + elbow_keep(
            &scored[neg_start..]
                .iter()
                .map(|(_, f)| *f)
                .collect::<Vec<_>>(),
        );
    scored.truncate(keep);
    let mut picked: Vec<(&str, f32)> = scored.iter().take(n_top.max(1)).copied().collect();
    // Contiguous: the next `n_bottom` below picked, still within the elbow.
    // Jumping to the deepest would redraw the cliff inside the chart and
    // flatten every bar above it into invisibility.
    let tail: Vec<(&str, f32)> = scored
        .iter()
        .skip(picked.len())
        .take(n_bottom)
        .copied()
        .collect();
    picked.extend(tail);
    if picked.is_empty() {
        return Ok(vec![]);
    }
    // Per-side scales: positives fill the right half against their own max,
    // negatives the left half against theirs. Heat caps at 6 while dormancy
    // sinks unbounded — one shared scale would erase the top cluster.
    let pos_max = picked
        .iter()
        .filter(|(_, f)| *f >= 0.0)
        .map(|(_, f)| *f)
        .fold(0.0_f32, f32::max)
        .max(0.01);
    let neg_max = picked
        .iter()
        .filter(|(_, f)| *f < 0.0)
        .map(|(_, f)| f.abs())
        .fold(0.0_f32, f32::max)
        .max(0.01);
    // Fixed columns: names right-anchored at `name_x`, values right-anchored
    // at `value_x` (tabular, never collides with bars), zero axis, bars.
    let title_h = 44.0;
    let row_h = 26.0;
    let name_x = 130.0;
    let value_x = 200.0;
    let zero_x = 260.0;
    let max_bar = 200.0;
    let rows = picked
        .into_iter()
        .enumerate()
        .map(|(i, (label, f))| {
            let len = if f >= 0.0 {
                f as f64 / pos_max as f64 * max_bar
            } else {
                f.abs() as f64 / neg_max as f64 * max_bar
            };
            let bx = if f >= 0.0 { zero_x } else { zero_x - len };
            let ty = title_h + i as f64 * row_h + 18.0;
            FocusRow {
                label: xml_escape(&truncate_name(label, LABEL_MAX)),
                value: format!("{f:.2}"),
                color: if f >= 0.0 {
                    palette::heating()
                } else {
                    palette::focus_negative().to_string()
                },
                pulse: palette::heat_pulse(Some(f)).to_string(),
                bx: f1(bx),
                bw: f1(len),
                by: f1(title_h + i as f64 * row_h + 6.0),
                tx: f1(name_x),
                ty: f1(ty),
                vx: f1(value_x),
                vanchor: "end".to_string(),
                clip: format!("fc{i}"),
            }
        })
        .collect();
    Ok(rows)
}

/// Plugin function: validated graph JSON in, focus-bars document out.
#[oxdock_func(pure, name = "FOCUS_BARS")]
fn focus_bars(graph_json: String, n_top: i64, n_bottom: i64) -> anyhow::Result<Value> {
    let graph_json = transport_out(&graph_json)?;
    let rows = focus_rows(&graph_json, n_top.max(1) as usize, n_bottom.max(0) as usize)?;
    let h = 44.0 + rows.len() as f64 * 26.0 + 16.0;
    let doc = FocusDoc {
        header: format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" role=\"img\" font-family=\"{family}\"><title>Focus ranking</title><text x=\"16\" y=\"30\" font-size=\"20\" fill=\"{ink}\">Focus ranking</text>",
            f1(480.0),
            f1(h),
            ink = palette::ink(),
            family = palette::family(),
        ),
        footer: "</svg>".to_string(),
        zero_x: f1(260.0),
        zero_y1: f1(44.0),
        zero_y2: f1(h - 16.0),
        bar_h: "14".to_string(),
        fs: "14".to_string(),
        ink: palette::ink(),
        bar_ink: palette::ink_on_bar(),
        muted: palette::muted(),
        mx: f1(240.0),
        my: f1(44.0 + 40.0),
        msg: "No focus data".to_string(),
        empty: rows.is_empty(),
        rows,
    };
    Ok(Value::string(serde_json::to_string(&doc)?))
}

/// Render the focus ranking bars for validated graph JSON. Same transport
/// as the pie: embedded script emits markup into a temp workspace.
pub fn render_focus_svg(graph_json: &str) -> anyhow::Result<String> {
    let engine = engine();
    let arg = transport_in(graph_json);
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS],
        IMPORT [CHARTS]
        IMPORT [STD]
        LET $rows_json: STRING = CHARTS::FOCUS_BARS(#arg, 12, 5)
        WRITE rows.json $rows_json
        LET $doc: MAP<ANY> = LOAD_JSON("rows.json")
        WRITE out.svg $doc.header
        LET $style: STRING = CHARTS::CHART_STYLE()
        APPEND out.svg $style
        APPEND out.svg "<line x1=\"{{ $doc.zero_x }}\" y1=\"{{ $doc.zero_y1 }}\" x2=\"{{ $doc.zero_x }}\" y2=\"{{ $doc.zero_y2 }}\" stroke=\"{{ $doc.muted }}\" stroke-width=\"1\"/>"
        IF $doc.empty {
            APPEND out.svg "<text x=\"{{ $doc.mx }}\" y=\"{{ $doc.my }}\" text-anchor=\"middle\" font-size=\"{{ $doc.fs }}\" fill=\"{{ $doc.muted }}\">{{ $doc.msg }}</text>"
        } ELSE {
            FOR $r: MAP<ANY> IN $doc.rows {
                APPEND out.svg """
                    <clipPath id="{{ $r.clip }}"><rect x="{{ $r.bx }}" y="{{ $r.by }}" width="{{ $r.bw }}" height="{{ $doc.bar_h }}"/></clipPath>
                    <rect x="{{ $r.bx }}" y="{{ $r.by }}" width="{{ $r.bw }}" height="{{ $doc.bar_h }}" fill="{{ $r.color }}" class="{{ $r.pulse }}"><title>{{ $r.label }} {{ $r.value }}</title></rect>
                    """
                APPEND out.svg "<text x=\"{{ $r.tx }}\" y=\"{{ $r.ty }}\" text-anchor=\"end\" font-size=\"{{ $doc.fs }}\" fill=\"{{ $doc.ink }}\">{{ $r.label }}</text><text x=\"{{ $r.vx }}\" y=\"{{ $r.ty }}\" text-anchor=\"end\" font-size=\"{{ $doc.fs }}\" fill=\"{{ $doc.ink }}\">{{ $r.value }}</text>"
                APPEND out.svg "<text x=\"{{ $r.tx }}\" y=\"{{ $r.ty }}\" text-anchor=\"end\" font-size=\"{{ $doc.fs }}\" fill=\"{{ $doc.bar_ink }}\" clip-path=\"url(#{{ $r.clip }})\">{{ $r.label }}</text><text x=\"{{ $r.vx }}\" y=\"{{ $r.ty }}\" text-anchor=\"end\" font-size=\"{{ $doc.fs }}\" fill=\"{{ $doc.bar_ink }}\" clip-path=\"url(#{{ $r.clip }})\">{{ $r.value }}</text>"
            }
        }
        APPEND out.svg $doc.footer
        LET $check: STRING = READ out.svg
        ASSERT_CONTAINS $check "<svg"
        ASSERT_CONTAINS $check "</svg>"
    };
    let temp = oxdock::GuardedPath::tempdir().expect("tempdir");
    let root = temp.as_guarded_path().clone();
    engine.run_steps_without_snapshot(&root, &steps)?;
    let resolver = oxdock::PathResolver::new(root.as_path(), root.as_path()).expect("resolver");
    let out = root.join("out.svg").expect("out path");
    Ok(resolver.read_to_string(&out).expect("read out"))
}

/// Plugin function: validated graph JSON in, chart document out.
/// Pure data shaping (no IO) — the binary reads the file once, from the
/// `paths::` constant; the script never names a data path.
#[oxdock_func(pure, name = "PIE_SLICES")]
fn pie_slices(graph_json: String, n: i64) -> anyhow::Result<Value> {
    let graph_json = transport_out(&graph_json)?;
    let rows = share_rows(&graph_json, n.max(1) as usize)?;
    let lay = layout(rows.len());
    let doc = PieDoc {
        header: format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" role=\"img\" font-family=\"{family}\"><title>Trailing-5d commit share</title><text x=\"16\" y=\"30\" font-size=\"20\" fill=\"{ink}\">Trailing-5d commit share</text>",
            f1(lay.w),
            f1(lay.h),
            ink = palette::ink(),
            family = palette::family(),
        ),
        footer: "</svg>".to_string(),
        cx: f1(lay.cx),
        cy: f1(lay.cy),
        r: f1(lay.r),
        cell: "14".to_string(),
        fs: "14".to_string(),
        ink: palette::ink(),
        muted: palette::muted(),
        mx: f1(lay.w / 2.0),
        my: f1(lay.title_h + 40.0),
        msg: "No commits in trailing 5d".to_string(),
        empty: rows.is_empty(),
        rows,
    };
    Ok(Value::string(serde_json::to_string(&doc)?))
}

/// One card: name, what it's about, activity, top work. Whisked through
/// one at a time; the readout is the drilldown — no tiny bars to squint at.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct FlameRepo {
    rid: String,
    nav: String,
    tour_delay: String,
    title: String,
    desc: String,
    stats: String,
    detail: String,
    share_w: String,
    color: String,
    pulse: String,
}

/// Flame document: static decor prebuilt in Rust, rows interpolated.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct FlameDoc {
    header: String,
    legend: String,
    footer: String,
    port_label: String,
    empty: bool,
    msg: String,
    muted: String,
    fw: String,
    fh: String,
    tour_dur: String,
    grads: String,
    // Layout values, so the template carries no coordinates.
    px: String,
    py: String,
    pw: String,
    ph: String,
    pty: String,
    dx: String,
    dy: String,
    dw: String,
    dh: String,
    lx: String,
    l1y: String,
    l2y: String,
    l3y: String,
    l4y: String,
    shy: String,
    shh: String,
    rows: Vec<FlameRepo>,
}

/// Flame layout: the prototype's skeleton, widths now pro-rata real data.
const FLAME_W: f64 = 800.0;
const FLAME_H: f64 = 284.0;
const FLAME_X0: f64 = 16.0;
const FLAME_TRACK: f64 = 768.0;
const FLAME_TOPN: usize = 8;

/// Band word for a heat, same thresholds as `heat_color`.
fn heat_band(h: Option<f32>) -> &'static str {
    match h {
        Some(v) if v > 1.5 => "heating",
        Some(v) if v > 0.5 => "steady",
        Some(v) if v > 0.05 => "cooling",
        _ => "dormant",
    }
}

/// Flame rows: portfolio totals, top-8 repos by 30d share. Branch detail
/// rides the readout caption as text. Widths are honest against the
/// portfolio total — the tail gap is real and footnoted, never stretched.
fn flame_rows(graph_json: &str) -> anyhow::Result<FlameDoc> {
    let graph = jzombie_graph::graph::Graph::from_json(graph_json)?;
    let mut repos: Vec<&jzombie_graph::graph::Node> = graph
        .nodes
        .iter()
        .filter(|nd| !nd.private && jzombie_graph::graph::is_own_work(nd))
        .filter(|nd| nd.commits_30d > 0)
        .collect();
    repos.sort_by(|a, b| {
        b.commits_30d
            .cmp(&a.commits_30d)
            .then_with(|| a.name.cmp(&b.name))
    });
    let total: u32 = repos.iter().map(|n| n.commits_30d).sum();
    let empty = FlameDoc {
        header: flame_header(),
        legend: String::new(),
        footer: "</svg>".to_string(),
        port_label: String::new(),
        empty: true,
        fw: f1(FLAME_W),
        fh: f1(FLAME_H),
        msg: "No commits in trailing 30d".to_string(),
        muted: palette::muted(),
        tour_dur: String::new(),
        grads: String::new(),
        px: String::new(),
        py: String::new(),
        pw: String::new(),
        ph: String::new(),
        pty: String::new(),
        dx: f1(FLAME_X0),
        dy: f1(100.0),
        dw: f1(FLAME_TRACK),
        dh: f1(104.0),
        lx: f1(FLAME_X0 + 16.0),
        l1y: f1(130.0),
        l2y: f1(152.0),
        l3y: f1(168.0),
        l4y: f1(182.0),
        shy: f1(190.0),
        shh: f1(6.0),
        rows: vec![],
    };
    if total == 0 {
        return Ok(empty);
    }
    let cut = repos.len().saturating_sub(FLAME_TOPN);
    let shown = &repos[..repos.len().min(FLAME_TOPN)];
    let mut rows: Vec<FlameRepo> = vec![];
    let mut grads = String::new();
    for (ri, n) in shown.iter().enumerate() {
        let heat = n.commit_heat.or_else(|| n.commit_heat());
        let band = heat_band(heat);
        let share = n.commits_30d as f64 / total as f64;
        // About line: description trumps everything; truncate to one line.
        let desc = n.description.as_deref().unwrap_or("").replace('\n', " ");
        let desc = if desc.chars().count() > 120 {
            desc.chars().take(119).collect::<String>() + "\u{2026}"
        } else {
            desc
        };
        let lang = n.language.as_deref().unwrap_or("");
        let stats = match heat {
            Some(h) => format!(
                "{} commits \u{00b7} {:.1}% of portfolio \u{00b7} heat {h:.1} {band}{}",
                n.commits_30d,
                share * 100.0,
                if lang.is_empty() {
                    String::new()
                } else {
                    format!(" \u{00b7} {lang}")
                }
            ),
            None => format!(
                "{} commits \u{00b7} {:.1}% of portfolio \u{00b7} dormant{}",
                n.commits_30d,
                share * 100.0,
                if lang.is_empty() {
                    String::new()
                } else {
                    format!(" \u{00b7} {lang}")
                }
            ),
        };
        // Top branches ride the card as text.
        let mut tops: Vec<&jzombie_graph::graph::BranchHeat> =
            n.branch_heat.iter().filter(|b| b.commits_30d > 0).collect();
        tops.sort_by(|a, b| {
            b.commits_30d
                .cmp(&a.commits_30d)
                .then_with(|| a.branch.cmp(&b.branch))
        });
        let top_txt: String = tops
            .iter()
            .take(2)
            .map(|b| {
                let short = b.branch.rsplit('/').next().unwrap_or(&b.branch);
                format!("{} ({})", short, b.commits_30d)
            })
            .collect::<Vec<_>>()
            .join(", ");
        // Keywords: hash-joined topics, capped with a +N tail.
        let tags: String = if n.topics.is_empty() {
            String::new()
        } else {
            let shown: Vec<String> = n.topics.iter().take(6).map(|t| format!("#{t}")).collect();
            let mut s = shown.join(" ");
            if n.topics.len() > 6 {
                s.push_str(&format!(" +{}", n.topics.len() - 6));
            }
            s
        };
        let mut detail = if top_txt.is_empty() {
            String::new()
        } else {
            format!("top: {top_txt}")
        };
        if !tags.is_empty() {
            if !detail.is_empty() {
                detail.push_str(" \u{00b7} ");
            }
            detail.push_str(&tags);
        }
        // Recency wash: oldest→newest window marginals as stop offsets,
        // cool→hot left to right. Widths, not colors, carry the mix.
        let m5 = n.commits_5d.saturating_sub(n.commits_1d);
        let m15 = n.commits_15d.saturating_sub(n.commits_5d);
        let m30 = n.commits_30d.saturating_sub(n.commits_15d);
        let t = n.commits_30d.max(1) as f64;
        let (o1, o2, o3) = (
            m30 as f64 / t,
            (m30 + m15) as f64 / t,
            (m30 + m15 + m5) as f64 / t,
        );
        grads.push_str(&format!(
            "<linearGradient id=\"grad-r{ri}\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\"><stop offset=\"0\" stop-color=\"{}\" stop-opacity=\"0.55\"/><stop offset=\"{:.3}\" stop-color=\"{}\" stop-opacity=\"0.55\"/><stop offset=\"{:.3}\" stop-color=\"{}\" stop-opacity=\"0.55\"/><stop offset=\"{:.3}\" stop-color=\"{}\" stop-opacity=\"0.55\"/><stop offset=\"1\" stop-color=\"{}\" stop-opacity=\"0.55\"/></linearGradient>",
            palette::dormant(),
            o1,
            palette::cooling(),
            o2,
            palette::steady(),
            o3,
            palette::heating(),
            palette::heating()
        ));
        let title = match heat {
            Some(h) => format!(
                "{} \u{00b7} {} commits/30d \u{00b7} heat {h:.2} {band}",
                n.name, n.commits_30d
            ),
            None => format!(
                "{} \u{00b7} {} commits/30d \u{00b7} dormant",
                n.name, n.commits_30d
            ),
        };
        rows.push(FlameRepo {
            rid: format!("r{ri}"),
            nav: xml_escape(&n.name),
            tour_delay: if ri == 0 {
                "0s".to_string()
            } else {
                format!("-{}s", f1(ri as f64 * 4.0))
            },
            title: xml_escape(&title),
            desc: xml_escape(&desc),
            stats: xml_escape(&stats),
            detail: xml_escape(&detail),
            share_w: f1(share * FLAME_TRACK),
            color: heat_color(heat.unwrap_or(0.0)),
            pulse: palette::heat_pulse(heat).to_string(),
        });
    }
    let cut_c: u32 = repos[shown.len()..].iter().map(|n| n.commits_30d).sum();
    let sums = |f: fn(&jzombie_graph::graph::Node) -> u32| repos.iter().map(|n| f(n)).sum();
    let port_heat = jzombie_graph::graph::commit_heat_from_windows(
        sums(|n| n.commits_1d),
        sums(|n| n.commits_5d),
        sums(|n| n.commits_15d),
        sums(|n| n.commits_30d),
    );
    let port_label = match port_heat {
        Some(h) => format!(
            "portfolio \u{00b7} {total} commits / 30d \u{00b7} heat {h:.1} {}",
            heat_band(port_heat)
        ),
        None => format!("portfolio \u{00b7} {total} commits / 30d"),
    };
    let mut note = String::from("heat bands \u{00b7} cycling cards");
    if cut > 0 {
        note.push_str(&format!(" \u{00b7} +{cut} more \u{00b7} {cut_c} commits"));
    }
    Ok(FlameDoc {
        header: flame_header(),
        legend: flame_legend(&note),
        footer: "</svg>".to_string(),
        port_label: xml_escape(&port_label),
        empty: false,
        fw: f1(FLAME_W),
        fh: f1(FLAME_H),
        tour_dur: f1(shown.len() as f64 * 4.0),
        grads,
        msg: String::new(),
        muted: palette::muted(),
        px: f1(FLAME_X0),
        py: f1(52.0),
        pw: f1(FLAME_TRACK),
        ph: f1(34.0),
        pty: f1(74.0),
        dx: f1(FLAME_X0),
        dy: f1(100.0),
        dw: f1(FLAME_TRACK),
        dh: f1(104.0),
        lx: f1(FLAME_X0 + 16.0),
        l1y: f1(130.0),
        l2y: f1(152.0),
        l3y: f1(168.0),
        l4y: f1(182.0),
        shy: f1(190.0),
        shh: f1(6.0),
        rows,
    })
}

fn flame_header() -> String {
    format!(
        "<svg class=\"flame-svg\" xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" role=\"img\" font-family=\"{family}\"><title>Portfolio heat flame</title><text x=\"16\" y=\"24\" font-size=\"14\" font-weight=\"bold\" fill=\"{ink}\">portfolio heat</text><text x=\"16\" y=\"40\" class=\"muted\">one card per repo · heat bands · cycling</text>",
        f1(FLAME_W),
        f1(FLAME_H),
        ink = palette::portfolio_bar(),
        family = palette::family(),
    )
}

fn flame_legend(note: &str) -> String {
    format!(
        "<g font-size=\"12\" fill=\"{dim}\"><circle cx=\"24\" cy=\"220\" r=\"6\" fill=\"{hot}\"/><text x=\"36\" y=\"224\">heating &gt;1.5</text><circle cx=\"150\" cy=\"220\" r=\"6\" fill=\"{steady}\"/><text x=\"162\" y=\"224\">steady ~1.0</text><circle cx=\"268\" cy=\"220\" r=\"6\" fill=\"{cool}\"/><text x=\"280\" y=\"224\">cooling &lt;0.5</text><circle cx=\"392\" cy=\"220\" r=\"6\" fill=\"{dorm}\"/><text x=\"404\" y=\"224\">dormant</text><text x=\"16\" y=\"250\" class=\"muted\">{note}</text></g>",
        hot = palette::heating(),
        steady = palette::steady(),
        cool = palette::cooling(),
        dorm = palette::dormant(),
        dim = palette::dim(),
        note = xml_escape(note),
    )
}

/// Plugin function: validated graph JSON in, flame document out.
#[oxdock_func(pure, name = "FLAME")]
fn flame(graph_json: String) -> anyhow::Result<Value> {
    let graph_json = transport_out(&graph_json)?;
    Ok(Value::string(serde_json::to_string(&flame_rows(
        &graph_json,
    )?)?))
}

/// Render the heat flame for validated graph JSON. Same transport as the
/// pie: embedded script emits markup into a temp workspace.
pub fn render_flame_svg(graph_json: &str) -> anyhow::Result<String> {
    let engine = engine();
    let arg = transport_in(graph_json);
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS],
        IMPORT [CHARTS]
        IMPORT [STD]
        LET $doc_json: STRING = CHARTS::FLAME(#arg)
        WRITE rows.json $doc_json
        LET $doc: MAP<ANY> = LOAD_JSON("rows.json")
        LET $style: STRING = CHARTS::CHART_STYLE()
        WRITE out.svg $doc.header
        APPEND out.svg $style
        IF $doc.empty {
            APPEND out.svg "<text x=\"16\" y=\"60\" font-size=\"14\" fill=\"{{ $doc.muted }}\">{{ $doc.msg }}</text>"
        } ELSE {
            APPEND out.svg "<rect x=\"{{ $doc.px }}\" y=\"{{ $doc.py }}\" width=\"{{ $doc.pw }}\" height=\"{{ $doc.ph }}\" rx=\"4\" fill=\"#222\"/>"
            APPEND out.svg "<text x=\"28\" y=\"{{ $doc.pty }}\" class=\"lbl\">{{ $doc.port_label }}</text>"
            APPEND out.svg $doc.grads
            FOR $r: MAP<ANY> IN $doc.rows {
                APPEND out.svg "<g id=\"{{ $r.rid }}\" class=\"card {{ $r.pulse }}\" style=\"animation-duration:{{ $doc.tour_dur }}s;animation-delay:{{ $r.tour_delay }}\">"
                APPEND out.svg "<rect x=\"{{ $doc.dx }}\" y=\"{{ $doc.dy }}\" width=\"{{ $doc.dw }}\" height=\"{{ $doc.dh }}\" rx=\"4\" fill=\"url(#grad-{{ $r.rid }})\"><title>{{ $r.title }}</title></rect>"
                APPEND out.svg "<text x=\"{{ $doc.lx }}\" y=\"{{ $doc.l1y }}\" font-size=\"20\" font-weight=\"bold\" fill=\"#222\">{{ $r.nav }}</text>"
                APPEND out.svg "<text x=\"{{ $doc.lx }}\" y=\"{{ $doc.l2y }}\" class=\"dim\">{{ $r.desc }}</text>"
                APPEND out.svg "<text x=\"{{ $doc.lx }}\" y=\"{{ $doc.l3y }}\" class=\"sub\">{{ $r.stats }}</text>"
                APPEND out.svg "<text x=\"{{ $doc.lx }}\" y=\"{{ $doc.l4y }}\" class=\"muted\">{{ $r.detail }}</text>"
                APPEND out.svg "<rect x=\"{{ $doc.dx }}\" y=\"{{ $doc.shy }}\" width=\"{{ $r.share_w }}\" height=\"{{ $doc.shh }}\" fill=\"{{ $r.color }}\"/>"
                APPEND out.svg "</g>"
            }
            APPEND out.svg $doc.legend
        }
        APPEND out.svg $doc.footer
        LET $check: STRING = READ out.svg
        ASSERT_CONTAINS $check "<svg"
        ASSERT_CONTAINS $check "</svg>"
    };
    let temp = oxdock::GuardedPath::tempdir().expect("tempdir");
    let root = temp.as_guarded_path().clone();
    engine.run_steps_without_snapshot(&root, &steps)?;
    let resolver = oxdock::PathResolver::new(root.as_path(), root.as_path()).expect("resolver");
    let out = root.join("out.svg").expect("out path");
    Ok(resolver.read_to_string(&out).expect("read out"))
}

fn engine() -> Engine {
    let mut engine = Engine::new();
    engine.register_module(HostModule {
        name: "CHARTS".to_string(),
        funcs: vec![
            PieSlices::registration(),
            FocusBars::registration(),
            Indicators::registration(),
            Portfolio::registration(),
            ChartStyleFn::registration(),
            Flame::registration(),
        ],
        types: vec![],
        record_schemas: vec![],
        states: vec![],
    });
    engine
}

/// Render the commit-share pie for validated graph JSON. The embedded
/// script emits markup into a temp workspace; Rust reads it back and the
/// caller writes the bytes (so output paths stay in one place).
pub fn render_share_svg(graph_json: &str) -> anyhow::Result<String> {
    let engine = engine();
    let arg = transport_in(graph_json);
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS],
        IMPORT [STD, CHARTS]
        LET $rows_json: STRING = CHARTS::PIE_SLICES(#arg, 8)
        WRITE rows.json $rows_json
        LET $doc: MAP<ANY> = LOAD_JSON("rows.json")
        LET $style: STRING = CHARTS::CHART_STYLE()
        WRITE out.svg $doc.header
        APPEND out.svg $style
        IF $doc.empty {
            APPEND out.svg "<text x=\"{{ $doc.mx }}\" y=\"{{ $doc.my }}\" text-anchor=\"middle\" font-size=\"{{ $doc.fs }}\" fill=\"{{ $doc.muted }}\">{{ $doc.msg }}</text>"
        } ELSE {
            FOR $r: MAP<ANY> IN $doc.rows {
                IF $r.circle {
                    APPEND out.svg """
                        <circle cx="{{ $doc.cx }}" cy="{{ $doc.cy }}" r="{{ $doc.r }}" fill="{{ $r.color }}" class="{{ $r.pulse }}"><title>{{ $r.label }} {{ $r.pct }}</title></circle>
                        """
                } ELSE {
                    APPEND out.svg """
                        <path d="{{ $r.d }}" fill="{{ $r.color }}" class="{{ $r.pulse }}"><title>{{ $r.label }} {{ $r.pct }}</title></path>
                        """
                }
                APPEND out.svg """
                    <rect x="{{ $r.sx }}" y="{{ $r.sy }}" width="{{ $doc.cell }}" height="{{ $doc.cell }}" fill="{{ $r.color }}"/><text x="{{ $r.tx }}" y="{{ $r.ty }}" font-size="{{ $doc.fs }}" fill="{{ $doc.ink }}">{{ $r.label }} {{ $r.pct }}</text>
                    """
            }
        }
        APPEND out.svg $doc.footer
        LET $check: STRING = READ out.svg
        ASSERT_CONTAINS $check "<svg"
        ASSERT_CONTAINS $check "</svg>"
    };
    let temp = oxdock::GuardedPath::tempdir().expect("tempdir");
    let root = temp.as_guarded_path().clone();
    engine.run_steps_without_snapshot(&root, &steps)?;
    let resolver = oxdock::PathResolver::new(root.as_path(), root.as_path()).expect("resolver");
    let out = root.join("out.svg").expect("out path");
    Ok(resolver.read_to_string(&out).expect("read out"))
}

/// Status indicators: small round indicators with a CSS pulse. Rust picks the
/// sample set and precomputes color/rate; the script emits markup only.
/// Pure SVG + embedded `<style>` — no SMIL, no scripts — so the pulse runs
/// anywhere the file renders as an image, README embeds included.
/// Indicators carry no text: the table around them names the repo; `<title>`
/// keeps a hover tooltip.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct IndicatorRow {
    band: String,
    file: String,
    label: String,
    heat: String,
    color: String,
    core_opacity: String,
    ring: bool,
    /// Halo class (`halo`, `halo slow`, or empty).
    /// Heating pulses fast, steady slow, cooling+dormant static.
    pulse: String,
    cx: String,
}

/// Indicator document: dots in a row, one cell each. All geometry precomputed;
/// the script only interpolates.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct IndicatorDoc {
    header: String,
    footer: String,
    cy: String,
    dot_r: String,
    ring_w: String,
    rows: Vec<IndicatorRow>,
}

/// Table-sized cells: 44px, dot r8 (pulse peaks inside the cell).
const INDICATOR_CELL: f64 = 44.0;
const INDICATOR_CY: f64 = 22.0;
const INDICATOR_R: f64 = 8.0;

/// Heat color, stepped on the documented `commit_heat` bands: heating green,
/// steady blue, cooling amber, dormant faint gray.
/// All stops from the shared palette.
fn heat_color(h: f32) -> String {
    if h > 1.5 {
        palette::heating()
    } else if h > 0.5 {
        palette::steady()
    } else if h > 0.05 {
        palette::cooling()
    } else {
        palette::other()
    }
}

/// Reusable badges, one per heat band — not per repo. The table maps a
/// repo's heat to a band; the badge files stay fixed. Canonical heats sit
/// on the documented `commit_heat` points: heating well above, steady 1.0,
/// cooling low, dormant zero.
const BANDS: [(&str, f32); 4] = [
    ("heating", 3.0),
    ("steady", 1.0),
    ("cooling", 0.25),
    ("dormant", 0.0),
];

/// Band names in strip order, for the badge files.
pub fn indicator_names() -> Vec<String> {
    BANDS.iter().map(|(name, _)| name.to_string()).collect()
}

/// Shared row builder: one row per band, strip order.
fn indicator_rows() -> Vec<IndicatorRow> {
    BANDS
        .into_iter()
        .map(|(name, h)| {
            let pulse = if h > 1.5 {
                "halo".to_string()
            } else if h > 0.5 {
                "halo slow".to_string()
            } else {
                String::new()
            };
            let norm = (h / 6.0).clamp(0.0, 1.0);
            IndicatorRow {
                band: name.to_string(),
                file: format!("indicators/{name}.svg"),
                label: format!("{name} {h:.2}"),
                heat: format!("{h:.2}"),
                color: heat_color(h).to_string(),
                core_opacity: format!("{:.2}", 0.45 + 0.55 * norm as f64),
                ring: !pulse.is_empty(),
                pulse,
                cx: String::new(),
            }
        })
        .collect()
}

/// Plugin function: indicator document out. Bands are static by design —
/// the graph arg only keeps the call shape uniform with the other charts.
#[oxdock_func(pure, name = "INDICATORS")]
fn indicators(graph_json: String, only: String) -> anyhow::Result<Value> {
    let _ = graph_json;
    let rows: Vec<IndicatorRow> = indicator_rows()
        .into_iter()
        .filter(|r| only.is_empty() || r.band == only)
        .collect();
    let mut doc = IndicatorDoc {
        header: String::new(),
        footer: "</svg>".to_string(),
        cy: f1(INDICATOR_CY),
        dot_r: f1(INDICATOR_R),
        ring_w: "2".to_string(),
        rows,
    };
    for (i, r) in doc.rows.iter_mut().enumerate() {
        r.cx = f1(i as f64 * INDICATOR_CELL + INDICATOR_CELL / 2.0);
    }
    doc.header = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" role=\"img\" font-family=\"{family}\"><title>Status indicators</title>",
        f1(doc.rows.len() as f64 * INDICATOR_CELL),
        f1(INDICATOR_CELL),
        family = palette::family(),
    );
    Ok(Value::string(serde_json::to_string(&doc)?))
}

/// Portfolio page document: fragment + chart embeds + the indicator table
/// (rendered by its own template, passed in — one row-template total).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct PortfolioDoc {
    header: String,
    table: String,
    footer: String,
}

/// Plugin function: portfolio page out. Static bands, caller-supplied
/// fragment and image names — no paths live in any template or script.
#[oxdock_func(pure, name = "PORTFOLIO")]
fn portfolio(
    frag: String,
    share: String,
    focus: String,
    flame: String,
    table: String,
) -> anyhow::Result<Value> {
    let frag = transport_out(&frag)?;
    let header = format!(
        "# portfolio-graph — render proof\n\n*Generated — do not hand-edit. Regenerate with `oxdock scripts/preview-indicators.oxfile`. Fragment carried verbatim; charts embedded as files.*\n\n## Map fragment\n\n{frag}\n## Commit share\n\n![share]({share})\n\n## Focus ranking\n\n![focus]({focus})\n\n## Heat flame\n\n![flame]({flame})\n\n",
        frag = frag.trim_end(),
        share = share,
        focus = focus,
        flame = flame,
    );
    let doc = PortfolioDoc {
        header,
        table,
        footer: String::new(),
    };
    Ok(Value::string(serde_json::to_string(&doc)?))
}

/// Render the indicator strip: all four bands in one row. Graph-free —
/// bands are static; the strip is the look-check.
pub fn render_indicators_svg() -> anyhow::Result<String> {
    render_indicator_doc("")
}

/// Render one badge by band name (`heating`/`steady`/`cooling`/`dormant`).
/// Unknown bands are an error, not an empty file.
pub fn render_indicator_svg(band: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        BANDS.iter().any(|(name, _)| *name == band),
        "unknown indicator band: {band}"
    );
    render_indicator_doc(band)
}

/// Render the preview table: badge embeds + band + heat, one row per band.
/// Same doc as the SVGs, markdown template — the committed render proof.
/// Static header in Rust; only the rows ride the template.
pub fn render_indicators_md() -> anyhow::Result<String> {
    let engine = engine();
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS],
        IMPORT [STD, CHARTS]
        LET $rows_json: STRING = CHARTS::INDICATORS("", "")
        WRITE rows.json $rows_json
        LET $doc: MAP<ANY> = LOAD_JSON("rows.json")
        WRITE md.md """
            # Indicator preview

            *Generated — do not hand-edit. Regenerate with `oxdock scripts/preview-indicators.oxfile`. Badges sit beside this file; the rows come from the same template doc as the SVGs.*

            | badge | band | heat |
            |---|---|---|
            """
        APPEND md.md "\n"
        FOR $r: MAP<ANY> IN $doc.rows {
            // TODO: APPEND_LINE adds exactly one LF (verified absent from
            // engine 0.35.0-alpha) — switch when we track past it.
            APPEND md.md "| ![{{ $r.band }}]({{ $r.file }}) | {{ $r.band }} | {{ $r.heat }} |\n"
        }
        LET $check: STRING = READ md.md
        ASSERT_CONTAINS $check "| badge |"
    };
    let temp = oxdock::GuardedPath::tempdir().expect("tempdir");
    let root = temp.as_guarded_path().clone();
    engine.run_steps_without_snapshot(&root, &steps)?;
    let resolver = oxdock::PathResolver::new(root.as_path(), root.as_path()).expect("resolver");
    let out = root.join("md.md").expect("md path");
    Ok(resolver.read_to_string(&out).expect("read md"))
}

/// Render the full proof page: fragment + embeds + indicator table.
/// One OxDock template, zero path literals — names arrive as arguments.
pub fn render_preview_md(
    frag: &str,
    share: &str,
    focus: &str,
    flame: &str,
    table: &str,
) -> anyhow::Result<String> {
    let engine = engine();
    let (frag_arg, share_arg, focus_arg, flame_arg, table_arg) = (
        transport_in(frag),
        share.to_string(),
        focus.to_string(),
        flame.to_string(),
        table.to_string(),
    );
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS],
        IMPORT [STD, CHARTS]
        LET $page_json: STRING = CHARTS::PORTFOLIO(#frag_arg, #share_arg, #focus_arg, #flame_arg, #table_arg)
        WRITE rows.json $page_json
        LET $doc: MAP<ANY> = LOAD_JSON("rows.json")
        WRITE md.md $doc.header
        APPEND md.md $doc.table
        LET $check: STRING = READ md.md
        ASSERT_CONTAINS $check "| badge |"
    };
    let temp = oxdock::GuardedPath::tempdir().expect("tempdir");
    let root = temp.as_guarded_path().clone();
    engine.run_steps_without_snapshot(&root, &steps)?;
    let resolver = oxdock::PathResolver::new(root.as_path(), root.as_path()).expect("resolver");
    let out = root.join("md.md").expect("md path");
    Ok(resolver.read_to_string(&out).expect("read md"))
}

fn render_indicator_doc(only: &str) -> anyhow::Result<String> {
    let engine = engine();
    let only_arg = only.to_string();
    let graph_json = String::new();
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS],
        IMPORT [STD, CHARTS]
        LET $rows_json: STRING = CHARTS::INDICATORS(#graph_json, #only_arg)
        WRITE rows.json $rows_json
        LET $doc: MAP<ANY> = LOAD_JSON("rows.json")
        LET $style: STRING = CHARTS::CHART_STYLE()
        WRITE out.svg $doc.header
        APPEND out.svg $style
        FOR $r: MAP<ANY> IN $doc.rows {
            IF $r.ring {
                APPEND out.svg """
                    <circle cx="{{ $r.cx }}" cy="{{ $doc.cy }}" r="{{ $doc.dot_r }}" fill="none" stroke="{{ $r.color }}" stroke-width="{{ $doc.ring_w }}" class="{{ $r.pulse }}"><title>{{ $r.label }}</title></circle>
                    """
            } ELSE {
                APPEND out.svg """
                    <circle cx="{{ $r.cx }}" cy="{{ $doc.cy }}" r="{{ $doc.dot_r }}" fill="none" stroke="{{ $r.color }}" stroke-width="{{ $doc.ring_w }}"><title>{{ $r.label }}</title></circle>
                    """
            }
            APPEND out.svg """
                <circle cx="{{ $r.cx }}" cy="{{ $doc.cy }}" r="{{ $doc.dot_r }}" fill="{{ $r.color }}" opacity="{{ $r.core_opacity }}"><title>{{ $r.label }}</title></circle>
                """
        }
        APPEND out.svg $doc.footer
        LET $check: STRING = READ out.svg
        ASSERT_CONTAINS $check "<svg"
        ASSERT_CONTAINS $check "</svg>"
    };
    let temp = oxdock::GuardedPath::tempdir().expect("tempdir");
    let root = temp.as_guarded_path().clone();
    engine.run_steps_without_snapshot(&root, &steps)?;
    let resolver = oxdock::PathResolver::new(root.as_path(), root.as_path()).expect("resolver");
    let out = root.join("out.svg").expect("out path");
    Ok(resolver.read_to_string(&out).expect("read out"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_with(commits: &[(&str, u32)]) -> String {
        let pairs: Vec<(&str, u32, u32)> = commits.iter().map(|(n, c)| (*n, *c, *c * 6)).collect();
        graph_with_heat(&pairs)
    }

    /// Explicit (5d, 30d) windows so heat is controlled, not derived.
    fn graph_with_heat(pairs: &[(&str, u32, u32)]) -> String {
        use jzombie_graph::graph::{Graph, Layer, Node};
        let nodes = pairs
            .iter()
            .map(|(name, c5, c30)| Node {
                id: format!("me/{name}"),
                owner: "me".into(),
                name: (*name).into(),
                fork: false,
                private: false,
                upstream: None,
                ahead_by: None,
                pushed_at: None,
                description: None,
                topics: vec![],
                language: Some("Rust".into()),
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
                commits_1d: *c5 / 5,
                commits_5d: *c5,
                commits_15d: *c5 * 3,
                commits_30d: *c30,
                commit_heat: None,
                focus: None,
                last_commit_at: None,
                branch_heat: vec![],
                recent_prs: vec![],
                visible: true,
                visible_reason: None,
                layer: Layer::Isolated,
            })
            .collect();
        jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap()
    }

    fn graph_with_focus<S: AsRef<str>>(scores: &[(S, Option<f32>)]) -> String {
        use jzombie_graph::graph::{Graph, Layer, Node};
        let nodes = scores
            .iter()
            .map(|(name, f)| Node {
                id: format!("me/{}", name.as_ref()),
                owner: "me".into(),
                name: name.as_ref().into(),
                fork: false,
                private: false,
                upstream: None,
                ahead_by: None,
                pushed_at: None,
                description: None,
                topics: vec![],
                language: Some("Rust".into()),
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
                focus: *f,
                last_commit_at: None,
                branch_heat: vec![],
                visible: true,
                recent_prs: vec![],
                visible_reason: None,
                layer: Layer::Isolated,
            })
            .collect();
        jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap()
    }

    #[test]
    fn rows_rank_top_n_plus_other_and_conserve() {
        let g = graph_with(&[("a", 10), ("b", 7), ("c", 3), ("d", 1), ("e", 0)]);
        let rows = share_rows(&g, 2).unwrap();
        assert_eq!(rows.len(), 3); // a, b, Other(c+d)
        assert_eq!(rows[0].label, "a");
        assert_eq!(rows[2].label, "Other");
        // Zero-commit repos never slice.
        assert!(!rows.iter().any(|r| r.label == "e"));
        // Shares conserve: 10/21 + 7/21 + 4/21.
        assert!(rows[0].pct.starts_with("47.6"));
        assert!(!rows[0].circle);
        assert!(rows[0].d.starts_with("M ") && rows[0].d.ends_with(" Z"));
    }

    #[test]
    fn empty_window_yields_no_rows() {
        let g = graph_with(&[("a", 0)]);
        assert!(share_rows(&g, 8).unwrap().is_empty());
    }

    #[test]
    fn single_contributor_is_a_circle() {
        let g = graph_with(&[("solo", 5)]);
        let rows = share_rows(&g, 8).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].circle);
        assert_eq!(rows[0].pct, "100.0%");
    }

    #[test]
    fn labels_escape_xml() {
        // Repo names can't contain these, but the emitter must be safe anyway.
        assert_eq!(xml_escape("a&b<c>"), "a&amp;b&lt;c&gt;");
        assert_eq!(f1(12.345), "12.3");
    }

    fn focus_val(rows: &[FocusRow], label: &str) -> f32 {
        rows.iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("missing row {label}"))
            .value
            .parse()
            .unwrap()
    }

    #[test]
    fn focus_ranks_descending_unknowns_excluded() {
        let g = graph_with_focus(&[
            ("hot", Some(3.0)),
            ("warm", Some(0.43)),
            ("flat", Some(0.0)),
            ("cool", Some(-0.2)),
            ("stale", Some(-8.5)),
            ("ancient", Some(-54.83)),
            ("mystery", None),
        ]);
        let rows = focus_rows(&g, 12, 5).unwrap();
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        // Descending, unknowns excluded (can't sit on a scale).
        assert_eq!(
            labels,
            vec!["hot", "warm", "flat", "cool", "stale", "ancient"]
        );
        assert!((focus_val(&rows, "ancient") - -54.83).abs() < 1e-6);
    }

    #[test]
    fn elbow_cuts_cliff_keeps_smooth() {
        // Cliff dwarfs the rest: cut.
        assert_eq!(elbow_keep(&[-0.2, -0.23, -0.3, -57.0, -60.0]), 3);
        // Smooth slope: no cut, even though the last gap is biggest.
        assert_eq!(elbow_keep(&[-0.2, -0.5, -0.9, -1.5]), 4);
        // Too few points to elbow.
        assert_eq!(elbow_keep(&[-1.0, -2.0]), 2);
    }

    #[test]
    fn focus_drops_abyss_below_elbow() {
        let g = graph_with_focus(&[
            ("hot", Some(2.9)),
            ("warm", Some(0.36)),
            ("a", Some(-0.2)),
            ("b", Some(-0.25)),
            ("c", Some(-0.3)),
            ("abyss1", Some(-57.0)),
            ("abyss2", Some(-60.0)),
        ]);
        let rows = focus_rows(&g, 12, 5).unwrap();
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, vec!["hot", "warm", "a", "b", "c"]);
    }
    #[test]
    fn indicators_pulse_with_heat_static_when_dormant() {
        // Stepped ramp on the documented heat bands: heating green,
        // steady blue, cooling amber (still pulses), dormant faint.
        assert_eq!(heat_color(6.0), palette::heating());
        assert_eq!(heat_color(1.0), palette::steady());
        assert_eq!(heat_color(0.3), palette::cooling());
        assert_eq!(heat_color(0.0), palette::other());
    }

    #[test]
    fn indicators_render_shared_breathe() {
        let strip = render_indicators_svg().unwrap();
        assert!(strip.starts_with("<svg "));
        assert!(strip.ends_with("</svg>"));
        // One shared style block — no SMIL, no scripts, no text:
        // the table around the badge names the repo.
        assert!(strip.contains("@keyframes halo"));
        assert!(strip.contains("@keyframes breathe"));
        assert!(!strip.contains("@keyframes ping"));
        assert!(!strip.contains("style=\"animation-duration"));
        assert!(!strip.contains("<animate"));
        assert!(!strip.contains("<script"));
        assert!(!strip.contains("<text"));
        // All four bands present; only heating + steady move.
        for band in ["heating", "steady", "cooling", "dormant"] {
            assert!(strip.contains(band), "missing {band}");
        }
        assert_eq!(strip.matches("halo slow").count(), 1);
        assert_eq!(strip.matches("halo\"").count(), 1);
    }

    #[test]
    fn chart_style_is_the_single_source() {
        // The DSL file owns the tokens; Rust only extracts the payload.
        let css = chart_style().unwrap();
        assert!(css.starts_with("<style>"));
        assert!(css.ends_with("</style>"));
        for token in [
            ".bar{stroke:#fff;stroke-width:2;rx:4}",
            ".pulse{animation:breathe",
            ".slow{animation-duration:3.4s}",
            "@keyframes breathe",
            "prefers-reduced-motion",
        ] {
            assert!(css.contains(token), "missing {token}");
        }
        // Every chart embeds this exact block.
        let g = graph_with(&[("a", 10)]);
        for svg in [
            render_share_svg(&g).unwrap(),
            render_focus_svg(&graph_with_focus(&[("a", Some(2.0))])).unwrap(),
            render_indicators_svg().unwrap(),
            render_flame_svg(&g).unwrap(),
        ] {
            assert!(svg.contains(&css), "chart missing shared style");
        }
    }

    #[test]
    fn token_set_matches_style_oxfile() {
        let mut names: Vec<&str> = super::palette::token_names();
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                "cat3",
                "cat4",
                "cat5",
                "cat6",
                "cat7",
                "cooling",
                "dim",
                "dormant",
                "family",
                "heating",
                "ink",
                "ink_on_bar",
                "portfolio_bar",
                "steady",
            ]
        );
    }

    #[test]
    fn transport_survives_quotes_and_backslashes() {
        // PR titles brought `"` into the data and broke `#var`
        // interpolation (raw inline). Base64 has no escapes to break.
        let raw = "say \"hi\" \\ bye";
        let back = transport_out(&transport_in(raw)).unwrap();
        assert_eq!(back, raw);
    }

    #[test]
    fn render_survives_quotes_in_data() {
        // End to end through the template: quotes/backslashes in repo
        // text must not break script parsing (the TPC-H title did).
        // Description rides along unrendered (it only needs to survive);
        // the quoted name proves interpolation is intact.
        let mut v: serde_json::Value = serde_json::from_str(&graph_with(&[("q", 5)])).unwrap();
        v["nodes"][0]["description"] = serde_json::Value::String("Add TPC-H post".into());
        v["nodes"][0]["name"] = serde_json::Value::String("qu\"oted".into());
        v["nodes"][0]["id"] = serde_json::Value::String("me/qu\"oted".into());
        let g = serde_json::to_string(&v).unwrap();
        let svg = render_share_svg(&g).unwrap();
        assert!(svg.contains("qu&quot;oted"));
        let svg = render_flame_svg(&g).unwrap();
        assert!(svg.contains("qu&quot;oted"));
    }

    #[test]
    fn heat_pulse_steps_on_documented_bands() {
        assert_eq!(palette::heat_pulse(Some(3.0)), "pulse");
        assert_eq!(palette::heat_pulse(Some(1.0)), "pulse slow");
        assert_eq!(palette::heat_pulse(Some(0.3)), "");
        assert_eq!(palette::heat_pulse(Some(0.0)), "");
        assert_eq!(palette::heat_pulse(None), "");
        assert_eq!(heat_band(Some(3.0)), "heating");
        assert_eq!(heat_band(Some(1.0)), "steady");
        assert_eq!(heat_band(Some(0.3)), "cooling");
        assert_eq!(heat_band(None), "dormant");
    }

    #[test]
    fn tour_cycles_every_repo() {
        // One slot per repo, 4s each; delays stagger the shared whisk.
        let g = graph_with(&[("big", 30), ("small", 10)]);
        let doc = flame_rows(&g).unwrap();
        assert_eq!(doc.tour_dur, "8.0");
        assert_eq!(doc.rows[0].tour_delay, "0s");
        assert_eq!(doc.rows[1].tour_delay, "-4.0s");
        let svg = render_flame_svg(&g).unwrap();
        assert!(svg.contains("@keyframes whisk"));
        assert!(svg.contains("animation-delay:-4.0s"));
        assert!(!svg.contains("@keyframes acc"));
        // Reduced-motion freezes the deck on the lead card.
        assert!(svg.contains(".pulse,.halo,.card{animation:none}"));
        // Every repo gets a card with its readout, however small.
        assert!(svg.contains(">small<"));
        assert!(svg.contains("60 commits"));
        // Recency gradients: oldest window dormant, newest heating.
        assert!(svg.contains("grad-r0"));
        assert!(svg.contains("stop-color=\"#009E73\""));
    }

    #[test]
    fn flame_rows_are_pro_rata_and_honest() {
        // Two repos, 3:1 split (c30 derives 6x c5: 180 + 60).
        let g = graph_with(&[("big", 30), ("small", 10)]);
        let doc = flame_rows(&g).unwrap();
        assert!(!doc.empty);
        assert_eq!(doc.rows.len(), 2);
        // Share bars stay pro-rata even though cards are full-width.
        let w0: f64 = doc.rows[0].share_w.parse().unwrap();
        let w1: f64 = doc.rows[1].share_w.parse().unwrap();
        assert!((w0 - 576.0).abs() < 0.2, "{w0}");
        assert!((w1 - 192.0).abs() < 0.2, "{w1}");
        // Portfolio label carries the total, not the shown sum.
        assert!(doc.port_label.contains("240 commits / 30d"));
        // Tiny repos still get a full card with their readout.
        let thin = graph_with(&[("huge", 100), ("sliver", 1)]);
        let doc = flame_rows(&thin).unwrap();
        assert_eq!(doc.rows.len(), 2);
        assert_eq!(doc.rows[1].tour_delay, "-4.0s");
    }

    #[test]
    fn flame_renders_shared_style_no_smoke() {
        let g = graph_with(&[("big", 30), ("small", 10)]);
        let svg = render_flame_svg(&g).unwrap();
        assert!(svg.starts_with("<svg "));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("@keyframes breathe"));
        assert!(!svg.contains("<animate"));
        assert!(!svg.contains("<script"));
        // Portfolio track + one card per repo, each with its readout.
        assert!(svg.contains("portfolio · 240 commits / 30d"));
        assert!(svg.contains(">big</text>"));
        assert!(svg.contains("heating &gt;1.5"));
    }

    #[test]
    fn indicators_md_rows_are_newline_separated() {
        let md = render_indicators_md().unwrap();
        assert!(md.contains("| badge | band | heat |\n|---|---|---|\n"));
        for band in ["heating", "steady", "cooling", "dormant"] {
            assert!(md.contains(&format!("| ![{band}](indicators/{band}.svg) | {band} |")));
        }
        assert!(md.ends_with("|\n"));
    }

    #[test]
    fn preview_page_carries_everything_verbatim() {
        let table = render_indicators_md().unwrap();
        let md = render_preview_md("# Frag\n", "s.svg", "f.svg", "fl.svg", &table).unwrap();
        // Fragment verbatim, embeds by file name, indicator rows inlined.
        assert!(md.contains("# Frag\n"));
        assert!(md.contains("![share](s.svg)"));
        assert!(md.contains("![focus](f.svg)"));
        assert!(md.contains("![flame](fl.svg)"));
        assert!(md.contains("| ![heating](indicators/heating.svg) | heating | 3.00 |"));
        assert!(md.contains("oxdock scripts/preview-indicators.oxfile"));
        assert!(md.ends_with("|\n"));
    }

    #[test]
    fn indicators_single_is_one_badge() {
        let one = render_indicator_svg("steady").unwrap();
        // Ring + core, one 44px cell, tooltip kept, unknown bands error.
        assert_eq!(one.matches("<circle").count(), 2);
        assert!(one.contains("viewBox=\"0 0 44.0 44.0\""));
        assert!(one.contains("steady 1.00"));
        assert!(render_indicator_svg("volcanic").is_err());
    }

    #[test]
    fn focus_top_plus_bottom_never_duplicates() {
        let g = graph_with_focus(&[("a", Some(2.0)), ("b", Some(-1.0))]);
        // More slots than repos: every repo exactly once.
        let rows = focus_rows(&g, 12, 5).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "a");
        assert_eq!(rows[1].label, "b");
    }

    #[test]
    fn focus_continues_ranking_below_top() {
        let scores: Vec<(String, Option<f32>)> = (0..20)
            .map(|i| (format!("r{i:02}"), Some(10.0 - i as f32)))
            .collect();
        let g = graph_with_focus(&scores);
        let rows = focus_rows(&g, 12, 5).unwrap();
        // Top 12 by score, then the next 5 contiguous — one smooth ranking.
        assert_eq!(rows.len(), 17);
        assert_eq!(rows[0].label, "r00");
        assert_eq!(rows[11].label, "r11");
        assert_eq!(rows[12].label, "r12");
        assert_eq!(rows[16].label, "r16");
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(labels.len(), sorted.len());
    }

    #[test]
    fn focus_bars_diverge_from_zero_by_sign() {
        let g = graph_with_focus(&[("up", Some(2.0)), ("down", Some(-4.0))]);
        let rows = focus_rows(&g, 12, 5).unwrap();
        // Fixed tabular columns: names end at 130, values end at 200,
        // zero axis at 260 — text over the page stays gray.
        for r in &rows {
            let tx: f64 = r.tx.parse().unwrap();
            let vx: f64 = r.vx.parse().unwrap();
            assert!((tx - 130.0).abs() < 1e-6);
            assert!((vx - 200.0).abs() < 1e-6);
            assert_eq!(r.vanchor, "end");
        }
        let up = rows.iter().find(|r| r.label == "up").unwrap();
        let down = rows.iter().find(|r| r.label == "down").unwrap();
        let up_bw: f64 = up.bw.parse().unwrap();
        let down_bx: f64 = down.bx.parse().unwrap();
        let down_bw: f64 = down.bw.parse().unwrap();
        // Positive starts at zero and extends right; negative ends at zero.
        let up_bx: f64 = up.bx.parse().unwrap();
        assert!((up_bx - 260.0).abs() < 1e-6);
        assert!((down_bx + down_bw - 260.0).abs() < 1e-6);
        // Each side fills its own half against its own max: both bars span
        // the full half-width despite |−4| ≠ |2|.
        assert!((up_bw - 200.0).abs() < 1e-6);
        assert!((down_bw - 200.0).abs() < 1e-6);
        // Long names truncate; short ones pass through.
        assert_eq!(truncate_name("short", LABEL_MAX), "short");
        assert!(truncate_name(&"x".repeat(40), LABEL_MAX).ends_with('…'));
    }

    #[test]
    fn focus_ink_splits_by_crossing() {
        // Two-pass ink: gray text throughout, dark re-drawn clipped to each
        // bar rect — clip ids unique per row, both passes present in markup.
        let g = graph_with_focus(&[
            ("top", Some(10.0)),
            ("mid", Some(-2.0)),
            ("low", Some(-10.0)),
        ]);
        let rows = focus_rows(&g, 12, 5).unwrap();
        let mut clips: Vec<&str> = rows.iter().map(|r| r.clip.as_str()).collect();
        clips.sort_unstable();
        clips.dedup();
        assert_eq!(clips.len(), rows.len());
        let svg = render_focus_svg(&g).unwrap();
        assert!(svg.contains("<clipPath"));
        for r in &rows {
            assert!(svg.contains(&format!("url(#{})", r.clip)));
        }
        assert!(svg.contains(palette::ink().as_str()));
        assert!(svg.contains(palette::ink_on_bar().as_str()));
    }

    fn mixed_ownership_graph() -> String {
        use jzombie_graph::graph::{Graph, Layer, Node};
        let mk =
            |name: &str, fork: bool, upstream: Option<&str>, ahead: Option<u32>, private: bool| {
                Node {
                    id: format!("me/{name}"),
                    owner: "me".into(),
                    name: name.into(),
                    fork,
                    private,
                    upstream: upstream.map(str::to_string),
                    ahead_by: ahead,
                    pushed_at: None,
                    description: None,
                    topics: vec![],
                    language: Some("Rust".into()),
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
                    commits_1d: 1,
                    commits_5d: 4,
                    commits_15d: 12,
                    commits_30d: 24,
                    commit_heat: None,
                    focus: Some(1.0),
                    last_commit_at: None,
                    branch_heat: vec![],
                    visible: true,
                    visible_reason: None,
                    recent_prs: vec![],
                    layer: Layer::Isolated,
                }
            };
        let nodes = vec![
            mk("mine", false, None, None, false),
            mk("pristine", true, Some("other/pristine"), Some(0), false),
            mk("diverged", true, Some("other/diverged"), Some(3), false),
            mk("renamed", true, Some("other/upstream-name"), Some(0), false),
            mk("secret", false, None, None, true),
        ];
        jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap()
    }

    #[test]
    fn charts_show_only_own_work() {
        // Pristine forks (nothing contributed) and private repos never
        // chart, even with commits and focus. Diverged or renamed forks do.
        // `visible` is deliberately ignored here: it answers what the map
        // draws, not what you worked on (all fixtures mark visible).
        let g = mixed_ownership_graph();
        let pie = share_rows(&g, 8).unwrap();
        let pie_names: Vec<&str> = pie.iter().map(|r| r.label.as_str()).collect();
        assert!(pie_names.contains(&"mine"));
        assert!(pie_names.contains(&"diverged"));
        assert!(pie_names.contains(&"renamed"));
        assert!(!pie_names.contains(&"pristine"));
        assert!(!pie_names.contains(&"secret"));
        let focus = focus_rows(&g, 12, 5).unwrap();
        let focus_names: Vec<&str> = focus.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(focus_names, pie_names);
    }
}
