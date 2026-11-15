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
    let text = STYLE_OXFILE;
    let start = text
        .find("RETURN \"\"\"")
        .map(|i| i + "RETURN \"\"\"".len())
        .ok_or_else(|| anyhow::anyhow!("CHART_STYLE payload missing in style.oxfile"))?;
    let end = text[start..]
        .find("\"\"\"")
        .map(|i| start + i)
        .ok_or_else(|| anyhow::anyhow!("CHART_STYLE payload unterminated"))?;
    // Strip the common indent so the file can be indented for humans
    // while every consumer gets the same canonical block.
    let body = text[start..end].trim();
    let indent = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    Ok(body
        .lines()
        .map(|l| l.get(indent..).unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string())
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
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" role=\"img\" font-family=\"{family}\"><title>Developer focus</title><text x=\"16\" y=\"30\" font-size=\"20\" fill=\"{ink}\">Developer focus</text>",
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

/// One window edge: x position plus which edge it is (`15d`, `5d`,
/// `1d`). Interior boundaries only — the card edges need no line.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct FlameStop {
    x: String,
    label: String,
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
    pulse: String,
    stops: Vec<FlameStop>,
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
    /// Right track end (left end is `px`): the frame never moves, so the
    /// `30d ago → today` endpoint labels are static while per-card ticks
    /// travel with their card.
    ax1: String,
    dx: String,
    dy: String,
    dw: String,
    dh: String,
    lx: String,
    l1y: String,
    l2y: String,
    l3y: String,
    l4y: String,
    /// Window-edge marker geometry: label row above the cards, tick
    /// bottom just inside the card top.
    tick_ly: String,
    tick_by: String,
    rows: Vec<FlameRepo>,
}

/// Flame layout: cards share one full-track width; the per-card gradient
/// wash carries the mix (oldest commits left, newest right), and marker
/// ticks name the 1d/5d/15d window edges on each card.
const FLAME_W: f64 = 800.0;
const FLAME_H: f64 = 262.0;
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
        ax1: String::new(),
        dx: f1(FLAME_X0),
        dy: f1(100.0),
        dw: f1(FLAME_TRACK),
        dh: f1(104.0),
        lx: f1(FLAME_X0 + 16.0),
        l1y: f1(130.0),
        l2y: f1(152.0),
        l3y: f1(168.0),
        l4y: f1(182.0),
        tick_ly: String::new(),
        tick_by: String::new(),
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
        let detail = tags;
        // Recency wash: oldest→newest window marginals as stop offsets,
        // cool→hot left to right. Widths, not colors, carry the mix.
        let m5 = n.commits_5d.saturating_sub(n.commits_1d);
        let m15 = n.commits_15d.saturating_sub(n.commits_5d);
        let m30 = n.commits_30d.saturating_sub(n.commits_15d);
        let t = n.commits_30d.max(1) as f64;
        // Collapse zero-width buckets: adjacent duplicates draw
        // as hard edges, so only real spans get stops.
        let m1 = n.commits_1d;
        let mut stops: Vec<(f64, String)> = vec![(0.0, palette::dormant())];
        let mut acc = 0.0;
        for (m, c) in [
            (m30, palette::dormant()),
            (m15, palette::cooling()),
            (m5, palette::steady()),
            (m1, palette::heating()),
        ] {
            acc += m as f64 / t;
            if acc > stops.last().map(|(o, _)| *o).unwrap_or(0.0) + 1e-9 {
                stops.push((acc.min(1.0), c));
            }
        }
        grads.push_str(&format!(
            "<linearGradient id=\"grad-r{ri}\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\">"
        ));
        for (o, c) in &stops {
            grads.push_str(&format!(
                "<stop offset=\"{:.3}\" stop-color=\"{c}\" stop-opacity=\"0.55\"/>",
                o.min(1.0)
            ));
        }
        grads.push_str("</linearGradient>");
        // Window-edge markers: the same bucket boundaries as the gradient
        // above, but only interior ones (card edges need no line). Each
        // line says where a band stops, so the card reads without
        // forensics; the stats line below still carries exact numbers.
        let mut markers: Vec<FlameStop> = vec![];
        let mut macc = 0.0;
        let mut mlast = 0.0;
        for (m, edge) in [(m30, "15d"), (m15, "5d"), (m5, "1d")] {
            macc += m as f64 / t;
            if macc > mlast + 1e-9 && macc < 1.0 - 1e-9 {
                let x = FLAME_X0 + macc * FLAME_TRACK;
                // Near the right edge the static `today` label owns the
                // row: the tick still draws, its text yields to it.
                let label = if x > FLAME_X0 + FLAME_TRACK - 34.0 {
                    String::new()
                } else {
                    edge.to_string()
                };
                markers.push(FlameStop { x: f1(x), label });
                mlast = macc;
            }
        }
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
        let title = if top_txt.is_empty() {
            title
        } else {
            format!("{title} \u{00b7} top: {top_txt}")
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
            pulse: palette::heat_pulse(heat).to_string(),
            stops: markers,
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
    let mut note = String::from(
        "color = pace vs own usual (1.0 = usual); bands run oldest → newest, lines mark the 1d / 5d / 15d window edges",
    );
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
        ax1: f1(FLAME_X0 + FLAME_TRACK),
        dx: f1(FLAME_X0),
        dy: f1(100.0),
        dw: f1(FLAME_TRACK),
        dh: f1(104.0),
        lx: f1(FLAME_X0 + 16.0),
        l1y: f1(130.0),
        l2y: f1(152.0),
        l3y: f1(168.0),
        l4y: f1(182.0),
        tick_ly: f1(96.0),
        tick_by: f1(110.0),
        rows,
    })
}

fn flame_header() -> String {
    format!(
        "<svg class=\"flame-svg\" xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {W} {H}\" role=\"img\" font-family=\"{family}\"><title>Portfolio commit activity</title><text x=\"16\" y=\"24\" font-size=\"14\" font-weight=\"bold\" fill=\"{ink}\">portfolio activity</text><text x=\"16\" y=\"40\" class=\"muted\">one card per repo · heat bands · cycling</text>",
        W = f1(FLAME_W),
        H = f1(FLAME_H),
        ink = palette::portfolio_bar(),
        family = palette::family(),
    )
}

fn flame_legend(note: &str) -> String {
    format!(
        "<g font-size=\"12\" fill=\"#9198a1\"><circle cx=\"24\" cy=\"220\" r=\"6\" fill=\"{hot}\"/><text x=\"36\" y=\"224\">heating &gt;1.5</text><circle cx=\"150\" cy=\"220\" r=\"6\" fill=\"{steady}\"/><text x=\"162\" y=\"224\">steady ~1.0</text><circle cx=\"268\" cy=\"220\" r=\"6\" fill=\"{cool}\"/><text x=\"280\" y=\"224\">cooling &lt;0.5</text><circle cx=\"392\" cy=\"220\" r=\"6\" fill=\"{dorm}\"/><text x=\"404\" y=\"224\">dormant</text><text x=\"16\" y=\"250\" font-size=\"11\" fill=\"#9198a1\">{note}</text></g>",
        hot = palette::heating(),
        steady = palette::steady(),
        cool = palette::cooling(),
        dorm = palette::dormant(),
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

/// Render the commit activity flame for validated graph JSON. Same transport as the
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
            APPEND out.svg "<text x=\"{{ $doc.px }}\" y=\"{{ $doc.tick_ly }}\" font-size=\"10\" fill=\"#9198a1\">30d ago</text>"
            APPEND out.svg "<text x=\"{{ $doc.ax1 }}\" y=\"{{ $doc.tick_ly }}\" font-size=\"10\" text-anchor=\"end\" fill=\"#9198a1\">today</text>"
            APPEND out.svg $doc.grads
            FOR $r: MAP<ANY> IN $doc.rows {
                APPEND out.svg "<g id=\"{{ $r.rid }}\" class=\"card {{ $r.pulse }}\" style=\"animation-duration:{{ $doc.tour_dur }}s;animation-delay:{{ $r.tour_delay }}\">"
                APPEND out.svg "<rect x=\"{{ $doc.dx }}\" y=\"{{ $doc.dy }}\" width=\"{{ $doc.dw }}\" height=\"{{ $doc.dh }}\" rx=\"4\" fill=\"url(#grad-{{ $r.rid }})\"><title>{{ $r.title }}</title></rect>"
                FOR $s: MAP<ANY> IN $r.stops {
                    APPEND out.svg "<line x1=\"{{ $s.x }}\" y1=\"{{ $doc.dy }}\" x2=\"{{ $s.x }}\" y2=\"{{ $doc.tick_by }}\" stroke=\"#9198a1\" stroke-width=\"1\" opacity=\"0.8\"/>"
                    APPEND out.svg "<text x=\"{{ $s.x }}\" y=\"{{ $doc.tick_ly }}\" font-size=\"10\" text-anchor=\"middle\" fill=\"#9198a1\">{{ $s.label }}</text>"
                }
                APPEND out.svg "<text x=\"{{ $doc.lx }}\" y=\"{{ $doc.l1y }}\" font-size=\"20\" font-weight=\"bold\" fill=\"#222\">{{ $r.nav }}</text>"
                APPEND out.svg "<text x=\"{{ $doc.lx }}\" y=\"{{ $doc.l2y }}\" class=\"dim\" font-weight=\"bold\">{{ $r.desc }}</text>"
                APPEND out.svg "<text x=\"{{ $doc.lx }}\" y=\"{{ $doc.l3y }}\" class=\"sub\">{{ $r.stats }}</text>"
                APPEND out.svg "<text x=\"{{ $doc.lx }}\" y=\"{{ $doc.l4y }}\" class=\"wht\">{{ $r.detail }}</text>"
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

/// Pinned-repo cards: bordered card, book icon + name header, wrapped
/// description, language dot + stars + forks footer, plus a heat band
/// and proof footer. Top-9 by stars, uniform height, emitted as one
/// flowing paragraph so the viewport wraps them as it wishes.
const PINS_TOPN: usize = 9;
const PIN_W: f64 = 280.0;
const PIN_NAME_MAX: usize = 22;
const PIN_DESC_WIDTH: usize = 34;
const PIN_DESC_MAX_LINES: usize = 4;

/// Card palette: GitHub dark-mode bubbles — black card, dark link blue,
/// secondary gray type. Language dots keep linguist colors.
const PIN_TITLE: &str = "#4493f8";
const PIN_ICON: &str = "#9198a1";
const PIN_TEXT: &str = "#9198a1";
const PIN_BG: &str = "#000000";
const PIN_BORDER: &str = "#30363d";

/// Linguist language colors for the footer dot; unknown maps to the
/// same `#333` fallback the reference card uses.
fn lang_color(lang: &str) -> &'static str {
    match lang {
        "Rust" => "#dea584",
        "TypeScript" => "#3178c6",
        "JavaScript" => "#f1e05a",
        "Python" => "#3572A5",
        "Shell" => "#89e051",
        "CSS" => "#563d7c",
        "HTML" => "#e34c26",
        "Dockerfile" => "#384d54",
        "Go" => "#00ADD8",
        "Java" => "#b07219",
        "Kotlin" => "#A97BFF",
        "Swift" => "#F05138",
        "C" => "#555555",
        "C++" => "#f34b7d",
        "C#" => "#178600",
        "Ruby" => "#701516",
        "PHP" => "#4F5D95",
        "Lua" => "#000080",
        "Vim script" => "#199f4b",
        _ => "#333",
    }
}

/// Compact counts like the reference card (`1200` → `1.2k`).
fn k_fmt(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

/// Word-wrap a description for the card: 34 chars per line, 4 lines
/// max, left-aligned. Overflow pushes nothing — the canvas is fixed —
/// so a cut tail gets an ellipsis instead of vanishing mid-sentence.
/// Empty input names itself.
fn wrap_desc(s: &str) -> Vec<String> {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return vec!["No description provided".to_string()];
    }
    let mut lines: Vec<String> = vec![];
    let mut cur = String::new();
    let mut truncated = false;
    for w in flat.split(' ') {
        if w.is_empty() {
            continue;
        }
        let fits = cur.is_empty() || cur.len() + 1 + w.len() <= PIN_DESC_WIDTH;
        if !fits {
            lines.push(std::mem::take(&mut cur));
            if lines.len() == PIN_DESC_MAX_LINES {
                truncated = true;
                break;
            }
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(w);
    }
    if truncated {
        // Words were cut: fall through to the ellipsis below.
    } else if lines.len() < PIN_DESC_MAX_LINES {
        if !cur.is_empty() {
            lines.push(cur);
        }
    } else if !cur.is_empty() {
        truncated = true;
    }
    if lines.is_empty() {
        lines.push("No description provided".to_string());
    } else if truncated {
        let last = lines.last_mut().expect("wrapped line");
        while last.chars().count() + 1 > PIN_DESC_WIDTH {
            last.pop();
        }
        last.push('\u{2026}');
    }
    lines
}

/// Estimated 12px Segoe UI advance, for footer item spacing. The
/// reference measures real glyphs; estimation keeps Rust side-only.
fn est_w(s: &str) -> f64 {
    s.chars().count() as f64 * 6.6
}

/// One pinned card: precomputed strings, so the template carries no
/// coordinates — same split as every other chart (Rust shapes, OxDock
/// emits). Each card renders to its own SVG file, so every bubble
/// links to its own repo.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct PinCard {
    /// Temp workspace filename (`pin-{i}.svg`); final path rides `dest`.
    file: String,
    /// Final committed path, e.g. `proof/charts/portfolio-graph-pin-foo.svg`.
    dest: String,
    url: String,
    name: String,
    fullname: String,
    d1: String,
    d2: String,
    d3: String,
    d4: String,
    has2: bool,
    has3: bool,
    has4: bool,
    /// Heat-band color: left strip + footer dot, so the card carries
    /// proof data (30d heat) GitHub's pinned cards don't show.
    band: String,
    /// Breathe class for the heat markers (`pulse`, `pulse slow`, or
    /// empty): heating breathes fast, steady slow, cooling/dormant static.
    pulse: String,
    /// Footer readout, e.g. `steady · 19 commits in 30d`: heat band
    /// spelled out next to the count, on its own footer line under the
    /// GitHub-like language/stars/forks row. Visible text, not a hover —
    /// README `<img>` embeds are non-interactive, so tooltips don't fire.
    heat: String,
    /// Plain-language summary for `<desc>`: what the card is plus the
    /// numbers, for direct-file readers (screen readers, agents, indexers).
    meta: String,
    has_lang: bool,
    lang: String,
    lang_color: String,
    stars: String,
    forks: String,
    fx_star: String,
    fx_fork: String,
    w: String,
    h: String,
    bw: String,
    bh: String,
    fy: String,
    fy2: String,
}

/// Pins document: static decor prebuilt in Rust, cards + fragment
/// interpolated. One oxdock run writes the whole family.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct PinDoc {
    header: String,
    footer: String,
    empty: bool,
    msg: String,
    muted: String,
    font: String,
    css: String,
    bg: String,
    border: String,
    icon: String,
    title: String,
    cards: Vec<PinCard>,
    frag_flow: String,
}

/// Pins rows: top-6 own-work repos by stars, ties break on forks
/// then name — the same popularity order GitHub's own section uses.
fn pin_rows(graph_json: &str) -> anyhow::Result<PinDoc> {
    let graph = jzombie_graph::graph::Graph::from_json(graph_json)?;
    let empty = PinDoc {
        header: String::new(),
        footer: String::new(),
        empty: true,
        msg: "No commits in trailing 30d".to_string(),
        muted: palette::muted(),
        font: palette::family(),
        css: pins_card_css()?,
        bg: PIN_BG.to_string(),
        border: PIN_BORDER.to_string(),
        icon: PIN_ICON.to_string(),
        title: "Popular repositories".to_string(),
        cards: vec![],
        frag_flow: String::new(),
    };
    let mut repos: Vec<&jzombie_graph::graph::Node> = graph
        .nodes
        .iter()
        .filter(|nd| !nd.private && jzombie_graph::graph::is_own_work(nd))
        .filter(|nd| nd.commits_30d > 0)
        .collect();
    if repos.is_empty() {
        return Ok(empty);
    }
    repos.sort_by(|a, b| {
        b.stars
            .cmp(&a.stars)
            .then_with(|| b.forks_count.cmp(&a.forks_count))
            .then_with(|| a.name.cmp(&b.name))
    });
    let shown = &repos[..repos.len().min(PINS_TOPN)];
    // Cache-bust token: the sweep timestamp, compact. Bytes change
    // nightly under stable filenames, and GitHub's image proxy caches
    // by URL — so every rendered asset URL carries the build it came
    // from. Same data, same token; new sweep, fresh fetch.
    let bust = graph.generated_at.format("%Y%m%d%H%M").to_string();
    // Card height follows the reference formula per card, then the
    // tallest wins for the whole family: uniform files, so the profile
    // grid reads as one set instead of six odd sizes.
    let mut line_sets: Vec<Vec<String>> = vec![];
    let mut h_max = 0.0_f64;
    for nd in shown.iter() {
        let desc = nd.description.as_deref().unwrap_or("");
        let lines = wrap_desc(desc);
        let h = if lines.len() > 1 { 100.0 } else { 90.0 } + lines.len() as f64 * 10.0 + 16.0;
        h_max = h_max.max(h);
        line_sets.push(lines);
    }
    let mut cards: Vec<PinCard> = vec![];
    for (i, nd) in shown.iter().enumerate() {
        let lines = &line_sets[i];
        let mut d = lines.clone();
        while d.len() < 4 {
            d.push(String::new());
        }
        let name = if nd.name.chars().count() > PIN_NAME_MAX {
            nd.name.chars().take(PIN_NAME_MAX - 1).collect::<String>() + "\u{2026}"
        } else {
            nd.name.clone()
        };
        let slug: String = nd
            .name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let stars = k_fmt(nd.stars);
        let forks = k_fmt(nd.forks_count);
        let lang_raw = nd.language.as_deref();
        // Left cluster: language, then stars, then forks. Left-anchored
        // rows degrade gracefully under font substitution; right-edge
        // anchoring clipped instead.
        let lang_w = match lang_raw {
            Some(l) => 10.0 + 4.0 + est_w(l),
            None => 0.0,
        };
        let star_w = 16.0 + 3.0 + est_w(&stars);
        let fx_star = if lang_raw.is_some() {
            lang_w + 12.0
        } else {
            0.0
        };
        let fx_fork = fx_star + star_w + 12.0;
        let heat = nd.commit_heat.or_else(|| nd.commit_heat());
        let (band, pulse) = match heat_band(heat) {
            "heating" => (palette::heating(), "pulse"),
            "steady" => (palette::steady(), "pulse slow"),
            "cooling" => (palette::cooling(), ""),
            _ => (palette::dormant(), ""),
        };
        let url = format!("https://github.com/{}", nd.id);
        let dest = format!(
            "{}/portfolio-graph-pin-{}.svg",
            jzombie_graph::paths::CHARTS_DIR,
            slug
        );
        cards.push(PinCard {
            file: format!("pin-{i}.svg"),
            dest: dest.clone(),
            url: url.clone(),
            name: xml_escape(&name),
            fullname: xml_escape(&nd.name),
            d1: xml_escape(&d[0]),
            d2: xml_escape(&d[1]),
            d3: xml_escape(&d[2]),
            d4: xml_escape(&d[3]),
            has2: lines.len() > 1,
            has3: lines.len() > 2,
            has4: lines.len() > 3,
            band,
            pulse: pulse.to_string(),
            heat: format!(
                "{} · {} {} in 30d",
                heat_band(heat),
                nd.commits_30d,
                if nd.commits_30d == 1 {
                    "commit"
                } else {
                    "commits"
                }
            ),
            meta: xml_escape(&format!(
                "{}: {}. {} {}, {} {}, {} {} in the last 30 days.",
                nd.name,
                lines.join(" "),
                nd.stars,
                if nd.stars == 1 { "star" } else { "stars" },
                nd.forks_count,
                if nd.forks_count == 1 { "fork" } else { "forks" },
                nd.commits_30d,
                if nd.commits_30d == 1 {
                    "commit"
                } else {
                    "commits"
                }
            )),
            has_lang: lang_raw.is_some(),
            lang: xml_escape(lang_raw.unwrap_or("")),
            lang_color: lang_raw.map(lang_color).unwrap_or("").to_string(),
            stars: xml_escape(&stars),
            forks: xml_escape(&forks),
            fx_star: f1(fx_star),
            fx_fork: f1(fx_fork),
            w: f1(PIN_W),
            h: f1(h_max),
            bw: f1(PIN_W - 1.0),
            bh: f1(h_max - 1.0),
            fy: f1(h_max - 26.0),
            fy2: f1(h_max - 10.0),
        });
    }
    // Fragment: one flowing paragraph, rank order left to right. No
    // fixed trios — the viewport wraps the bubbles as it wishes, and the
    // centered wrapper balances each visual line.
    let mut cells: Vec<String> = vec![];
    for c in cards.iter() {
        cells.push(format!(
            "[![{}](charts/{}?v={})]({})",
            c.name,
            c.dest.rsplit('/').next().unwrap_or(&c.dest),
            bust,
            c.url
        ));
    }
    let frag_flow = cells.join(" ");
    Ok(PinDoc {
        header: String::new(),
        footer: String::new(),
        empty: false,
        msg: String::new(),
        muted: palette::muted(),
        font: palette::family(),
        css: pins_card_css()?,
        bg: PIN_BG.to_string(),
        border: PIN_BORDER.to_string(),
        icon: PIN_ICON.to_string(),
        title: "Popular repositories".to_string(),
        cards,
        frag_flow,
    })
}

/// Shared motion rules, extracted from the single-sourced chart style —
/// no second copy of the breathe timing to drift. Fails loudly if the
/// shared stylesheet ever stops carrying these rules.
fn chart_pulse_css() -> anyhow::Result<String> {
    let css = chart_style()?;
    fn block<'a>(css: &'a str, pat: &str) -> anyhow::Result<&'a str> {
        let start = css
            .find(pat)
            .ok_or_else(|| anyhow::anyhow!("{pat} missing in shared style"))?;
        let mut depth = 0;
        for (i, c) in css[start..].char_indices() {
            if c == '{' {
                depth += 1;
            }
            if c == '}' {
                depth -= 1;
                if depth == 0 {
                    return Ok(css[start..start + i + 1].trim());
                }
            }
        }
        Err(anyhow::anyhow!("{pat} unterminated in shared style"))
    }
    Ok([
        block(&css, ".pulse {")?,
        block(&css, ".slow {")?,
        block(&css, "@keyframes breathe")?,
        block(&css, "@media (prefers-reduced-motion: reduce)")?,
    ]
    .join("\n"))
}

/// Pins chrome: per-card font classes on the shared Segoe stack, plus the
/// family's breathe rules lifted from the shared template above.
/// Heating breathes fast, steady slow, cooling/dormant stay static —
/// same thresholds as the flame and indicator bands. One copy rides every
/// card file so each SVG stands alone.
fn pins_card_css() -> anyhow::Result<String> {
    Ok(format!(
        ".pname {{ font: 600 18px 'Segoe UI', Ubuntu, Sans-Serif; fill: {title}; }} .pdesc {{ font: 400 12px 'Segoe UI', Ubuntu, Sans-Serif; fill: {text}; }} .pfoot {{ font: 400 12px 'Segoe UI', Ubuntu, Sans-Serif; fill: {text}; }} {pulse}",
        title = PIN_TITLE,
        text = PIN_TEXT,
        pulse = chart_pulse_css()?,
    ))
}

/// Plugin function: validated graph JSON in, pins document out.
#[oxdock_func(pure, name = "PINS")]
fn pins(graph_json: String) -> anyhow::Result<Value> {
    let graph_json = transport_out(&graph_json)?;
    Ok(Value::string(serde_json::to_string(&pin_rows(
        &graph_json,
    )?)?))
}

/// Render the popular-repositories pins family: one SVG per top repo
/// plus the fragment table that embeds them, each bubble linking to
/// its own repo. Returns `(committed relpath, bytes)` pairs; the
/// caller writes the bytes (so output paths stay in one place).
/// Same transport as the pie: embedded script emits markup into a
/// temp workspace.
pub fn render_pins_bundle(graph_json: &str) -> anyhow::Result<Vec<(String, String)>> {
    let engine = engine();
    let arg = transport_in(graph_json);
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS],
        IMPORT [CHARTS]
        IMPORT [STD]
        LET $doc_json: STRING = CHARTS::PINS(#arg)
        WRITE rows.json $doc_json
        LET $doc: MAP<ANY> = LOAD_JSON("rows.json")
        MKDIR svg
        MKDIR frag
        IF $doc.empty {
            WRITE frag/pins.md "## Popular repositories\n\n*{{ $doc.msg }}*\n"
        } ELSE {
            FOR $c: MAP<ANY> IN $doc.cards {
                WRITE svg/{{ $c.file }} "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{{ $c.w }}\" height=\"{{ $c.h }}\" viewBox=\"0 0 {{ $c.w }} {{ $c.h }}\" role=\"img\" font-family=\"{{ $doc.font }}\"><title>{{ $c.fullname }}</title><desc>{{ $c.meta }}</desc><style>{{ $doc.css }}</style><a href=\"{{ $c.url }}\"><rect x=\"0.5\" y=\"0.5\" width=\"{{ $c.bw }}\" height=\"{{ $c.bh }}\" rx=\"4.5\" fill=\"{{ $doc.bg }}\" stroke=\"{{ $doc.border }}\"/><rect x=\"0.5\" y=\"0.5\" width=\"5\" height=\"{{ $c.bh }}\" rx=\"2\" fill=\"{{ $c.band }}\" class=\"{{ $c.pulse }}\"/><g transform=\"translate(12,30)\">"
                APPEND svg/{{ $c.file }} "<svg x=\"0\" y=\"-13\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\" version=\"1.1\" fill=\"{{ $doc.icon }}\"><path fill-rule=\"evenodd\" d=\"M2 2.5A2.5 2.5 0 014.5 0h8.75a.75.75 0 01.75.75v12.5a.75.75 0 01-.75.75h-2.5a.75.75 0 110-1.5h1.75v-2h-8a1 1 0 00-.714 1.7.75.75 0 01-1.072 1.05A2.495 2.495 0 012 11.5v-9zm10.5-1V9h-8c-.356 0-.694.074-1 .208V2.5a1 1 0 011-1h8zM5 12.25v3.25a.25.25 0 00.4.2l1.45-1.087a.25.25 0 01.3 0L8.6 15.7a.25.25 0 00.4-.2v-3.25a.25.25 0 00-.25-.25h-3.5a.25.25 0 00-.25.25z\"/></svg><text x=\"28\" y=\"0\" class=\"pname\">{{ $c.name }}</text></g>"
                APPEND svg/{{ $c.file }} "<text x=\"12\" y=\"54\" class=\"pdesc\">{{ $c.d1 }}</text>"
                IF $c.has2 {
                    APPEND svg/{{ $c.file }} "<text x=\"12\" y=\"70\" class=\"pdesc\">{{ $c.d2 }}</text>"
                }
                IF $c.has3 {
                    APPEND svg/{{ $c.file }} "<text x=\"12\" y=\"86\" class=\"pdesc\">{{ $c.d3 }}</text>"
                }
                IF $c.has4 {
                    APPEND svg/{{ $c.file }} "<text x=\"12\" y=\"102\" class=\"pdesc\">{{ $c.d4 }}</text>"
                }
                APPEND svg/{{ $c.file }} "<g transform=\"translate(15,{{ $c.fy }})\" class=\"pfoot\">"
                IF $c.has_lang {
                    APPEND svg/{{ $c.file }} "<circle cx=\"0\" cy=\"-4\" r=\"5\" fill=\"{{ $c.lang_color }}\"/><text x=\"9\" y=\"0\">{{ $c.lang }}</text>"
                }
                APPEND svg/{{ $c.file }} "<g transform=\"translate({{ $c.fx_star }},0)\"><svg x=\"0\" y=\"-13\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\" version=\"1.1\" fill=\"{{ $doc.icon }}\"><path fill-rule=\"evenodd\" d=\"M8 .25a.75.75 0 01.673.418l1.882 3.815 4.21.612a.75.75 0 01.416 1.279l-3.046 2.97.719 4.192a.75.75 0 01-1.088.791L8 12.347l-3.766 1.98a.75.75 0 01-1.088-.79l.72-4.194L.818 6.374a.75.75 0 01.416-1.28l4.21-.611L7.327.668A.75.75 0 018 .25zm0 2.445L6.615 5.5a.75.75 0 01-.564.41l-3.097.45 2.24 2.184a.75.75 0 01.216.664l-.528 3.084 2.769-1.456a.75.75 0 01.698 0l2.77 1.456-.53-3.084a.75.75 0 01.216-.664l2.24-2.183-3.096-.45a.75.75 0 01-.564-.41L8 2.694v.001z\"/></svg><text x=\"19\" y=\"0\">{{ $c.stars }}</text></g><g transform=\"translate({{ $c.fx_fork }},0)\"><svg x=\"0\" y=\"-13\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\" version=\"1.1\" fill=\"{{ $doc.icon }}\"><path fill-rule=\"evenodd\" d=\"M5 3.25a.75.75 0 11-1.5 0 .75.75 0 011.5 0zm0 2.122a2.25 2.25 0 10-1.5 0v.878A2.25 2.25 0 005.75 8.5h1.5v2.128a2.251 2.251 0 101.5 0V8.5h1.5a2.25 2.25 0 002.25-2.25v-.878a2.25 2.25 0 10-1.5 0v.878a.75.75 0 01-.75.75h-4.5A.75.75 0 015 6.25v-.878zm3.75 7.378a.75.75 0 11-1.5 0 .75.75 0 011.5 0zm3-8.75a.75.75 0 100-1.5.75.75 0 000 1.5z\"></path></svg><text x=\"19\" y=\"0\">{{ $c.forks }}</text></g></g>"
                APPEND svg/{{ $c.file }} "<g transform=\"translate(15,{{ $c.fy2 }})\" class=\"pfoot\"><circle cx=\"0\" cy=\"-4\" r=\"5\" fill=\"{{ $c.band }}\" class=\"{{ $c.pulse }}\"/><text x=\"9\" y=\"0\">{{ $c.heat }}</text></g>"
                APPEND svg/{{ $c.file }} "</a></svg>"
            }
            WRITE frag/pins.md "## Popular repositories\n\n<div align=\"center\">\n\n"
            APPEND frag/pins.md "{{ $doc.frag_flow }}\n\n"
            APPEND frag/pins.md "</div>\n"
        }
        LET $check: STRING = READ frag/pins.md
        ASSERT_CONTAINS $check "Popular repositories"
    };
    let temp = oxdock::GuardedPath::tempdir().expect("tempdir");
    let root = temp.as_guarded_path().clone();
    engine.run_steps_without_snapshot(&root, &steps)?;
    let resolver = oxdock::PathResolver::new(root.as_path(), root.as_path()).expect("resolver");
    // Card inventory rides rows.json: deterministic temp names in,
    // committed relpaths out.
    let doc_path = root.join("rows.json").expect("doc path");
    let doc: PinDoc = serde_json::from_str(&resolver.read_to_string(&doc_path).expect("read doc"))?;
    let mut bundle: Vec<(String, String)> = vec![];
    for card in &doc.cards {
        let rel = format!("svg/{}", card.file);
        let temp_path = root.join(&rel).expect("card path");
        let svg = resolver.read_to_string(&temp_path).expect("read card");
        assert!(svg.starts_with("<svg "));
        assert!(svg.ends_with("</svg>"));
        bundle.push((card.dest.clone(), svg));
    }
    let frag_path = root.join("frag/pins.md").expect("frag path");
    let frag = resolver.read_to_string(&frag_path).expect("read frag");
    bundle.push((jzombie_graph::paths::PINS_FRAG_MD.to_string(), frag));
    Ok(bundle)
}

fn engine() -> Engine {
    let mut engine = Engine::new();
    engine.register_module(HostModule {
        name: "CHARTS".to_string(),
        funcs: vec![
            PieSlices::registration(),
            FocusBars::registration(),
            Indicators::registration(),
            DataTable::registration(),
            ChartStyleFn::registration(),
            Flame::registration(),
            Pins::registration(),
        ],
        types: vec![],
        record_schemas: vec![],
        states: vec![],
    });
    engine.register_module(oxdock_markdown_plugin::module());
    engine
}

/// Render the commit-share pie for validated graph JSON. The embedded
/// script emits markup into a temp workspace; Rust reads it back and the
/// caller writes the bytes (so output paths stay in one place).
pub fn bust_token(graph_json: &str) -> anyhow::Result<String> {
    let graph = jzombie_graph::graph::Graph::from_json(graph_json)?;
    Ok(graph.generated_at.format("%Y%m%d%H%M").to_string())
}

/// Section fragments with cache-busted image URLs. Same bust token as pins:
/// the sweep timestamp, so GitHub's image proxy refetches after every
/// nightly rebuild instead of serving the stale bytes.
pub fn render_flame_md(graph_json: &str) -> anyhow::Result<String> {
    let bust = bust_token(graph_json)?;
    Ok(format!(
        "[![flame](charts/portfolio-graph-flame.svg?v={bust})]({{{{ $global.repos }}}})\n"
    ))
}

pub fn render_share_md(graph_json: &str) -> anyhow::Result<String> {
    let bust = bust_token(graph_json)?;
    Ok(format!(
        "## Commit share\n\n[![share](charts/portfolio-graph-share.svg?v={bust})]({{{{ $global.repos }}}})\n"
    ))
}

pub fn render_focus_md(graph_json: &str) -> anyhow::Result<String> {
    let bust = bust_token(graph_json)?;
    Ok(format!(
        "## Developer focus\n\nDeveloper focus is a heuristic of what's being planned and developed right now.\n\n[![developer focus](charts/portfolio-graph-developer-focus.svg?v={bust})]({{{{ $global.repos }}}})\n\n<sub>Each score blends commit heat (recent commit rates vs the trailing 30d baseline) with issue comment heat (owner authored comment volume, same comparison), then discounts idle months. A signal with no baseline sits out: triage free repos rank on commits alone, commit free weeks on comments alone.</sub>\n"
    ))
}

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

/// Heat color, stepped on the documented `commit_heat` bands: heating
/// vermillion, steady amber, cooling sky blue, dormant faint gray.
/// Temperature order, Okabe-Ito stops — no legend needed.
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

/// One data-table row: about (description + topic tags), proof
/// (commits with the method shrunk beneath, heat badge beside it),
/// reuse (one portfolio-internal column, both directions labeled —
/// never GitHub's external "used by"; the JSON edges stay the machine
/// surface). Stars fold into the repo cell (`★ N`) instead of spending
/// a column on 0s. Keys sort alphabetically in the rendered table
/// (`about, commits/30d, heat, layer, repo, reuse (portfolio)`) —
/// Rust shapes the values, the MARKDOWN module owns the markup.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct DataRow {
    // Leading-space sort key: `MAP_TO_MD_TABLE` orders columns
    // alphabetically and Rust cannot hand it an order, so the identity
    // column takes a sort key that renders as `repo` (cells are padded
    // anyway). Deliberate, tested raw — not a silent trick.
    // TODO: teach OxDock's MAP_TO_MD_TABLE an explicit column order (parameter
    // or insertion-order framing) so the module owns sorting and this
    // key goes back to `repo`.
    #[serde(rename = " repo")]
    repo: String,
    layer: String,
    about: String,
    #[serde(rename = "commits/30d")]
    commits: String,
    heat: String,
    #[serde(rename = "reuse (portfolio)")]
    reuse: String,
}

/// Data-table document: summary line + rows + overflow note.
/// Same contract as the badge table (Rust shapes data, OxDock emits).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct DataDoc {
    summary: String,
    empty: bool,
    msg: String,
    rows: Vec<DataRow>,
    /// Tail note when repos fall below the top-10 cut: rendered as an
    /// HTML comment under the table, never a junk row. Empty when all
    /// repos fit.
    overflow: String,
}

/// Table scope: top-10 by 30d commits, the rest named in an HTML
/// comment below the table. Matches the strategy's 5–10 curation
/// cutline while staying inside token budget.
const DATA_TOPN: usize = 10;

/// Badge embed for a heat band, label shrunk beneath it, centered.
/// The badge towers over the text on a shared baseline, so side by
/// side they always read offset — stacked and centered instead.
/// `div align` is the only centering github.com honors in a cell.
fn heat_cell(band: &str) -> String {
    format!("<div align=\"center\">![{band}](indicators/{band}.svg)<br><sub>{band}</sub></div>")
}

/// Topic links for an about cell: tiny `#tag` links to a user-scoped repo
/// search, capped so one repo can't flood the table. Owner rides the
/// node — never a baked-in login.
fn topic_links(owner: &str, topics: &[String]) -> String {
    const MAX: usize = 4;
    let mut tags: Vec<String> = topics
        .iter()
        .take(MAX)
        .map(|t| {
            let enc: String = t
                .chars()
                .flat_map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' {
                        vec![c]
                    } else {
                        format!("%{0:02X}", c as u32).chars().collect::<Vec<_>>()
                    }
                })
                .collect();
            format!(
                "[#{t}](https://github.com/search?q=user%3A{owner}+topic%3A{enc}&type=repositories)"
            )
        })
        .collect();
    if topics.len() > MAX {
        tags.push(format!("+{}", topics.len() - MAX));
    }
    tags.join(" ")
}

/// One line of prose for a markdown table cell. The MARKDOWN module owns
/// escaping (`|` → `\|`, newline → `<br>`); Rust only keeps descriptions short.
fn cell_text(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        flat.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    } else {
        flat
    }
}

/// Rows for the commit-data table: own-work repos only (same filter as
/// the charts — pristine forks never read). Sorted by 30d share desc;
/// layers ride a column so the ranking stays contiguous.
fn data_rows(graph_json: &str) -> anyhow::Result<DataDoc> {
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
    let summary = format!(
        "Updated {} UTC — {} repos, {} commits in the last 30 days.",
        graph.generated_at.format("%Y-%m-%d %H:%M"),
        repos.len(),
        total,
    );
    if repos.is_empty() {
        return Ok(DataDoc {
            summary,
            empty: true,
            msg: "No commits in trailing 30d".to_string(),
            rows: vec![],
            overflow: String::new(),
        });
    }
    // Short names for link labels; edge ids outside the node list fall
    // back to the repo part of the id. Sorted for deterministic output.
    let short = |id: &str| {
        graph
            .nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| id.rsplit('/').next().unwrap_or(id).to_string())
    };
    let link = |id: &str| format!("[{}](https://github.com/{})", short(id), id);
    let mut used_by: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut uses: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for n in &graph.nodes {
        used_by.entry(n.id.clone()).or_default();
        uses.entry(n.id.clone()).or_default();
    }
    for e in &graph.edges {
        if e.kind.is_dependence() {
            used_by
                .entry(e.target.clone())
                .or_default()
                .push(e.source.clone());
            uses.entry(e.source.clone())
                .or_default()
                .push(e.target.clone());
        }
    }
    // Recruiter-readable connectivity: named links per direction, not
    // opaque `in/out` counts. Headers carry the direction, so cells are
    // pure link lists; an empty side reads `—`.
    let links = |ids: &[String]| {
        let mut v = ids.to_vec();
        v.sort();
        v.dedup();
        if v.is_empty() {
            return "—".to_string();
        }
        v.iter().map(|s| link(s)).collect::<Vec<_>>().join(", ")
    };
    let rows: Vec<DataRow> = repos
        .iter()
        .take(DATA_TOPN)
        .map(|n| {
            let heat = n.commit_heat.or_else(|| n.commit_heat());
            let desc = n
                .description
                .as_deref()
                .map(|d| cell_text(d, 80))
                .unwrap_or_default();
            let tags = topic_links(&n.owner, &n.topics);
            // Role carries the tiny topic search links; bare repo keeps "-".
            let about = match (desc.as_str(), tags.as_str()) {
                ("", "") => "-".to_string(),
                ("", _) => format!("<sub>{tags}</sub>"),
                (_, "") => desc,
                _ => format!("{desc} <sub>{tags}</sub>"),
            };
            DataRow {
                // Proof folds in here, shrunk: a full column of 0s and
                // 1s is noise, but `★ 37` still pops where it matters.
                repo: format!(
                    "[{}](https://github.com/{})<br><sub>★ {}</sub>",
                    n.name, n.id, n.stars
                ),
                // The cell prints the stored tag set: middleware with a
                // default binary reads `middleware · application` (both);
                // a tagless leaf library reads `—` (no claim, not a lie).
                layer: jzombie_graph::graph::Graph::layer_string(&n.layer),
                about,
                // The count carries its method on its own line, shrunk:
                // deduped oid union across N in-scope branches under `all`
                // (branch_heat spans every branch, so its length is the
                // union width), default-branch commits under `default`.
                // Branchless rows (fixtures, lean JSON) name no count.
                // Literal `<br>`: the formatter flattens raw newlines to
                // spaces (see the hostile-text test), tags pass through.
                commits: {
                    let width = n.branch_heat.len();
                    let method = match graph.velocity_scope {
                        jzombie_graph::graph::VelocityScope::All if width == 1 => {
                            "single branch".to_string()
                        }
                        jzombie_graph::graph::VelocityScope::All if width > 1 => {
                            format!("deduped across {width} branches")
                        }
                        jzombie_graph::graph::VelocityScope::All => {
                            "deduped across branches".to_string()
                        }
                        jzombie_graph::graph::VelocityScope::Default => {
                            "default branch".to_string()
                        }
                    };
                    format!("{}<br><sub>{method}</sub>", n.commits_30d)
                },
                heat: heat_cell(heat_band(heat)),
                // One reuse column: stacked labeled lines split by `<hr>`
                // (`---` would split the row). Empty sides drop out;
                // JSON edges stay the machine surface.
                reuse: {
                    let mut ub = used_by.get(&n.id).cloned().unwrap_or_default();
                    let mut us = uses.get(&n.id).cloned().unwrap_or_default();
                    ub.sort();
                    ub.dedup();
                    us.sort();
                    us.dedup();
                    match (ub.is_empty(), us.is_empty()) {
                        (true, true) => "—".to_string(),
                        (true, false) => format!("<sub>uses</sub><br>{}", links(&us)),
                        (false, true) => {
                            format!("<sub>used by</sub><br>{}", links(&ub))
                        }
                        (false, false) => {
                            format!(
                                "<sub>used by</sub><br>{}<br><hr><sub>uses</sub><br>{}",
                                links(&ub),
                                links(&us)
                            )
                        }
                    }
                },
            }
        })
        .collect();
    // Below the cut: no junk row — names plus combined commits ride an
    // HTML comment under the table instead.
    let rest = &repos[repos.len().min(DATA_TOPN)..];
    let overflow = if rest.is_empty() {
        String::new()
    } else {
        let names: Vec<String> = rest.iter().map(|n| n.name.clone()).collect();
        let total: u32 = rest.iter().map(|n| n.commits_30d).sum();
        format!(
            "\n<!-- +{} more below top-10 ({} commits/30d combined): {} -->\n",
            rest.len(),
            total,
            names.join(", ")
        )
    };
    Ok(DataDoc {
        summary,
        empty: false,
        msg: String::new(),
        rows,
        overflow,
    })
}

/// Plugin function: validated graph JSON in, data-table document out.
#[oxdock_func(pure, name = "DATA_TABLE")]
fn data_table(graph_json: String) -> anyhow::Result<Value> {
    let graph_json = transport_out(&graph_json)?;
    Ok(Value::string(serde_json::to_string(&data_rows(
        &graph_json,
    )?)?))
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

/// Render the indicator table: badge embeds + band + heat, one row per band.
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

            *Regenerate with `oxdock scripts/proof.oxfile`. Badges sit beside this file; the rows come from the same template doc as the SVGs.*

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

/// Render the machine-readable commit-data table: one row per active
/// repo (top-10, tail named in a comment), layer-labeled, same rows the
/// charts draw. Markup comes from the shared MARKDOWN module — Rust
/// shapes the rows, `MAP_TO_MD_TABLE` frames them. Data with quotes
/// rides base64 — raw `#var` inlining would break.
pub fn render_data_table_md(graph_json: &str) -> anyhow::Result<String> {
    let engine = engine();
    let arg = transport_in(graph_json);
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS, MARKDOWN],
        IMPORT [STD, CHARTS, MARKDOWN]
        LET $rows_json: STRING = CHARTS::DATA_TABLE(#arg)
        WRITE rows.json $rows_json
        LET $doc: MAP<ANY> = LOAD_JSON("rows.json")
        WRITE md.md "## Commit data\n\n"
        APPEND md.md $doc.summary
        APPEND md.md "\n\n"
        IF $doc.empty {
            APPEND md.md "*{{ $doc.msg }}*\n"
        } ELSE {
            APPEND md.md "{{ MARKDOWN::MAP_TO_MD_TABLE($doc.rows) }}"
            APPEND md.md $doc.overflow
        }
        LET $check: STRING = READ md.md
        ASSERT_CONTAINS $check "Updated"
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
                issues_open_1d: 0,
                issues_open_5d: 0,
                issues_open_15d: 0,
                issues_open_30d: 0,
                issues_closed_1d: 0,
                issues_closed_5d: 0,
                issues_closed_15d: 0,
                issues_closed_30d: 0,
                issue_comments_1d: 0,
                issue_comments_5d: 0,
                issue_comments_15d: 0,
                issue_comments_30d: 0,
                issue_labels: Default::default(),
                recent_issues: vec![],
                issue_heat: None,
                commit_heat: None,
                focus: None,
                last_commit_at: None,
                branch_heat: vec![],
                recent_prs: vec![],
                visible: true,
                visible_reason: None,
                layer: std::collections::BTreeSet::from([Layer::Isolated]),
                has_default_binary: false,
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
                issues_open_1d: 0,
                issues_open_5d: 0,
                issues_open_15d: 0,
                issues_open_30d: 0,
                issues_closed_1d: 0,
                issues_closed_5d: 0,
                issues_closed_15d: 0,
                issues_closed_30d: 0,
                issue_comments_1d: 0,
                issue_comments_5d: 0,
                issue_comments_15d: 0,
                issue_comments_30d: 0,
                issue_labels: Default::default(),
                recent_issues: vec![],
                issue_heat: None,
                commit_heat: None,
                focus: *f,
                last_commit_at: None,
                branch_heat: vec![],
                visible: true,
                recent_prs: vec![],
                visible_reason: None,
                layer: std::collections::BTreeSet::from([Layer::Isolated]),
                has_default_binary: false,
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

    #[test]
    fn wrap_desc_truncates_with_ellipsis() {
        // Overflow never pushes content off the fixed canvas: the cut
        // tail gets an ellipsis, and the line count never exceeds the cap.
        let long = "word ".repeat(200);
        let lines = wrap_desc(&long);
        assert_eq!(lines.len(), PIN_DESC_MAX_LINES);
        assert!(lines.last().unwrap().ends_with('\u{2026}'));
        assert!(lines.iter().all(|l| l.chars().count() <= PIN_DESC_WIDTH));
        // Short text passes through untouched, no ellipsis.
        let short = wrap_desc("hello world");
        assert_eq!(short, vec!["hello world"]);
        assert!(wrap_desc("").len() == 1);
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
        // Stepped ramp on the documented heat bands: heating vermillion,
        // steady amber, cooling blue (still pulses), dormant faint.
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
        // Whitespace-insensitive: reformatting the block must not break this.
        let css = chart_style().unwrap();
        assert!(css.starts_with("<style>"));
        assert!(css.ends_with("</style>"));
        let flat: String = css.split_whitespace().collect();
        for token in [
            ".bar{stroke:#fff;",
            "rx:4",
            ".pulse{animation:breathe",
            ".slow{animation-duration:3.4s",
            "@keyframesbreathe",
            ".wht{fill:#fff;",
            "@keyframeswhisk",
            "prefers-reduced-motion",
        ] {
            assert!(flat.contains(token), "missing {token}");
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
        let flat_svg: String = svg.split_whitespace().collect();
        assert!(flat_svg.contains("prefers-reduced-motion"));
        assert!(flat_svg.contains("animation:none"));
        // Every repo gets a card with its readout, however small.
        assert!(svg.contains(">small<"));
        assert!(svg.contains("60 commits"));
        // Recency gradients: oldest window dormant, newest heating.
        assert!(svg.contains("grad-r0"));
        assert!(svg.contains("stop-color=\"#D55E00\""));
    }

    #[test]
    fn flame_rows_carry_recency_and_share() {
        // Two repos, 3:1 split (c30 derives 6x c5: 180 + 60).
        let g = graph_with(&[("big", 30), ("small", 10)]);
        // Recency gradients carry window shares; offsets must sum to 1.
        let doc = flame_rows(&g).unwrap();
        assert!(!doc.empty);
        assert_eq!(doc.rows.len(), 2);
        assert!(doc.grads.contains("grad-r0"));
        assert!(doc.grads.contains("grad-r1"));
        // Big's oldest window holds half its commits: stop at 0.500.
        assert!(doc.grads.contains("offset=\"0.500\""));
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
    fn data_table_top10_names_the_tail_in_a_comment() {
        // 12 active repos: 10 rows, tail named in an HTML comment —
        // no junk row. Hottest first, badge in the heat cell.
        let names: Vec<String> = (0..12).map(|i| format!("r{i:02}")).collect();
        let pairs: Vec<(&str, u32, u32)> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), (i as u32 + 1) * 2, (i as u32 + 1) * 12))
            .collect();
        let g = graph_with_heat(&pairs);
        let md = render_data_table_md(&g).unwrap();
        // Formatter owns the frame: alphabetical columns, padded cells.
        let header = md.lines().find(|l| l.starts_with('|')).unwrap();
        // Repo leads: the sort key carries a leading space, so the raw
        // first cell opens with two (delimiter padding + key space).
        // Trimmed assertions below would pass either way — this one
        // proves the trick is present, not silent.
        let raw_first = header.split('|').nth(1).unwrap();
        assert!(
            raw_first.starts_with("  "),
            "repo sort key present:\n{header}"
        );
        let cols: Vec<String> = header
            .split('|')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty() && !c.chars().all(|c| c == '-'))
            .collect();
        assert_eq!(
            cols,
            vec![
                "repo",
                "about",
                "commits/30d",
                "heat",
                "layer",
                "reuse (portfolio)"
            ],
            "union columns, alphabetical:\n{md}"
        );
        // Top row is the hottest (r11 first), tail named below the table.
        let top = md.find("r11").unwrap();
        let second = md.find("r10").unwrap();
        assert!(top < second, "30d desc");
        assert!(!md.contains("_Other"), "no roll-up row");
        assert!(
            md.contains("<!-- +2 more below top-10"),
            "tail in a comment:\n{md}"
        );
        // Count carries its method on its own line, shrunk
        // (fixture scope is All, fixture nodes are branchless).
        assert!(
            md.contains("<br><sub>deduped across branches</sub>"),
            "computation method in cell:\n{md}"
        );
        assert!(
            md.contains("r00") && md.contains("r01"),
            "tail named:\n{md}"
        );
        // Heat cell carries the band badge over its label, centered.
        assert!(
            md.contains(
                "<div align=\"center\">![steady](indicators/steady.svg)<br><sub>steady</sub></div>"
            ),
            "badge + small label in heat cell:\n{md}"
        );
        // Summary is data: run time and counts; raw-file links live
        // nowhere in the rendered page.
        assert!(md.contains("12 repos"));
        assert!(md.contains("Updated"), "last run on top");
        assert!(md.contains("UTC"), "timestamps read UTC");
        assert!(
            !md.contains("data/portfolio-graph.json"),
            "no raw-file links in prose"
        );
        assert!(!md.contains("blob/main"), "no absolute repo links in prose");
    }

    #[test]
    fn data_table_commits_cell_names_branch_count() {
        // The method sub carries the union width: N in-scope branches
        // under scope All. Singular reads right too.
        use jzombie_graph::graph::{BranchHeat, Graph, Layer, Node};
        let heat = |branch: &str, c30: u32| BranchHeat {
            branch: branch.into(),
            is_default: branch == "main",
            commits_1d: 0,
            commits_5d: 0,
            commits_15d: 0,
            commits_30d: c30,
        };
        let node = |name: &str, c30: u32, branches: Vec<BranchHeat>| Node {
            id: format!("me/{name}"),
            owner: "me".into(),
            name: name.into(),
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
            commits_30d: c30,
            issues_open_1d: 0,
            issues_open_5d: 0,
            issues_open_15d: 0,
            issues_open_30d: 0,
            issues_closed_1d: 0,
            issues_closed_5d: 0,
            issues_closed_15d: 0,
            issues_closed_30d: 0,
            issue_comments_1d: 0,
            issue_comments_5d: 0,
            issue_comments_15d: 0,
            issue_comments_30d: 0,
            issue_labels: Default::default(),
            recent_issues: vec![],
            issue_heat: None,
            commit_heat: None,
            focus: None,
            last_commit_at: None,
            branch_heat: branches,
            recent_prs: vec![],
            visible: true,
            visible_reason: None,
            layer: std::collections::BTreeSet::from([Layer::Isolated]),
            has_default_binary: false,
        };
        let g = jzombie_graph::output::render_json(&Graph::new(
            vec![
                node(
                    "wide",
                    60,
                    vec![heat("main", 60), heat("a", 60), heat("b", 60)],
                ),
                node("solo", 5, vec![heat("main", 5)]),
            ],
            vec![],
        ))
        .unwrap();
        let md = render_data_table_md(&g).unwrap();
        assert!(
            md.contains("60<br><sub>deduped across 3 branches</sub>"),
            "union width in cell:\n{md}"
        );
        assert!(
            md.contains("5<br><sub>single branch</sub>"),
            "singular reads right:\n{md}"
        );
    }

    #[test]
    fn data_table_cells_survive_hostile_text() {
        // Pipes, quotes, and newlines in a description must not break the
        // table or the OxDock transport (quotes ride base64 end-to-end).
        use jzombie_graph::graph::{Graph, Layer, Node};
        let nodes = vec![Node {
            id: "me/quot".into(),
            owner: "me".into(),
            name: "quot".into(),
            fork: false,
            private: false,
            upstream: None,
            ahead_by: None,
            pushed_at: None,
            description: Some("does \"things\" | pipes\nand lines".into()),
            topics: vec![],
            language: Some("Rust".into()),
            homepage: None,
            stars: 7,
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
            commits_1d: 2,
            commits_5d: 10,
            commits_15d: 30,
            commits_30d: 60,
            issues_open_1d: 0,
            issues_open_5d: 0,
            issues_open_15d: 0,
            issues_open_30d: 0,
            issues_closed_1d: 0,
            issues_closed_5d: 0,
            issues_closed_15d: 0,
            issues_closed_30d: 0,
            issue_comments_1d: 0,
            issue_comments_5d: 0,
            issue_comments_15d: 0,
            issue_comments_30d: 0,
            issue_labels: Default::default(),
            recent_issues: vec![],
            issue_heat: None,
            commit_heat: None,
            focus: None,
            last_commit_at: None,
            branch_heat: vec![],
            recent_prs: vec![],
            visible: true,
            visible_reason: None,
            layer: std::collections::BTreeSet::from([Layer::Isolated]),
            has_default_binary: false,
        }];
        let g = jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap();
        let md = render_data_table_md(&g).unwrap();
        assert!(
            md.contains("[quot](https://github.com/me/quot)"),
            "repo linked"
        );
        assert!(md.contains("isolated"), "layer column");
        // Pipes ride the formatter's escaping, not a Rust rewrite.
        assert!(
            md.contains("does \"things\" \\| pipes and lines"),
            "cell escaped"
        );
        assert!(md.contains("<sub>★ 7</sub>"), "stars folded into repo cell");
    }

    #[test]
    fn data_table_layer_cell_carries_default_binary_tag() {
        // term-wm case: the stored set holds both tags (middleware
        // positionally, application by default binary — see
        // `layers_default_binary_adds_application_tag` for the rule).
        // The cell prints the set: `middleware · application` (both).
        use jzombie_graph::graph::{Graph, Layer, Node};
        let node = |tags: &[Layer]| Node {
            id: "me/app".into(),
            owner: "me".into(),
            name: "app".into(),
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
            commits_1d: 1,
            commits_5d: 5,
            commits_15d: 15,
            commits_30d: 30,
            issues_open_1d: 0,
            issues_open_5d: 0,
            issues_open_15d: 0,
            issues_open_30d: 0,
            issues_closed_1d: 0,
            issues_closed_5d: 0,
            issues_closed_15d: 0,
            issues_closed_30d: 0,
            issue_comments_1d: 0,
            issue_comments_5d: 0,
            issue_comments_15d: 0,
            issue_comments_30d: 0,
            issue_labels: Default::default(),
            recent_issues: vec![],
            issue_heat: None,
            commit_heat: None,
            focus: None,
            last_commit_at: None,
            branch_heat: vec![],
            recent_prs: vec![],
            visible: true,
            visible_reason: None,
            layer: tags.iter().copied().collect(),
            has_default_binary: false,
        };
        let both = [Layer::Middleware, Layer::Application];
        let bin_g =
            jzombie_graph::output::render_json(&Graph::new(vec![node(&both)], vec![])).unwrap();
        assert!(
            render_data_table_md(&bin_g)
                .unwrap()
                .contains("middleware · application"),
            "both tags in the cell"
        );
        let mid = [Layer::Middleware];
        let lib_g =
            jzombie_graph::output::render_json(&Graph::new(vec![node(&mid)], vec![])).unwrap();
        let lib_md = render_data_table_md(&lib_g).unwrap();
        assert!(lib_md.contains("middleware"), "layer still there");
        assert!(
            !lib_md.contains("· application"),
            "no tag without a default binary"
        );
    }

    #[test]
    fn data_table_used_by_and_uses_carry_repo_links() {
        // lib <- mid <- app: separate columns, named links, no counts.
        // Associations (docs) never land in either column.
        use jzombie_graph::graph::{Edge, EdgeKind, Graph, Layer, Node};
        let node = |name: &str| Node {
            id: format!("me/{name}"),
            owner: "me".into(),
            name: name.into(),
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
            commits_1d: 1,
            commits_5d: 5,
            commits_15d: 15,
            commits_30d: 30,
            issues_open_1d: 0,
            issues_open_5d: 0,
            issues_open_15d: 0,
            issues_open_30d: 0,
            issues_closed_1d: 0,
            issues_closed_5d: 0,
            issues_closed_15d: 0,
            issues_closed_30d: 0,
            issue_comments_1d: 0,
            issue_comments_5d: 0,
            issue_comments_15d: 0,
            issue_comments_30d: 0,
            issue_labels: Default::default(),
            recent_issues: vec![],
            issue_heat: None,
            commit_heat: None,
            focus: None,
            last_commit_at: None,
            branch_heat: vec![],
            recent_prs: vec![],
            visible: true,
            visible_reason: None,
            layer: std::collections::BTreeSet::from([Layer::Isolated]),
            has_default_binary: false,
        };
        let edge = |s: &str, t: &str, kind: EdgeKind| Edge {
            source: s.into(),
            target: t.into(),
            kind,
            declaring_branches: vec!["main".into()],
            recency: None,
            version: None,
        };
        let g = jzombie_graph::output::render_json(&Graph::new(
            vec![node("lib"), node("mid"), node("app")],
            vec![
                edge("me/mid", "me/lib", EdgeKind::Manifest),
                edge("me/app", "me/mid", EdgeKind::Manifest),
                edge("me/app", "me/lib", EdgeKind::Docs),
            ],
        ))
        .unwrap();
        let md = render_data_table_md(&g).unwrap();
        // Formatter pads cells, so parse the header like the top-10 test.
        let header = md.lines().find(|l| l.starts_with('|')).unwrap();
        let cols: Vec<String> = header
            .split('|')
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty() && !c.chars().all(|c| c == '-'))
            .collect();
        assert_eq!(
            cols,
            vec![
                "repo",
                "about",
                "commits/30d",
                "heat",
                "layer",
                "reuse (portfolio)"
            ],
            "single reuse column:\n{md}"
        );
        let app_row = md
            .lines()
            .find(|l| l.contains("[app](https://github.com/me/app)"))
            .unwrap()
            .to_string();
        assert!(
            app_row.contains("<sub>uses</sub><br>[mid](https://github.com/me/mid)"),
            "labeled uses line:\n{app_row}"
        );
        assert!(
            !app_row.contains("[lib](https://github.com/me/lib)"),
            "docs association never counts:\n{app_row}"
        );
        let lib_row = md
            .lines()
            .find(|l| l.contains("[lib](https://github.com/me/lib)"))
            .unwrap()
            .to_string();
        assert!(
            lib_row.contains("<sub>used by</sub><br>[mid](https://github.com/me/mid)"),
            "labeled used-by line:\n{lib_row}"
        );
        let mid_row = md
            .lines()
            .find(|l| l.contains("[mid](https://github.com/me/mid)<br><sub>★ 0</sub>"))
            .unwrap()
            .to_string();
        assert!(
            mid_row.contains(
                "<sub>used by</sub><br>[app](https://github.com/me/app)<br><hr><sub>uses</sub><br>[lib](https://github.com/me/lib)"
            ),
            "both directions, divider between:\n{mid_row}"
        );
    }

    #[test]
    fn data_table_empty_when_no_commits() {
        let g = graph_with(&[("old", 0)]);
        let md = render_data_table_md(&g).unwrap();
        assert!(md.contains("No commits in trailing 30d"));
    }
    #[test]
    fn data_table_about_carries_topic_search_links() {
        // `#tag` links scope the search to the node's owner; capped at 4.
        use jzombie_graph::graph::{Graph, Layer, Node};
        let node = |name: &str, topics: Vec<&str>| Node {
            id: format!("me/{name}"),
            owner: "me".into(),
            name: name.into(),
            fork: false,
            private: false,
            upstream: None,
            ahead_by: None,
            pushed_at: None,
            description: Some("does things".into()),
            topics: topics.into_iter().map(str::to_string).collect(),
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
            commits_5d: 5,
            commits_15d: 15,
            commits_30d: 30,
            issues_open_1d: 0,
            issues_open_5d: 0,
            issues_open_15d: 0,
            issues_open_30d: 0,
            issues_closed_1d: 0,
            issues_closed_5d: 0,
            issues_closed_15d: 0,
            issues_closed_30d: 0,
            issue_comments_1d: 0,
            issue_comments_5d: 0,
            issue_comments_15d: 0,
            issue_comments_30d: 0,
            issue_labels: Default::default(),
            recent_issues: vec![],
            issue_heat: None,
            commit_heat: None,
            focus: None,
            last_commit_at: None,
            branch_heat: vec![],
            recent_prs: vec![],
            visible: true,
            visible_reason: None,
            layer: std::collections::BTreeSet::from([Layer::Isolated]),
            has_default_binary: false,
        };
        let many: Vec<&str> = vec!["a", "b", "c", "d", "e", "f"];
        let g = jzombie_graph::output::render_json(&Graph::new(
            vec![node("r", many), node("bare", vec![])],
            vec![],
        ))
        .unwrap();
        let md = render_data_table_md(&g).unwrap();
        assert!(
            md.contains("[#a](https://github.com/search?q=user%3Ame+topic%3Aa&type=repositories)"),
            "user-scoped search link:\n{md}"
        );
        assert!(md.contains("<sub>"), "tiny links");
        assert!(md.contains("+2"), "tail capped");
        assert!(!md.contains("topic%3Ae"), "fifth topic cut");
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
                    issues_open_1d: 0,
                    issues_open_5d: 0,
                    issues_open_15d: 0,
                    issues_open_30d: 0,
                    issues_closed_1d: 0,
                    issues_closed_5d: 0,
                    issues_closed_15d: 0,
                    issues_closed_30d: 0,
                    issue_comments_1d: 0,
                    issue_comments_5d: 0,
                    issue_comments_15d: 0,
                    issue_comments_30d: 0,
                    issue_labels: Default::default(),
                    recent_issues: vec![],
                    issue_heat: None,
                    commit_heat: None,
                    focus: Some(1.0),
                    last_commit_at: None,
                    branch_heat: vec![],
                    visible: true,
                    visible_reason: None,
                    recent_prs: vec![],
                    layer: std::collections::BTreeSet::from([Layer::Isolated]),
                    has_default_binary: false,
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
