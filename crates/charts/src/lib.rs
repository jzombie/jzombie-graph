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

/// One pie slice: everything preformatted — the script interpolates only.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
struct SliceRow {
    label: String,
    pct: String,
    color: String,
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
    let mut repos: Vec<(&str, u32)> = graph
        .nodes
        .iter()
        .filter(|nd| !nd.private && jzombie_graph::graph::is_own_work(nd))
        .filter(|nd| nd.commits_5d > 0)
        .map(|nd| (nd.name.as_str(), nd.commits_5d))
        .collect();
    repos.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let total: u32 = repos.iter().map(|(_, c)| c).sum();
    if total == 0 {
        return Ok(vec![]);
    }
    let n = n.max(1);
    let mut kept: Vec<(String, u32)> = repos
        .iter()
        .take(n)
        .map(|(name, c)| (name.to_string(), *c))
        .collect();
    let rest: u32 = repos.iter().skip(n).map(|(_, c)| c).sum();
    if rest > 0 {
        kept.push(("Other".to_string(), rest));
    }
    let kept_len = kept.len();
    let lay = layout(kept_len);
    let mut acc = 0.0;
    let rows = kept
        .into_iter()
        .enumerate()
        .map(|(i, (label, c))| {
            let share = c as f64 / total as f64;
            let sweep = share * 360.0;
            let a0 = -90.0 + acc * 360.0;
            acc += share;
            let color = if label == "Other" {
                palette::OTHER.to_string()
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
                    palette::FOCUS_POSITIVE.to_string()
                } else {
                    palette::FOCUS_NEGATIVE.to_string()
                },
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
    let rows = focus_rows(&graph_json, n_top.max(1) as usize, n_bottom.max(0) as usize)?;
    let h = 44.0 + rows.len() as f64 * 26.0 + 16.0;
    let doc = FocusDoc {
        header: format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" role=\"img\" font-family=\"Helvetica, Arial, sans-serif\"><title>Focus ranking</title><text x=\"16\" y=\"30\" font-size=\"20\" fill=\"{ink}\">Focus ranking</text>",
            f1(480.0),
            f1(h),
            ink = palette::INK
        ),
        footer: "</svg>".to_string(),
        zero_x: f1(260.0),
        zero_y1: f1(44.0),
        zero_y2: f1(h - 16.0),
        bar_h: "14".to_string(),
        fs: "14".to_string(),
        ink: palette::INK.to_string(),
        bar_ink: palette::INK_ON_BAR.to_string(),
        muted: palette::MUTED.to_string(),
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
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS],
        IMPORT [CHARTS]
        IMPORT [STD]
        LET $rows_json: STRING = CHARTS::FOCUS_BARS(#graph_json, 12, 5)
        WRITE rows.json $rows_json
        LET $doc: MAP<ANY> = LOAD_JSON("rows.json")
        WRITE out.svg $doc.header
        APPEND out.svg "<line x1=\"{{ $doc.zero_x }}\" y1=\"{{ $doc.zero_y1 }}\" x2=\"{{ $doc.zero_x }}\" y2=\"{{ $doc.zero_y2 }}\" stroke=\"{{ $doc.muted }}\" stroke-width=\"1\"/>"
        IF $doc.empty {
            APPEND out.svg "<text x=\"{{ $doc.mx }}\" y=\"{{ $doc.my }}\" text-anchor=\"middle\" font-size=\"{{ $doc.fs }}\" fill=\"{{ $doc.muted }}\">{{ $doc.msg }}</text>"
        } ELSE {
            FOR $r: MAP<ANY> IN $doc.rows {
                APPEND out.svg """
                    <clipPath id="{{ $r.clip }}"><rect x="{{ $r.bx }}" y="{{ $r.by }}" width="{{ $r.bw }}" height="{{ $doc.bar_h }}"/></clipPath>
                    <rect x="{{ $r.bx }}" y="{{ $r.by }}" width="{{ $r.bw }}" height="{{ $doc.bar_h }}" fill="{{ $r.color }}"><title>{{ $r.label }} {{ $r.value }}</title></rect>
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
    let rows = share_rows(&graph_json, n.max(1) as usize)?;
    let lay = layout(rows.len());
    let doc = PieDoc {
        header: format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" role=\"img\" font-family=\"Helvetica, Arial, sans-serif\"><title>Trailing-5d commit share</title><text x=\"16\" y=\"30\" font-size=\"20\" fill=\"{ink}\">Trailing-5d commit share</text>",
            f1(lay.w),
            f1(lay.h),
            ink = palette::INK
        ),
        footer: "</svg>".to_string(),
        cx: f1(lay.cx),
        cy: f1(lay.cy),
        r: f1(lay.r),
        cell: "14".to_string(),
        fs: "14".to_string(),
        ink: palette::INK.to_string(),
        muted: palette::MUTED.to_string(),
        mx: f1(lay.w / 2.0),
        my: f1(lay.title_h + 40.0),
        msg: "No commits in trailing 5d".to_string(),
        empty: rows.is_empty(),
        rows,
    };
    Ok(Value::string(serde_json::to_string(&doc)?))
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
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS],
        IMPORT [STD, CHARTS]
        LET $rows_json: STRING = CHARTS::PIE_SLICES(#graph_json, 8)
        WRITE rows.json $rows_json
        LET $doc: MAP<ANY> = LOAD_JSON("rows.json")
        WRITE out.svg $doc.header
        IF $doc.empty {
            APPEND out.svg "<text x=\"{{ $doc.mx }}\" y=\"{{ $doc.my }}\" text-anchor=\"middle\" font-size=\"{{ $doc.fs }}\" fill=\"{{ $doc.muted }}\">{{ $doc.msg }}</text>"
        } ELSE {
            FOR $r: MAP<ANY> IN $doc.rows {
                IF $r.circle {
                    APPEND out.svg """
                        <circle cx="{{ $doc.cx }}" cy="{{ $doc.cy }}" r="{{ $doc.r }}" fill="{{ $r.color }}"><title>{{ $r.label }} {{ $r.pct }}</title></circle>
                        """
                } ELSE {
                    APPEND out.svg """
                        <path d="{{ $r.d }}" fill="{{ $r.color }}"><title>{{ $r.label }} {{ $r.pct }}</title></path>
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
    period: String,
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
/// steady blue, cooling amber (still pulses, slow), dormant faint gray.
/// All stops from the shared palette.
fn heat_color(h: f32) -> &'static str {
    if h > 1.5 {
        palette::FOCUS_POSITIVE
    } else if h > 0.5 {
        palette::CATEGORICAL[1]
    } else if h > 0.05 {
        palette::CATEGORICAL[0]
    } else {
        palette::OTHER
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
            let norm = (h / 6.0).clamp(0.0, 1.0);
            IndicatorRow {
                band: name.to_string(),
                file: format!("indicators/{name}.svg"),
                label: format!("{name} {h:.2}"),
                heat: format!("{h:.2}"),
                color: heat_color(h).to_string(),
                core_opacity: format!("{:.2}", 0.45 + 0.55 * norm as f64),
                ring: h > 0.05,
                period: format!("{:.1}s", 2.6 - norm as f64 * 1.8),
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
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {} {}\" role=\"img\" font-family=\"Helvetica, Arial, sans-serif\"><title>Status indicators</title><style>.ring{{transform-box:fill-box;transform-origin:center;animation:ping 1.6s ease-out infinite}}@keyframes ping{{0%{{transform:scale(1);opacity:.7}}100%{{transform:scale(2.4);opacity:0}}}}@media (prefers-reduced-motion:reduce){{.ring{{animation:none}}}}</style>",
        f1(doc.rows.len() as f64 * INDICATOR_CELL),
        f1(INDICATOR_CELL),
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
fn portfolio(frag: String, share: String, focus: String, table: String) -> anyhow::Result<Value> {
    let header = format!(
        "# portfolio-graph — render proof\n\n*Generated — do not hand-edit. Regenerate with `oxdock scripts/preview-indicators.oxfile`. Fragment carried verbatim; charts embedded as files.*\n\n## Map fragment\n\n{frag}\n## Commit share\n\n![share]({share})\n\n## Focus ranking\n\n![focus]({focus})\n\n",
        frag = frag.trim_end(),
        share = share,
        focus = focus,
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
            // TODO: APPEND_LINE once the engine lands it — the explicit
            // terminator below only exists because APPEND emits none.
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
    table: &str,
) -> anyhow::Result<String> {
    let engine = engine();
    let (frag_arg, share_arg, focus_arg, table_arg) = (
        frag.to_string(),
        share.to_string(),
        focus.to_string(),
        table.to_string(),
    );
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS],
        IMPORT [STD, CHARTS]
        LET $page_json: STRING = CHARTS::PORTFOLIO(#frag_arg, #share_arg, #focus_arg, #table_arg)
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
        WRITE out.svg $doc.header
        FOR $r: MAP<ANY> IN $doc.rows {
            IF $r.ring {
                APPEND out.svg """
                    <circle cx="{{ $r.cx }}" cy="{{ $doc.cy }}" r="{{ $doc.dot_r }}" fill="none" stroke="{{ $r.color }}" stroke-width="{{ $doc.ring_w }}" class="ring" style="animation-duration:{{ $r.period }}"><title>{{ $r.label }} {{ $r.heat }}</title></circle>
                    """
            } ELSE {
                APPEND out.svg """
                    <circle cx="{{ $r.cx }}" cy="{{ $doc.cy }}" r="{{ $doc.dot_r }}" fill="none" stroke="{{ $r.color }}" stroke-width="{{ $doc.ring_w }}"><title>{{ $r.label }} {{ $r.heat }}</title></circle>
                    """
            }
            APPEND out.svg """
                <circle cx="{{ $r.cx }}" cy="{{ $doc.cy }}" r="{{ $doc.dot_r }}" fill="{{ $r.color }}" opacity="{{ $r.core_opacity }}"><title>{{ $r.label }} {{ $r.heat }}</title></circle>
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
                commits_5d: *c5,
                commits_30d: *c30,
                commit_heat: None,
                focus: None,
                last_commit_at: None,
                branches: vec!["main".into()],
                visible: true,
                visible_reason: None,
                layer: Layer::Isolated,
            })
            .collect();
        jzombie_graph::output::render_json(&Graph::new(nodes, vec![])).unwrap()
    }

    fn graph_with_focus(scores: &[(&str, Option<f32>)]) -> String {
        use jzombie_graph::graph::{Graph, Layer, Node};
        let nodes = scores
            .iter()
            .map(|(name, f)| Node {
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
                commits_5d: 0,
                commits_30d: 0,
                commit_heat: None,
                focus: *f,
                last_commit_at: None,
                branches: vec!["main".into()],
                visible: true,
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
        assert_eq!(heat_color(6.0), palette::FOCUS_POSITIVE);
        assert_eq!(heat_color(1.0), palette::CATEGORICAL[1]);
        assert_eq!(heat_color(0.3), palette::CATEGORICAL[0]);
        assert_eq!(heat_color(0.0), palette::OTHER);
    }

    #[test]
    fn indicators_render_pure_css_pulse() {
        let strip = render_indicators_svg().unwrap();
        assert!(strip.starts_with("<svg "));
        assert!(strip.ends_with("</svg>"));
        // Embedded CSS keyframes — no SMIL, no scripts, no text:
        // the table around the badge names the repo.
        assert!(strip.contains("@keyframes ping"));
        assert!(strip.contains("animation-duration:1.7s"));
        assert!(!strip.contains("<animate"));
        assert!(!strip.contains("<script"));
        assert!(!strip.contains("<text"));
        // All four bands present, heating fastest.
        for band in ["heating", "steady", "cooling", "dormant"] {
            assert!(strip.contains(band), "missing {band}");
        }
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
        let md = render_preview_md("# Frag\n", "s.svg", "f.svg", &table).unwrap();
        // Fragment verbatim, embeds by file name, indicator rows inlined.
        assert!(md.contains("# Frag\n"));
        assert!(md.contains("![share](s.svg)"));
        assert!(md.contains("![focus](f.svg)"));
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
        let scores: Vec<(&str, Option<f32>)> = (0..20)
            .map(|i| {
                let name: &'static str = Box::leak(format!("r{i:02}").into_boxed_str());
                (name, Some(10.0 - i as f32))
            })
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
        assert!(svg.contains(palette::INK));
        assert!(svg.contains(palette::INK_ON_BAR));
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
                    commits_5d: 4,
                    commits_30d: 24,
                    commit_heat: None,
                    focus: Some(1.0),
                    last_commit_at: None,
                    branches: vec!["main".into()],
                    visible: true,
                    visible_reason: None,
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
