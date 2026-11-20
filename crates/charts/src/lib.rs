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

use indoc::formatdoc;
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
    /// Name position: right-anchored in the left column for positive
    /// rows; left-anchored right of the axis for negative rows, whose
    /// bars extend into the left column.
    tx: String,
    ty: String,
    na: String,
    /// Value position: tabular in the left column for positive rows;
    /// tabular right of the axis for negative rows. Labels always sit
    /// on white, never on a bar — one ink pass, no clip paths, so every
    /// renderer draws the same pixels.
    vx: String,
    va: String,
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
    // Fixed columns for positive rows: names right-anchored at
    // `name_x`, values right-anchored at `value_x`, zero axis, bars to
    // the right — all on white. Negative rows mirror right of the
    // axis (labels start past it, values end a bar-width beyond),
    // because their bars extend left into the label columns.
    // One ink pass, no clipped duplicates: every renderer draws the
    // same pixels, and opacity animation on bars can't ghost the text.
    let title_h = 44.0;
    let row_h = 26.0;
    let name_x = 130.0;
    let value_x = 200.0;
    let zero_x = 260.0;
    let max_bar = 200.0;
    let neg_name_x = zero_x + 8.0;
    let neg_value_x = zero_x + max_bar;
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
            let (tx, na, vx, va) = if f >= 0.0 {
                (name_x, "end", value_x, "end")
            } else {
                (neg_name_x, "start", neg_value_x, "end")
            };
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
                tx: f1(tx),
                ty: f1(ty),
                na: na.to_string(),
                vx: f1(vx),
                va: va.to_string(),
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
                APPEND out.svg "<rect x=\"{{ $r.bx }}\" y=\"{{ $r.by }}\" width=\"{{ $r.bw }}\" height=\"{{ $doc.bar_h }}\" fill=\"{{ $r.color }}\" class=\"{{ $r.pulse }}\"><title>{{ $r.label }} {{ $r.value }}</title></rect>"
                APPEND out.svg "<text x=\"{{ $r.tx }}\" y=\"{{ $r.ty }}\" text-anchor=\"{{ $r.na }}\" font-size=\"{{ $doc.fs }}\" fill=\"{{ $doc.ink }}\">{{ $r.label }}</text><text x=\"{{ $r.vx }}\" y=\"{{ $r.ty }}\" text-anchor=\"{{ $r.va }}\" font-size=\"{{ $doc.fs }}\" fill=\"{{ $doc.ink }}\">{{ $r.value }}</text>"
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
    ptx: String,
    /// Twin embers pinned to the bar's right end: same band color,
    /// overlapping, each a third of a cycle behind the last so the
    /// trio ripples instead of breathing in sync.
    pfx1: String,
    pfx2: String,
    pfx3: String,
    port_alt_delay: String,
    port_alt_delay2: String,
    port_color: String,
    port_pulse: String,
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
        ptx: String::new(),
        pfx1: String::new(),
        pfx2: String::new(),
        pfx3: String::new(),
        port_alt_delay: String::new(),
        port_alt_delay2: String::new(),
        port_color: String::new(),
        port_pulse: String::new(),
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
    // Ember trio for the portfolio bar: band color, breathing with the
    // band's motion (heating fast, steady slow, else static). Each runs
    // a third of a cycle behind the last so the trio ripples.
    let (port_color, port_pulse, port_alt_delay, port_alt_delay2) = match heat_band(port_heat) {
        "heating" => (palette::heating(), "pulse", "-0.73s", "-1.47s"),
        "steady" => (palette::steady(), "pulse slow", "-1.13s", "-2.27s"),
        "cooling" => (palette::cooling(), "", "0s", "0s"),
        _ => (palette::dormant(), "", "0s", "0s"),
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
        ptx: f1(28.0),
        // Ember trio pinned to the bar's right end (12px pad, 7px
        // stagger): label length never matters, so nightly numbers
        // can't collide with them.
        pfx1: f1(FLAME_X0 + FLAME_TRACK - 42.0),
        pfx2: f1(FLAME_X0 + FLAME_TRACK - 35.0),
        pfx3: f1(FLAME_X0 + FLAME_TRACK - 28.0),
        port_color,
        port_pulse: port_pulse.to_string(),
        port_alt_delay: port_alt_delay.to_string(),
        port_alt_delay2: port_alt_delay2.to_string(),
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
            APPEND out.svg "<text x=\"{{ $doc.ptx }}\" y=\"{{ $doc.pty }}\" class=\"lbl\">{{ $doc.port_label }}</text>"
            APPEND out.svg "<svg x=\"{{ $doc.pfx1 }}\" y=\"61\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\" fill=\"{{ $doc.port_color }}\" class=\"{{ $doc.port_pulse }}\"><path d=\"M8 0 C10 3 13 6 13 10 A5 5 0 0 1 3 10 C3 8 4 6.5 5 5.5 C5.5 7 6.5 7.5 6.5 7.5 C6 5 7 2.5 8 0 Z\"/></svg>"
            APPEND out.svg "<svg x=\"{{ $doc.pfx2 }}\" y=\"60\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\" fill=\"{{ $doc.port_color }}\" class=\"{{ $doc.port_pulse }}\" style=\"animation-delay:{{ $doc.port_alt_delay }}\"><path d=\"M8 0 C10 3 13 6 13 10 A5 5 0 0 1 3 10 C3 8 4 6.5 5 5.5 C5.5 7 6.5 7.5 6.5 7.5 C6 5 7 2.5 8 0 Z\"/></svg>"
            APPEND out.svg "<svg x=\"{{ $doc.pfx3 }}\" y=\"59\" width=\"16\" height=\"16\" viewBox=\"0 0 16 16\" fill=\"{{ $doc.port_color }}\" class=\"{{ $doc.port_pulse }}\" style=\"animation-delay:{{ $doc.port_alt_delay2 }}\"><path d=\"M8 0 C10 3 13 6 13 10 A5 5 0 0 1 3 10 C3 8 4 6.5 5 5.5 C5.5 7 6.5 7.5 6.5 7.5 C6 5 7 2.5 8 0 Z\"/></svg>"
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

/// Selected-work list: top-`SELECTED_TOPN` own-work repos by
/// `selected_score`, emitted as one markdown list — name plus
/// description first, then topics, then proof numbers (heat, commits,
/// stars, dependents, homepage) on a shrunk line. Markdown rather than
/// cards: it reflows at any width, reads in light and dark themes, and
/// screen readers get text instead of an image's `<desc>`.
const SELECTED_TOPN: usize = 9;

/// One list row: everything the fragment prints, shaped in Rust so the
/// renderer below carries no graph access.
#[derive(Debug, Clone, PartialEq)]
struct SelectedRow {
    /// Canonical `owner/repo` id — the radar file stem derives from it,
    /// so the fragment and the radar file family can never disagree.
    id: String,
    /// Repo URL and display name.
    url: String,
    name: String,
    /// Empty when the repo has none — the description line drops out
    /// rather than printing a placeholder.
    desc: String,
    /// Owner login: the topic search links are user-scoped, never baked.
    owner: String,
    /// Repo topics, all of them, each linked to a user-scoped search.
    topics: Vec<String>,
    /// Repo homepage (site or crate page); `None` when unset or blank.
    homepage: Option<String>,
    /// `fork of owner/parent (+N own commits)` when the row is a
    /// diverged fork; `None` for original work.
    fork_note: Option<String>,
    /// Heat band word; doubles as the badge file name.
    band: &'static str,
    /// Velocity and pull: trailing-30d commits, stars, forks, backlog.
    commits_30d: u32,
    stars: u32,
    forks: u32,
    open_issues: u32,
    /// Dependence edges both ways — who leans on this repo, what it
    /// leans on. Associations (docs, workflows) never count.
    fan_in: u32,
    fan_out: u32,
    /// Positional tags (`middleware · application`) plus identity facts.
    layer: String,
    language: Option<String>,
    license: Option<String>,
    focus: Option<f32>,
    last_commit: Option<String>,
    branches: usize,
    size_kb: u32,
    /// Agent-parseability audit from the default-branch tree: which of
    /// readme / license / llms.txt / agents.md exist.
    surfaces: Vec<&'static str>,
    /// Thread traffic in the trailing 30d, the issue gauge, and the
    /// label census over the sampled threads (bugs vs enhancements).
    issues_open_30d: u32,
    issues_closed_30d: u32,
    issue_comments_30d: u32,
    issue_heat: Option<f32>,
    labels: Vec<(String, u32)>,
    /// Recent PRs: captured total, counts per state. Titles are never
    /// rendered — this is a dashboard, not a `git log`.
    prs_total: usize,
    prs_states: Vec<(String, usize)>,
    /// Sampled planning threads: captured total and state counts.
    /// Titles are never rendered.
    issues_total: usize,
    issues_states: Vec<(String, usize)>,
}

/// Titles are gone: the review correctly called PR/issue titles signal
/// destruction (a Dependabot bump teaches nothing at portfolio scale).
/// Counts only.
const SELECTED_TAG_MAX: usize = 4;

/// Repo size as the API reports it (KB) in the units a reader expects.
fn size_label(size_kb: u32) -> String {
    if size_kb >= 1024 {
        format!("{:.1} MB", size_kb as f64 / 1024.0)
    } else {
        format!("{size_kb} KB")
    }
}

/// Flagship score: load-bearing reuse first — one portfolio dependent
/// outweighs any star — then external pull, then product-ness and recent
/// shipping. Diverged forks pay for being forks: original work reads as
/// the flagship. Pure ranking: no units, no meaning beyond the order it
/// produces, which is why the section prose states the measure.
fn selected_score(
    fan_in: u32,
    stars: u32,
    forks: u32,
    application: bool,
    has_readme: bool,
    commits_30d: u32,
    fork: bool,
) -> f64 {
    2.5 * f64::from(fan_in)
        + 1.5 * (1.0 + f64::from(stars)).ln()
        + 0.5 * (1.0 + f64::from(forks)).ln()
        + if application { 2.0 } else { 0.0 }
        + if has_readme { 0.5 } else { 0.0 }
        + 0.5 * (1.0 + f64::from(commits_30d)).ln()
        - if fork { 1.0 } else { 0.0 }
}

/// GitHub profile README repo (`owner` equals `name`, case-insensitive):
/// a whole-portfolio documentation index, never selected work. Same rule
/// the cartographer uses to keep it out of the cluster.
fn is_profile_repo(id: &str) -> bool {
    let Some((owner, name)) = id.split_once('/') else {
        return false;
    };
    owner.eq_ignore_ascii_case(name)
}

/// Selected rows: public own-work repos with commits in the trailing 30d
/// — the pool the pins drew from — scored, sorted, cut to the top. Ties
/// break on name, so the order is a pure function of the JSON. Takes a
/// parsed graph: the fragment shares one parse with the radar readouts.
fn selected_rows(graph: &jzombie_graph::graph::Graph) -> Vec<SelectedRow> {
    let fan_in = graph.node_fan_in();
    let mut scored: Vec<(f64, &jzombie_graph::graph::Node)> = graph
        .nodes
        .iter()
        .filter(|nd| {
            !nd.private
                && jzombie_graph::graph::is_own_work(nd)
                && nd.commits_30d > 0
                && !is_profile_repo(&nd.id)
        })
        .map(|nd| {
            let fi = fan_in.get(&nd.id).copied().unwrap_or(0);
            let score = selected_score(
                fi,
                nd.stars,
                nd.forks_count,
                nd.layer.contains(&jzombie_graph::graph::Layer::Application),
                nd.has_readme,
                nd.commits_30d,
                nd.fork,
            );
            (score, nd)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.name.cmp(&b.1.name))
    });
    scored.truncate(SELECTED_TOPN);
    let fan_out = graph.node_fan_out();
    scored
        .into_iter()
        .map(|(_, nd)| {
            let mut surfaces = vec![];
            if nd.has_readme {
                surfaces.push("readme");
            }
            if nd.has_license_file {
                surfaces.push("license");
            }
            if nd.has_llms_txt {
                surfaces.push("llms.txt");
            }
            if nd.has_agents_md {
                surfaces.push("agents.md");
            }
            let mut labels: Vec<(String, u32)> = nd
                .issue_labels
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect();
            // Loudest label first, name breaks ties: one order, every run.
            labels.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let counts = |states: &[&str], hit: &dyn Fn(&str) -> usize| -> Vec<(String, usize)> {
                states.iter().map(|s| ((*s).to_string(), hit(s))).collect()
            };
            let prs_total = nd.recent_prs.len();
            let prs_states = counts(&["merged", "open", "closed"], &|s| {
                nd.recent_prs
                    .iter()
                    .filter(|p| p.state.eq_ignore_ascii_case(s))
                    .count()
            });
            let issues_total = nd.recent_issues.len();
            let issues_states = counts(&["open", "closed"], &|s| {
                nd.recent_issues
                    .iter()
                    .filter(|i| i.state.eq_ignore_ascii_case(s))
                    .count()
            });
            let fork_note = if nd.fork {
                Some(match (nd.upstream.as_deref(), nd.ahead_by) {
                    (Some(up), Some(ahead)) if ahead > 0 => {
                        format!("fork of {up} (+{ahead} own commits)")
                    }
                    (Some(up), _) => format!("fork of {up}"),
                    (None, _) => "fork".to_string(),
                })
            } else {
                None
            };
            SelectedRow {
                id: nd.id.clone(),
                url: format!("https://github.com/{}", nd.id),
                name: nd.name.clone(),
                desc: nd.description.clone().unwrap_or_default(),
                band: heat_band(nd.commit_heat.or_else(|| nd.commit_heat())),
                commits_30d: nd.commits_30d,
                stars: nd.stars,
                forks: nd.forks_count,
                open_issues: nd.open_issues,
                fan_in: fan_in.get(&nd.id).copied().unwrap_or(0),
                fan_out: fan_out.get(&nd.id).copied().unwrap_or(0),
                owner: nd.owner.clone(),
                topics: nd.topics.clone(),
                homepage: nd
                    .homepage
                    .as_deref()
                    .map(str::trim)
                    .filter(|h| !h.is_empty())
                    .map(str::to_string),
                fork_note,
                layer: jzombie_graph::graph::Graph::layer_string(&nd.layer),
                language: nd.language.clone(),
                license: nd.license.clone(),
                focus: nd.focus,
                last_commit: nd.last_commit_at.map(|t| t.format("%Y-%m-%d").to_string()),
                branches: nd.branch_heat.len(),
                size_kb: nd.size_kb,
                surfaces,
                issues_open_30d: nd.issues_open_30d,
                issues_closed_30d: nd.issues_closed_30d,
                issue_comments_30d: nd.issue_comments_30d,
                issue_heat: nd.issue_heat,
                labels,
                prs_total,
                prs_states,
                issues_total,
                issues_states,
            }
        })
        .collect()
}

/// Render the selected-work fragment: heading, the sentence that states
/// what the order measures, the legend for the shape radars, then one
/// bullet per row — a floated radar, description, topics (capped),
/// classification, proof, consolidated thread counts. The radar floats
/// `right` so it keeps its configured width at any viewport: inside a
/// markdown table the text column steals width on narrow screens and
/// the image shrinks to illegibility on mobile.
/// The markdown itself carries no cache-bust token (text re-renders with
/// the commit); the radar images do, because they are proxied and cached
/// like every other image on the page.
pub fn render_selected_md(graph_json: &str) -> anyhow::Result<String> {
    let graph = jzombie_graph::graph::Graph::from_json(graph_json)?;
    let rows = selected_rows(&graph);
    if rows.is_empty() {
        return Ok("## Selected work\n\n*No commits in trailing 30d*\n".to_string());
    }
    let plan = RadarPlan::load()?;
    let bust = bust_token(graph_json)?;
    let mut items = String::new();
    for (row_idx, row) in rows.iter().enumerate() {
        let link = format!("[{}]({})", row.name, row.url);
        let head = if row.desc.is_empty() {
            format!("**{link}**")
        } else {
            // Descriptions are single-line on GitHub; flatten anyway so
            // one stray newline cannot break the list. The whole head
            // line is bold — name and description read as one title.
            let desc = row.desc.split_whitespace().collect::<Vec<_>>().join(" ");
            format!("**{link}: {desc}**")
        };
        let radar = radar_img(&plan, &graph, row, &bust)?;
        // The radar rides on the head line (no `<br>` before it — two
        // breaks in a row would print a blank line) so every fact line
        // flows around the float, and the float keeps its width on
        // mobile where a table column would surrender it.
        let mut lines: Vec<String> = vec![format!("- {head} {radar}")];
        // Topics capped: the first N links carry the architecture, the
        // rest collapse to a `+N more` count instead of nine search links
        // that bury the metrics.
        if !row.topics.is_empty() {
            let mut tags: Vec<String> = row
                .topics
                .iter()
                .take(SELECTED_TAG_MAX)
                .map(|t| topic_link(&row.owner, t))
                .collect();
            if row.topics.len() > SELECTED_TAG_MAX {
                tags.push(format!("+{} more", row.topics.len() - SELECTED_TAG_MAX));
            }
            lines.push(format!("<sub>{}</sub>", tags.join(" ")));
        }

        // One classification line: what the repo *is* and how it
        // *stands*. Back in a list (not a table cell) pipes are safe
        // again, so identity and state separate into two groups.
        let mut identity: Vec<String> = vec![format!("layer: `{}`", row.layer)];
        if let Some(lang) = &row.language {
            identity.push(lang.clone());
        }
        if let Some(lic) = &row.license {
            identity.push(lic.clone());
        }
        if let Some(note) = &row.fork_note {
            identity.push(note.clone());
        }
        if !row.surfaces.is_empty() {
            identity.push(format!("docs: {}", row.surfaces.join(", ")));
        }
        let mut state: Vec<String> = vec![];
        if let Some(f) = row.focus {
            state.push(format!("focus: {f:+.2}"));
        }
        if let Some(d) = &row.last_commit {
            state.push(format!("last commit: {d}"));
        }
        state.push(if row.branches == 1 {
            "branch: 1".to_string()
        } else {
            format!("branches: {}", row.branches)
        });
        if row.size_kb > 0 {
            state.push(format!("size: {}", size_label(row.size_kb)));
        }
        let mut groups: Vec<String> = vec![];
        if !identity.is_empty() {
            groups.push(identity.join(" \u{b7} "));
        }
        if !state.is_empty() {
            groups.push(state.join(" \u{b7} "));
        }
        lines.push(format!("<sub>{}</sub>", groups.join(" | ")));

        // The proof line: gauge, volume, pull, backlog, reuse, site — the
        // full URL, never a truncated host. The badge file carries its own
        // 44px design size; inline here it asks for 24. Vertical centering
        // uses the legacy `absmiddle`, because GitHub strips `style` — and
        // `middle` is not a synonym: browsers map it to middle-on-
        // baseline, which measures ~4px low against the text, while
        // `absmiddle` is true centering (measured within 0.5px).
        let commits = if row.commits_30d == 1 {
            "1 commit in 30d".to_string()
        } else {
            format!("{} commits in 30d", row.commits_30d)
        };
        let mut proof = format!(
            "<img src=\"{badge}\" width=\"24\" align=\"absmiddle\" alt=\"\"> {band} \u{b7} {commits} \u{b7} \u{2605}{stars} \u{b7} \u{2442}{forks} \u{b7} {open} open issues | used by {fan_in} \u{b7} uses {fan_out}",
            badge = jzombie_graph::paths::indicator_url(row.band),
            band = row.band,
            commits = commits,
            stars = row.stars,
            forks = row.forks,
            open = row.open_issues,
            fan_in = row.fan_in,
            fan_out = row.fan_out,
        );
        if let Some(home) = &row.homepage {
            proof.push_str(&format!(" \u{b7} [{home}]({home})"));
        }
        lines.push(format!("<sub>{proof}</sub>"));

        // One consolidated thread line: window counts, gauge, census,
        // then lifetime totals — counts only, never titles. The old
        // layout printed the 30d window and the lifetime states as two
        // near-identical lines plus literal PR/issue titles; this is the
        // single unified metric.
        if row.prs_total > 0
            || row.issues_total > 0
            || row.issues_open_30d > 0
            || row.issues_closed_30d > 0
        {
            let mut threads: Vec<String> = vec![];
            if row.prs_total > 0 {
                threads.push(thread_counts(
                    &format!("PRs ({})", row.prs_total),
                    &row.prs_states,
                ));
            }
            if row.issues_total > 0 {
                threads.push(thread_counts(
                    &format!("issues ({})", row.issues_total),
                    &row.issues_states,
                ));
            }
            let mut window = vec![format!(
                "issues 30d: {} opened \u{b7} {} closed \u{b7} {} comments",
                row.issues_open_30d, row.issues_closed_30d, row.issue_comments_30d
            )];
            if let Some(h) = row.issue_heat {
                window.push(format!("issue heat: {h:.2}"));
            }
            threads.push(window.join(" \u{b7} "));
            if !row.labels.is_empty() {
                let census: Vec<String> = row
                    .labels
                    .iter()
                    .map(|(l, c)| format!("{l}\u{d7}{c}"))
                    .collect();
                threads.push(format!("label census: {}", census.join(", ")));
            }
            lines.push(format!("<sub>{}</sub>", threads.join(" \u{b7} ")));
        }

        // Join with literal `<br>`: markdown needs a hard break inside a
        // list item, and GitHub keeps raw HTML tags. The row closes with
        // `<br clear="both">`: the radar floats right at ~165px tall, so
        // a short row would otherwise spill its float into the next
        // bullet and shove that row's radar left. The clearing break
        // forces each `<li>` to contain its own float — GitHub's
        // sanitizer keeps the `clear` attribute. A `<hr>` inside the
        // bullet separates rows (a rule at column 0 would split the
        // `<ul>` in two, verified against the API); the last row skips
        // it so no rule dangles before the next section.
        items.push_str(&lines.join("<br>"));
        items.push_str("<br clear=\"both\">");
        if row_idx + 1 < rows.len() {
            items.push_str("<hr>");
        }
        items.push('\n');
    }
    // The legend reads in dial order — the same order the vertices turn
    // — so a reader can name a spoke without counting from the top.
    let axis_legend = plan
        .axes
        .iter()
        .map(|(label, _)| label.as_str())
        .collect::<Vec<_>>()
        .join(" \u{b7} ");
    let pool_label = plan.pool_label();
    Ok(formatdoc! {r#"
        ## Selected work

        Ordered by load-bearing reuse first: how many portfolio repos depend on a repo, then external pull (stars and forks), then recent activity, with product repos and documented ones nudged up. It’s a ranking of signal, not a tally of volume. Exact counts, layers, and reuse ride the commit data table below.

        Each row carries a shape radar: axes clockwise from top ({axis_legend}), and each vertex is that repo’s percentile among {pool_label}.

        {items}"#})
}

/// `label: 17 merged · 1 open · 2 closed` — counts only. Titles were cut:
/// a Dependabot bump teaches nothing at portfolio scale, and the samples
/// doubled the row height while burying heat, reuse, and layer.
fn thread_counts(label: &str, states: &[(String, usize)]) -> String {
    let counts: Vec<String> = states.iter().map(|(s, c)| format!("{c} {s}")).collect();
    format!("{label}: {}", counts.join(" \u{b7} "))
}

// --- Shape radar -------------------------------------------------------
//
// One hexagon per selected repo, floated right in its list row. Same
// split as every chart here: Rust owns percentiles and geometry, the
// embedded script owns markup emission only.

/// Radar config (`radar.toml`): which metrics, against which peers.
/// Loaded on every render — the file is tiny, and a config mistake
/// should surface as a render error, never as a stale cached shape.
#[derive(Debug, Clone, serde::Deserialize)]
struct RadarConfig {
    /// Floated image width in pixels (the GitHub `width` attribute).
    width: u32,
    pool: RadarPool,
    axes: Vec<RadarAxisCfg>,
}

/// The peer group percentiles rank against. `label` is required: the
/// legend and every alt text must be able to say what the numbers are
/// *among*, and a count baked into prose would drift as the graph grows.
#[derive(Debug, Clone, serde::Deserialize)]
struct RadarPool {
    label: String,
    /// Keep only own work: diverged forks stay, pristine ones sit out.
    #[serde(default)]
    own_work: bool,
    /// Keep only repos with at least this many trailing-30d commits.
    #[serde(default)]
    min_commits_30d: Option<u32>,
    /// Drop the profile README repo (`owner` == `name`).
    #[serde(default)]
    exclude_profile: bool,
}

/// One axis as configured: a metric key plus the words a reader sees.
#[derive(Debug, Clone, serde::Deserialize)]
struct RadarAxisCfg {
    metric: String,
    label: String,
}

/// Graph-derived facts an axis may read that are not node fields.
struct RadarCtx {
    fan_in: u32,
    fan_out: u32,
}

/// One primitive metric: how to read a value off a node. `radar.toml`
/// chooses which become axes — a new *combination* is a config line,
/// a new primitive is one line here.
type Metric = fn(&jzombie_graph::graph::Node, &RadarCtx) -> f64;

fn or_zero(v: Option<f32>) -> f64 {
    v.unwrap_or(0.0) as f64
}

/// Every metric `radar.toml` may name. Values stay `f64` end to end so
/// percentiles never truncate, and every axis lands on the same 0..1
/// scale — shapes drawn on different scales cannot be compared.
const RADAR_METRICS: &[(&str, Metric)] = &[
    ("commits_1d", |n, _| n.commits_1d as f64),
    ("commits_5d", |n, _| n.commits_5d as f64),
    ("commits_15d", |n, _| n.commits_15d as f64),
    ("commits_30d", |n, _| n.commits_30d as f64),
    ("issue_traffic", |n, _| {
        (n.issues_open_30d + n.issues_closed_30d) as f64
    }),
    ("issue_comments", |n, _| n.issue_comments_30d as f64),
    ("pull_requests", |n, _| n.recent_prs.len() as f64),
    ("stars", |n, _| n.stars as f64),
    ("forks", |n, _| n.forks_count as f64),
    ("fan_in", |_, c| c.fan_in as f64),
    ("fan_out", |_, c| c.fan_out as f64),
    ("focus", |n, _| n.focus.unwrap_or(0.0) as f64),
    ("open_issues", |n, _| n.open_issues as f64),
    ("branches", |n, _| n.branch_heat.len() as f64),
    ("size_kb", |n, _| n.size_kb as f64),
    ("commit_heat", |n, _| {
        or_zero(n.commit_heat.or_else(|| n.commit_heat()))
    }),
    ("issue_heat", |n, _| or_zero(n.issue_heat)),
];

/// The peer group percentiles rank against. Private nodes never qualify
/// (they are not portfolio signal); everything else is config.
fn radar_pool<'a>(
    graph: &'a jzombie_graph::graph::Graph,
    cfg: &RadarConfig,
) -> Vec<&'a jzombie_graph::graph::Node> {
    graph
        .nodes
        .iter()
        .filter(|nd| {
            if nd.private {
                return false;
            }
            if cfg.pool.own_work && !jzombie_graph::graph::is_own_work(nd) {
                return false;
            }
            if let Some(min) = cfg.pool.min_commits_30d
                && nd.commits_30d < min
            {
                return false;
            }
            !(cfg.pool.exclude_profile && is_profile_repo(&nd.id))
        })
        .collect()
}

/// Mid-rank percentile of `v` among `pool`: ties share the middle of
/// their run (a pool where most repos have zero stars doesn't pretend
/// they all sit at zero), negatives and outliers need no special
/// casing, and the result is always 0..1 — the shared scale every axis
/// is drawn on.
fn percentile(pool: &[f64], v: f64) -> f64 {
    if pool.is_empty() {
        return 0.0;
    }
    let below = pool.iter().filter(|&&x| x < v).count();
    let ties = pool.iter().filter(|&&x| x == v).count();
    (below as f64 + 0.5 * ties as f64) / pool.len() as f64
}

/// Config with every axis resolved to its metric. A typo'd `metric`
/// key fails here, naming the keys that exist, instead of rendering a
/// spoke nobody can explain.
#[derive(Debug)]
struct RadarPlan {
    cfg: RadarConfig,
    /// Display label and reader, in configured (dial) order.
    axes: Vec<(String, Metric)>,
}

impl RadarPlan {
    fn load() -> anyhow::Result<Self> {
        let cfg: RadarConfig = toml::from_str(include_str!("../radar.toml"))
            .map_err(|e| anyhow::anyhow!("radar.toml: {e}"))?;
        Self::resolve(cfg)
    }

    /// Validation: axes declared, width sane, every `metric` naming a
    /// registry key. Split from `load` so the typo path is testable
    /// against a config the test writes itself.
    fn resolve(cfg: RadarConfig) -> anyhow::Result<Self> {
        anyhow::ensure!(!cfg.axes.is_empty(), "radar.toml declares no axes");
        anyhow::ensure!(cfg.width > 0, "radar.toml: width must be positive");
        let mut axes = Vec::with_capacity(cfg.axes.len());
        for ax in &cfg.axes {
            let Some((_, metric)) = RADAR_METRICS.iter().find(|(key, _)| *key == ax.metric) else {
                let mut keys: Vec<&str> = RADAR_METRICS.iter().map(|(k, _)| *k).collect();
                keys.sort_unstable();
                anyhow::bail!(
                    "radar.toml: unknown metric {:?}; available: {}",
                    ax.metric,
                    keys.join(", ")
                );
            };
            axes.push((ax.label.clone(), *metric));
        }
        Ok(Self { cfg, axes })
    }

    fn pool_label(&self) -> &str {
        &self.cfg.pool.label
    }

    /// Percentiles for one node, in axis order — the numbers behind the
    /// shape, the hover title and the alt text, so the three can never
    /// disagree with each other.
    fn percentiles(
        &self,
        graph: &jzombie_graph::graph::Graph,
        node: &jzombie_graph::graph::Node,
    ) -> Vec<f64> {
        let fan_in = graph.node_fan_in();
        let fan_out = graph.node_fan_out();
        let ctx = |nd: &jzombie_graph::graph::Node| RadarCtx {
            fan_in: fan_in.get(&nd.id).copied().unwrap_or(0),
            fan_out: fan_out.get(&nd.id).copied().unwrap_or(0),
        };
        let pool = radar_pool(graph, &self.cfg);
        self.axes
            .iter()
            .map(|(_, metric)| {
                let values: Vec<f64> = pool.iter().map(|&nd| metric(nd, &ctx(nd))).collect();
                percentile(&values, metric(node, &ctx(node)))
            })
            .collect()
    }
}

/// `99` -> "99th": alt text reads as prose, not as a table.
fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// The numbers the shape only implies, spelled out: a screen reader
/// gets the same facts a sighted reader gets from the vertices, and
/// the hover title says the same words.
fn radar_readout(plan: &RadarPlan, percentiles: &[f64]) -> String {
    let facts: Vec<String> = plan
        .axes
        .iter()
        .zip(percentiles)
        .map(|((label, _), p)| {
            let rank = ordinal(((p * 100.0).round() as usize).min(100));
            format!("{label} {rank} percentile")
        })
        .collect();
    format!(
        "Radar chart of percentiles among {}: {}",
        plan.pool_label(),
        facts.join(", ")
    )
}

/// Geometry for one radar, precomputed — the script interpolates, Rust
/// does the math. Every vertex sits at `radius × percentile` of the
/// shared pool, which is what makes nine shapes comparable.
#[derive(Debug, Clone, serde::Serialize)]
struct RadarDoc {
    header: String,
    footer: String,
    /// Center of the fitted canvas, shared by rings, spokes, vertices.
    cx: String,
    cy: String,
    /// Drawing sizes the script interpolates: label font and vertex dot.
    font: String,
    dot: String,
    /// Grid ink: the same mid-gray every chart draws scaffolding in.
    ink: String,
    /// Heat-band color — the polygon wears the row's badge color.
    color: String,
    /// Breathing class from the repo's heat, as the share slices carry.
    pulse: String,
    /// Vertex `x,y` pairs, clockwise from top.
    points: String,
    rings: Vec<RadarRing>,
    spokes: Vec<RadarSpoke>,
    dots: Vec<RadarDot>,
    /// Axis labels, one entry per rendered line: a long name wraps to
    /// two lines on the diagram (the legend and alt text do not).
    labels: Vec<RadarLabel>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct RadarLabel {
    x: String,
    y: String,
    /// `start` / `middle` / `end`: text always runs away from center.
    anchor: &'static str,
    text: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct RadarRing {
    r: String,
    /// The outer ring reads stronger: it is the pool's ceiling.
    op: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct RadarSpoke {
    x: String,
    y: String,
}

#[derive(Debug, Clone, serde::Serialize)]
struct RadarDot {
    cx: String,
    cy: String,
}

/// Radar geometry. The canvas is *not* fixed: it is fitted to the
/// geometry plus the label extents, so a longer axis name widens the
/// diagram instead of running off the edge. The fitted size becomes
/// the SVG's intrinsic width/height, so a bare embed renders at the
/// design size rather than the browser's 300×150 default.
const RADAR_R: f64 = 42.0;
/// Label size in viewBox units; `width` in radar.toml scales the whole
/// diagram, so this is what a label costs when the config zooms.
const RADAR_FONT: f64 = 10.0;
const RADAR_LINE_H: f64 = 12.0;
/// Breathing room between a vertex and its label, and padding around
/// the fitted box.
const RADAR_GAP: f64 = 6.0;
const RADAR_PAD: f64 = 4.0;
const RADAR_DOT: f64 = 2.5;

/// Band name to the shared palette token: the shape wears the same
/// color as the heat badge printed on the same row.
fn band_color(band: &str) -> String {
    match band {
        "heating" => palette::heating(),
        "steady" => palette::steady(),
        "cooling" => palette::cooling(),
        _ => palette::dormant(),
    }
}

/// Label width at the radar's font: half an em per glyph — the same
/// estimation `est_w` uses for the other charts, scaled to this size.
fn label_w(s: &str) -> f64 {
    s.chars().count() as f64 * RADAR_FONT * 0.5
}

/// Split an axis label for the drawing: one line when it fits the
/// label budget, two at the space nearest the middle when it doesn't.
/// Only the diagram wraps — the legend sentence and the alt text keep
/// the full wording, so nothing a reader parses is abbreviated.
fn label_lines(label: &str) -> Vec<String> {
    const BUDGET: usize = 9;
    if label.chars().count() <= BUDGET {
        return vec![label.to_string()];
    }
    let words: Vec<&str> = label.split_whitespace().collect();
    if words.len() < 2 {
        return vec![label.to_string()];
    }
    // Split where the two lines come out most even.
    let total = label.chars().count();
    let mut best = 1;
    let mut best_gap = usize::MAX;
    for i in 1..words.len() {
        let head: usize = words[..i].iter().map(|w| w.chars().count() + 1).sum();
        let gap = (2 * head).abs_diff(total);
        if gap < best_gap {
            best_gap = gap;
            best = i;
        }
    }
    vec![words[..best].join(" "), words[best..].join(" ")]
}

/// One repo's radar geometry: config, graph and node in, a canvas
/// fitted to geometry plus labels out. The work happens in a local
/// frame (center at the origin, -90° up, clockwise); the fit finds the
/// bounding box and translates everything onto the canvas — so a longer
/// axis name widens the diagram instead of running off the edge.
fn radar_doc(
    graph: &jzombie_graph::graph::Graph,
    node: &jzombie_graph::graph::Node,
    plan: &RadarPlan,
) -> RadarDoc {
    let pcts = plan.percentiles(graph, node);
    let n = plan.axes.len();
    let dirs: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let a = (-90.0 + 360.0 * i as f64 / n as f64).to_radians();
            (a.cos(), a.sin())
        })
        .collect();
    let reach = |p: f64| RADAR_R * p.clamp(0.0, 1.0);
    let local = |i: usize, r: f64| (r * dirs[i].0, r * dirs[i].1);

    // The drawing wraps long names; the legend and alt text do not, so
    // nothing a reader parses as prose is ever abbreviated.
    let wrapped: Vec<Vec<String>> = plan
        .axes
        .iter()
        .map(|(label, _)| label_lines(label))
        .collect();

    // Fit: geometry first, then every label at its anchor. Labels are
    // anchored away from the center, so none can cross the polygon.
    let mut left = -RADAR_R - RADAR_DOT;
    let mut right = RADAR_R + RADAR_DOT;
    let mut top = -RADAR_R - RADAR_DOT;
    let mut bottom = RADAR_R + RADAR_DOT;
    // Per axis: label anchor x, first baseline, and text anchor.
    let mut placed: Vec<(f64, f64, &'static str)> = Vec::with_capacity(n);
    for (i, lines) in wrapped.iter().enumerate() {
        let (dx, dy) = dirs[i];
        let ax = (RADAR_R + RADAR_GAP) * dx;
        let ay = (RADAR_R + RADAR_GAP) * dy;
        let anchor = if dx > 0.1 {
            "start"
        } else if dx < -0.1 {
            "end"
        } else {
            "middle"
        };
        let w = lines.iter().map(|l| label_w(l)).fold(0.0_f64, f64::max);
        let (x0, x1) = match anchor {
            "start" => (ax, ax + w),
            "end" => (ax - w, ax),
            _ => (ax - w / 2.0, ax + w / 2.0),
        };
        // Where the first line's baseline goes. The discriminator is
        // the anchor, not the sign of dy: a side label sits at ±0.5 on
        // that axis and must stay optically centered on its spoke,
        // while only the true top/bottom labels stack above or below.
        let extra = RADAR_LINE_H * (lines.len() - 1) as f64;
        let b0 = if anchor == "middle" && dy < 0.0 {
            ay - RADAR_FONT * 0.25 - extra
        } else if anchor == "middle" {
            ay + RADAR_FONT * 0.75
        } else {
            ay + RADAR_FONT * 0.35 - extra / 2.0
        };
        left = left.min(x0);
        right = right.max(x1);
        top = top.min(b0 - RADAR_FONT * 0.75);
        bottom = bottom.max(b0 + RADAR_FONT * 0.25 + extra);
        placed.push((ax, b0, anchor));
    }
    // Translate the local frame onto the canvas: padding off the left
    // and top edges, and the center lands wherever the fit put it.
    let ox = RADAR_PAD - left;
    let oy = RADAR_PAD - top;
    let vw = right - left + 2.0 * RADAR_PAD;
    let vh = bottom - top + 2.0 * RADAR_PAD;

    let points = pcts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (x, y) = local(i, reach(*p));
            format!("{},{}", f1(x + ox), f1(y + oy))
        })
        .collect::<Vec<_>>()
        .join(" ");
    let spokes = (0..n)
        .map(|i| {
            let (x, y) = local(i, RADAR_R);
            RadarSpoke {
                x: f1(x + ox),
                y: f1(y + oy),
            }
        })
        .collect();
    // Quartile rings: the half ring doubles as the pool's median, so a
    // vertex sitting on it is an exactly average repo on that axis.
    let rings = [0.25, 0.5, 0.75, 1.0]
        .into_iter()
        .map(|q| RadarRing {
            r: f1(RADAR_R * q),
            op: if q == 1.0 { "0.7" } else { "0.4" }.to_string(),
        })
        .collect();
    let dots = pcts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let (x, y) = local(i, reach(*p));
            RadarDot {
                cx: f1(x + ox),
                cy: f1(y + oy),
            }
        })
        .collect();
    let mut labels = Vec::new();
    for (i, lines) in wrapped.iter().enumerate() {
        let (ax, b0, anchor) = placed[i];
        for (j, text) in lines.iter().enumerate() {
            labels.push(RadarLabel {
                x: f1(ax + ox),
                y: f1(b0 + RADAR_LINE_H * j as f64 + oy),
                anchor,
                text: text.clone(),
            });
        }
    }
    let heat = node.commit_heat.or_else(|| node.commit_heat());
    RadarDoc {
        header: format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" role=\"img\" font-family=\"{family}\"><title>{title}</title>",
            w = f1(vw),
            h = f1(vh),
            family = palette::family(),
            title = radar_readout(plan, &pcts),
        ),
        footer: "</svg>".to_string(),
        cx: f1(ox),
        cy: f1(oy),
        font: f1(RADAR_FONT),
        dot: f1(RADAR_DOT),
        ink: palette::ink(),
        color: band_color(heat_band(heat)),
        pulse: palette::heat_pulse(heat).to_string(),
        points,
        rings,
        spokes,
        dots,
        labels,
    }
}

/// Plugin function: graph JSON + repo id in, radar geometry document
/// out. Pure data shaping (no IO); base64 transport on entry, like the
/// other chart funcs — the script never sees raw JSON.
#[oxdock_func(pure, name = "RADAR")]
fn radar(graph_json: String, repo_id: String) -> anyhow::Result<Value> {
    let graph_json = transport_out(&graph_json)?;
    let repo_id = transport_out(&repo_id)?;
    let graph = jzombie_graph::graph::Graph::from_json(&graph_json)?;
    let node = graph
        .nodes
        .iter()
        .find(|n| n.id == repo_id)
        .ok_or_else(|| anyhow::anyhow!("no node {repo_id} in the graph"))?;
    let plan = RadarPlan::load()?;
    let doc = radar_doc(&graph, node, &plan);
    Ok(Value::string(serde_json::to_string(&doc)?))
}

/// Render one repo's radar: graph JSON + id in, SVG bytes out. The
/// embedded script emits markup only; every number arrives in the
/// geometry document the host func returns.
pub fn render_radar_svg(graph_json: &str, repo_id: &str) -> anyhow::Result<String> {
    let engine = engine();
    let arg = transport_in(graph_json);
    let id_arg = transport_in(repo_id);
    let steps: Vec<oxdock_parser::Step> = oxdock! {
        modules: [CHARTS],
        IMPORT [STD, CHARTS]
        LET $doc_json: STRING = CHARTS::RADAR(#arg, #id_arg)
        WRITE doc.json $doc_json
        LET $doc: MAP<ANY> = LOAD_JSON("doc.json")
        LET $style: STRING = CHARTS::CHART_STYLE()
        WRITE out.svg $doc.header
        APPEND out.svg $style
        FOR $ring: MAP<ANY> IN $doc.rings {
            APPEND out.svg "<circle cx=\"{{ $doc.cx }}\" cy=\"{{ $doc.cy }}\" r=\"{{ $ring.r }}\" fill=\"none\" stroke=\"{{ $doc.ink }}\" stroke-width=\"1\" opacity=\"{{ $ring.op }}\"/>"
        }
        FOR $spoke: MAP<ANY> IN $doc.spokes {
            APPEND out.svg "<line x1=\"{{ $doc.cx }}\" y1=\"{{ $doc.cy }}\" x2=\"{{ $spoke.x }}\" y2=\"{{ $spoke.y }}\" stroke=\"{{ $doc.ink }}\" stroke-width=\"1\" opacity=\"0.5\"/>"
        }
        APPEND out.svg "<polygon points=\"{{ $doc.points }}\" fill=\"{{ $doc.color }}\" fill-opacity=\"0.32\" stroke=\"{{ $doc.color }}\" stroke-width=\"2\" stroke-linejoin=\"round\" class=\"{{ $doc.pulse }}\"/>"
        FOR $dot: MAP<ANY> IN $doc.dots {
            APPEND out.svg "<circle cx=\"{{ $dot.cx }}\" cy=\"{{ $dot.cy }}\" r=\"{{ $doc.dot }}\" fill=\"{{ $doc.color }}\"/>"
        }
        FOR $label: MAP<ANY> IN $doc.labels {
            APPEND out.svg "<text x=\"{{ $label.x }}\" y=\"{{ $label.y }}\" text-anchor=\"{{ $label.anchor }}\" font-size=\"{{ $doc.font }}\" fill=\"{{ $doc.ink }}\">{{ $label.text }}</text>"
        }
        APPEND out.svg $doc.footer
        LET $check: STRING = READ out.svg
        ASSERT_CONTAINS $check "</svg>"
        ASSERT_CONTAINS $check "<polygon"
        ASSERT_CONTAINS $check "<text"
    };
    let temp = oxdock::GuardedPath::tempdir().expect("tempdir");
    let root = temp.as_guarded_path().clone();
    engine.run_steps_without_snapshot(&root, &steps)?;
    let resolver = oxdock::PathResolver::new(root.as_path(), root.as_path()).expect("resolver");
    let out = root.join("out.svg").expect("out path");
    Ok(resolver.read_to_string(&out).expect("read out"))
}

/// The radar family: one (file stem, SVG) pair per selected repo — the
/// same rows the fragment lists, so an embedded image can never point
/// at a file the lane didn't write.
pub fn render_radars(graph_json: &str) -> anyhow::Result<Vec<(String, String)>> {
    let graph = jzombie_graph::graph::Graph::from_json(graph_json)?;
    let rows = selected_rows(&graph);
    rows.iter()
        .map(|row| {
            let svg = render_radar_svg(graph_json, &row.id)?;
            Ok((jzombie_graph::paths::radar_stem(&row.id), svg))
        })
        .collect()
}

/// The floated radar on a row's head line: sized by config, aligned
/// with the legacy `align` attribute (the only alignment GitHub's
/// sanitizer keeps — `style` is stripped), sweep-stamped like every
/// other image, and anchored to its own repo — a bare `<img>` gets
/// auto-linked to the raw SVG bytes instead, which is a click nobody
/// wants. The alt text spells the percentiles out, so a screen reader
/// gets the same facts the vertices show. Floated (not tabled) so the
/// image keeps its width on narrow screens instead of shrinking with
/// its column.
fn radar_img(
    plan: &RadarPlan,
    graph: &jzombie_graph::graph::Graph,
    row: &SelectedRow,
    bust: &str,
) -> anyhow::Result<String> {
    let node = graph
        .nodes
        .iter()
        .find(|n| n.id == row.id)
        .ok_or_else(|| anyhow::anyhow!("selected row {} missing from the graph", row.id))?;
    let pcts = plan.percentiles(graph, node);
    Ok(format!(
        "<a href=\"{url}\"><img src=\"{src}\" width=\"{width}\" align=\"right\" alt=\"{alt}\"></a>",
        url = row.url,
        src = jzombie_graph::paths::radar_url(&jzombie_graph::paths::radar_stem(&row.id), bust),
        width = plan.cfg.width,
        alt = radar_readout(plan, &pcts),
    ))
}

/// Linguist language colors for a language dot; unknown maps to the
/// same `#333` fallback the reference uses.
fn lang_color(lang: &str) -> &'static str {
    match lang {
        "Rust" => "#dea584",
        // Lightened from linguist #3178c6: Python's #3572A5 sits next to
        // it at similar shares, and the two blues were indistinguishable.
        "TypeScript" => "#4FA3D1",
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

/// Estimated 12px Segoe UI advance, for footer item spacing. The
/// reference measures real glyphs; estimation keeps Rust side-only.
fn est_w(s: &str) -> f64 {
    s.chars().count() as f64 * 6.6
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
            Radar::registration(),
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

/// Section fragments with cache-busted image URLs. Same bust token as the
/// the sweep timestamp, so GitHub's image proxy refetches after every
/// nightly rebuild instead of serving the stale bytes.
pub fn render_flame_md(graph_json: &str) -> anyhow::Result<String> {
    let bust = bust_token(graph_json)?;
    Ok(formatdoc! {
        r#"
        [![flame]({})]({})
        "#,
        jzombie_graph::paths::chart_url(jzombie_graph::paths::FLAME_FILE, &bust),
        jzombie_graph::paths::REPOS_URL,
    })
}

pub fn render_share_md(graph_json: &str) -> anyhow::Result<String> {
    let bust = bust_token(graph_json)?;
    Ok(formatdoc! {
        r#"
        ## Commit share

        Each slice is a repo's share of every commit pushed in the trailing five days, across my own public repos — a fork only counts once it has diverged. Repos with nothing in that window drop out; the busiest are named and the rest roll into Other.

        [![share]({})]({})
        "#,
        jzombie_graph::paths::chart_url(jzombie_graph::paths::SHARE_FILE, &bust),
        jzombie_graph::paths::REPOS_URL,
    })
}

pub fn render_focus_md(graph_json: &str) -> anyhow::Result<String> {
    let bust = bust_token(graph_json)?;
    Ok(formatdoc! {
        r#"
        ## Developer focus

        Developer focus is a heuristic of what's being planned and developed right now. Scores blend recent commit and issue-comment activity against a trailing 30-day baseline, then discount idle months. A signal with no baseline sits out.

        It's a relative measure, not a tally of volume: each repo is scored against its own trailing-month pace, so heavy output that is simply normal for that project reads as steady. What climbs is a repo doing more than it usually does; what pulls it back down is having gone quiet.

        [![developer focus]({})]({})
        "#,
        jzombie_graph::paths::chart_url(jzombie_graph::paths::FOCUS_FILE, &bust),
        jzombie_graph::paths::REPOS_URL,
    })
}

/// Top languages strip: repo counts by detected language, own-work
/// non-private repos only. Unknown-language repos stay out entirely —
/// no "Unspecified" slot, same rule as the pin footers. Top-6 plus an
/// Other rollup when the tail is non-empty.
const LANG_TOPN: usize = 6;
/// Misc bucket gray: the shared secondary gray, not a language color.
const LANG_OTHER: &str = "#9198a1";
const LANG_W: f64 = 800.0;
const LANG_H: f64 = 56.0;
const LANG_X0: f64 = 16.0;
const LANG_TRACK: f64 = 768.0;

/// Language rows: `(language, repos, share)` sorted by repos desc, name
/// asc; the tail past `LANG_TOPN` rolls into `Other`.
fn language_shares(graph_json: &str) -> anyhow::Result<Vec<(String, u32, f32)>> {
    let graph = jzombie_graph::graph::Graph::from_json(graph_json)?;
    let mut counts: std::collections::BTreeMap<&str, u32> = std::collections::BTreeMap::new();
    for n in graph
        .nodes
        .iter()
        .filter(|nd| !nd.private && jzombie_graph::graph::is_own_work(nd))
    {
        if let Some(lang) = n.language.as_deref() {
            *counts.entry(lang).or_default() += 1;
        }
    }
    let mut ranked: Vec<(&str, u32)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let total: u32 = ranked.iter().map(|(_, c)| c).sum();
    if total == 0 {
        return Ok(vec![]);
    }
    let mut rows: Vec<(String, u32, f32)> = ranked
        .iter()
        .take(LANG_TOPN)
        .map(|(l, c)| ((*l).to_string(), *c, *c as f32 / total as f32))
        .collect();
    let rest: u32 = ranked.iter().skip(LANG_TOPN).map(|(_, c)| c).sum();
    if rest > 0 {
        rows.push(("Other".to_string(), rest, rest as f32 / total as f32));
    }
    Ok(rows)
}

/// Render the top-languages strip: one rounded bar, segments pro-rata
/// repo counts, legend with shares. Pure-Rust markup (no engine round
/// trip — the layout is arithmetic, fully unit-testable).
pub fn render_languages_svg(graph_json: &str) -> anyhow::Result<String> {
    let rows = language_shares(graph_json)?;
    let family = palette::family();
    if rows.is_empty() {
        return Ok(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {W} {H}\" role=\"img\" font-family=\"{family}\"><title>Top languages</title><text x=\"16\" y=\"34\" font-size=\"14\" fill=\"{muted}\">No language data in scope</text></svg>",
            W = f1(LANG_W),
            H = f1(LANG_H),
            muted = palette::muted(),
        ));
    }
    let mut x = LANG_X0;
    let mut segs = String::new();
    for (lang, _, share) in &rows {
        let w = *share as f64 * LANG_TRACK;
        let color = if lang == "Other" {
            LANG_OTHER.to_string()
        } else {
            lang_color(lang).to_string()
        };
        segs.push_str(&format!(
            "<rect x=\"{}\" y=\"16\" width=\"{}\" height=\"12\" fill=\"{color}\"/>",
            f1(x),
            f1(w),
        ));
        x += w;
    }
    let mut lx = LANG_X0;
    let mut legend = String::new();
    for (lang, _, share) in &rows {
        let label = format!("{lang} {:.1}%", share * 100.0);
        let color = if lang == "Other" {
            LANG_OTHER.to_string()
        } else {
            lang_color(lang).to_string()
        };
        legend.push_str(&format!(
            "<circle cx=\"{}\" cy=\"40\" r=\"5\" fill=\"{color}\"/><text x=\"{}\" y=\"44\" font-size=\"11\" fill=\"#9198a1\">{label}</text>",
            f1(lx + 5.0),
            f1(lx + 14.0),
        ));
        lx += 14.0 + est_w(&label) + 18.0;
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {W} {H}\" role=\"img\" font-family=\"{family}\"><title>Top languages</title><clipPath id=\"langclip\"><rect x=\"{X}\" y=\"16\" width=\"{T}\" height=\"12\" rx=\"6\"/></clipPath><g clip-path=\"url(#langclip)\">{segs}</g>{legend}</svg>",
        W = f1(LANG_W),
        H = f1(LANG_H),
        X = f1(LANG_X0),
        T = f1(LANG_TRACK),
    ))
}

pub fn render_languages_md(graph_json: &str) -> anyhow::Result<String> {
    let bust = bust_token(graph_json)?;
    Ok(formatdoc! {
        r#"
        ## Top languages

        [![languages]({})]({})

        <sub>Share of portfolio repos by detected language; repos without one stay out.</sub>
        "#,
        jzombie_graph::paths::chart_url(jzombie_graph::paths::LANGUAGES_FILE, &bust),
        jzombie_graph::paths::REPOS_URL,
    })
}

/// Footer fragment: credit plus the sweep timestamp, so the bottom of
/// the page always names when the dashboard was last rebuilt.
pub fn render_footer_md(graph_json: &str) -> anyhow::Result<String> {
    let graph = jzombie_graph::graph::Graph::from_json(graph_json)?;
    Ok(formatdoc! {
        r#"
        <sub>Charts rendered with [OxDock](https://github.com/jzombie/rust-oxdock); data generated by [jzombie-graph](https://github.com/jzombie/jzombie-graph) · updated {}.</sub>
        "#,
        graph.generated_at.format("%Y-%m-%dT%H:%MZ"),
    })
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
                file: jzombie_graph::paths::indicator_url(name),
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
    // Explicit width/height, not just a viewBox: a viewBox alone gives
    // the image no intrinsic size, so every renderer falls back to its
    // own default — browsers ratio-fit into 300x150, which turned a
    // 44px badge into a 150px block inside a text line. Pinning the
    // design size in the file makes it render the same everywhere,
    // including opened directly in a tab.
    let w = f1(doc.rows.len() as f64 * INDICATOR_CELL);
    let h = f1(INDICATOR_CELL);
    doc.header = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" role=\"img\" font-family=\"{family}\"><title>Status indicators</title>",
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
    format!(
        "<div align=\"center\">![{band}]({})<br><sub>{band}</sub></div>",
        jzombie_graph::paths::indicator_url(band)
    )
}

/// One `#topic` link to a user-scoped repo search. Owner rides the node —
/// never a baked-in login.
fn topic_link(owner: &str, topic: &str) -> String {
    let enc: String = topic
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
        "[#{topic}](https://github.com/search?q=user%3A{owner}+topic%3A{enc}&type=repositories)"
    )
}

/// Topic links for an about cell: tiny `#tag` links to a user-scoped repo
/// search, capped so one repo can't flood the table.
fn topic_links(owner: &str, topics: &[String]) -> String {
    const MAX: usize = 4;
    let mut tags: Vec<String> = topics
        .iter()
        .take(MAX)
        .map(|t| topic_link(owner, t))
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
        "Updated {} UTC — {} repos, {} commits in the last 30 days across all branches; a commit on more than one branch counts once.",
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
    fn selected_score_ranks_reuse_over_popularity() {
        // One portfolio dependent outweighs a crowd of stars: a repo the
        // portfolio leans on beats a popular repo nobody imports.
        assert!(
            selected_score(3, 0, 0, false, true, 1, false)
                > selected_score(0, 30, 0, false, true, 1, false)
        );
        // Product-ness and activity nudge a lead, never flip it.
        assert!(
            selected_score(2, 0, 0, true, true, 50, false)
                > selected_score(1, 5, 0, false, false, 0, false)
        );
        // A diverged fork pays exactly the fork penalty, all else equal.
        assert!(
            selected_score(1, 5, 1, false, true, 1, false)
                > selected_score(1, 5, 1, false, true, 1, true)
        );
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
                "<div align=\"center\">![steady](dashboard/indicators/steady.svg)<br><sub>steady</sub></div>"
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
        // Intrinsic size pinned: without it a renderer falls back to its
        // own default (browsers: 300x150 ratio-fit → a 150px block).
        assert!(
            one.contains("width=\"44.0\" height=\"44.0\""),
            "design size on the root:\n{one}"
        );
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
        let up = rows.iter().find(|r| r.label == "up").unwrap();
        let down = rows.iter().find(|r| r.label == "down").unwrap();
        // Positive rows read left of the axis: names end at 130,
        // values end at 200 — text over the page stays gray.
        let up_tx: f64 = up.tx.parse().unwrap();
        let up_vx: f64 = up.vx.parse().unwrap();
        assert!((up_tx - 130.0).abs() < 1e-6);
        assert!((up_vx - 200.0).abs() < 1e-6);
        assert_eq!(up.na, "end");
        assert_eq!(up.va, "end");
        // Negative rows read right of the axis, clear of bars that
        // extend left into the label columns.
        let down_tx: f64 = down.tx.parse().unwrap();
        let down_vx: f64 = down.vx.parse().unwrap();
        assert!((down_tx - 268.0).abs() < 1e-6);
        assert!((down_vx - 460.0).abs() < 1e-6);
        assert_eq!(down.na, "start");
        assert_eq!(down.va, "end");
        // Positive starts at zero and extends right; negative ends at zero.
        let up_bx: f64 = up.bx.parse().unwrap();
        let down_bx: f64 = down.bx.parse().unwrap();
        let down_bw: f64 = down.bw.parse().unwrap();
        assert!((up_bx - 260.0).abs() < 1e-6);
        assert!((down_bx + down_bw - 260.0).abs() < 1e-6);
        let up_bw: f64 = up.bw.parse().unwrap();
        // Each side fills its own half against its own max: both bars span
        // the full half-width despite |−4| ≠ |2|.
        assert!((up_bw - 200.0).abs() < 1e-6);
        assert!((down_bw - 200.0).abs() < 1e-6);
        // Long names truncate; short ones pass through.
        assert_eq!(truncate_name("short", LABEL_MAX), "short");
        assert!(truncate_name(&"x".repeat(40), LABEL_MAX).ends_with('…'));
    }

    #[test]
    fn focus_labels_sit_on_white_by_sign() {
        // One ink pass, no clip paths: positive labels live left of the
        // axis, negative labels right of it — never on a bar, so every
        // renderer draws the same pixels.
        let g = graph_with_focus(&[
            ("top", Some(10.0)),
            ("mid", Some(-2.0)),
            ("low", Some(-10.0)),
        ]);
        let rows = focus_rows(&g, 12, 5).unwrap();
        for r in &rows {
            let tx: f64 = r.tx.parse().unwrap();
            let bx: f64 = r.bx.parse().unwrap();
            let bw: f64 = r.bw.parse().unwrap();
            if r.label == "top" {
                assert!(tx < 260.0, "positive name left of axis");
                assert!(bx >= 260.0, "positive bar starts at axis");
            } else {
                assert!(tx > 260.0, "negative name right of axis");
                assert!(bx + bw <= 260.0 + 1e-6, "negative bar ends at axis");
            }
        }
        let svg = render_focus_svg(&g).unwrap();
        assert!(!svg.contains("<clipPath"), "no clip paths");
        assert!(svg.contains(palette::ink().as_str()));
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

    // --- Shape radar -------------------------------------------------

    /// A plan over exactly the axes a test names, with a pool that
    /// admits every node: geometry tests must not depend on which axes
    /// `radar.toml` happens to declare today.
    fn plan_over(axes: Vec<(&str, Metric)>) -> RadarPlan {
        RadarPlan {
            cfg: RadarConfig {
                width: 120,
                pool: RadarPool {
                    label: "test peers".into(),
                    own_work: false,
                    min_commits_30d: Some(1),
                    exclude_profile: false,
                },
                axes: vec![],
            },
            axes: axes
                .into_iter()
                .map(|(label, metric)| (label.to_string(), metric))
                .collect(),
        }
    }

    /// Mean vertex radius of a rendered doc — how far out the shape
    /// sits, which is what a percentile decides. The canvas is fitted,
    /// so the center comes from the doc rather than a constant.
    fn mean_radius(doc: &RadarDoc) -> f64 {
        let cx: f64 = doc.cx.parse().expect("cx");
        let cy: f64 = doc.cy.parse().expect("cy");
        let radii: Vec<f64> = doc
            .points
            .split(' ')
            .map(|pair| {
                let (x, y) = pair.split_once(',').expect("x,y pair");
                let (x, y): (f64, f64) = (x.parse().expect("x"), y.parse().expect("y"));
                ((x - cx).powi(2) + (y - cy).powi(2)).sqrt()
            })
            .collect();
        radii.iter().sum::<f64>() / radii.len() as f64
    }

    #[test]
    fn percentile_shares_ties_and_needs_no_special_casing() {
        // Ties take the middle of their run: three repos at zero stars
        // all read the same rank instead of all reading the floor.
        assert_eq!(percentile(&[0.0, 0.0, 0.0], 0.0), 0.5);
        // Negatives and outliers ride the same scale as anything else —
        // focus runs to -109 in the real graph and needs no handling.
        let pool = [-10.0, -1.0, 0.0, 5.0];
        assert_eq!(percentile(&pool, -10.0), 0.125);
        assert_eq!(percentile(&pool, 5.0), 0.875);
        // An empty pool degrades to the floor, never to NaN.
        assert_eq!(percentile(&[], 3.0), 0.0);
    }

    #[test]
    fn ordinal_reads_as_prose() {
        for (n, want) in [
            (1, "1st"),
            (2, "2nd"),
            (3, "3rd"),
            (4, "4th"),
            (11, "11th"),
            (12, "12th"),
            (13, "13th"),
            (21, "21st"),
            (100, "100th"),
        ] {
            assert_eq!(ordinal(n), want, "{n}");
        }
    }

    #[test]
    fn vertices_run_outward_with_percentile() {
        // Two repos, one axis: the leader's rank (0.75 in a two-repo
        // pool) must land farther from center than the trailer's (0.25)
        // — the mapping from percentile to radius.
        let g = graph_with(&[("strong", 30), ("weak", 10)]);
        let graph = jzombie_graph::graph::Graph::from_json(&g).expect("fixture parses");
        let plan = plan_over(vec![("Commits", |n, _| n.commits_30d as f64)]);
        let radius = |name: &str| {
            let node = graph
                .nodes
                .iter()
                .find(|n| n.name == name)
                .unwrap_or_else(|| panic!("{name} in fixture"));
            mean_radius(&radar_doc(&graph, node, &plan))
        };
        let strong = radius("strong");
        let weak = radius("weak");
        assert!(strong > weak, "strong {strong} must outrank weak {weak}");
        // Mid-rank percentiles stay inside the rim: a leader is never
        // drawn at the pool's ceiling, a trailer never collapses to a
        // point — both keep a readable shape.
        assert!(strong < RADAR_R, "leader stays inside the outer ring");
        assert!(weak > 0.0, "trailer keeps a visible shape");
    }

    #[test]
    fn radar_pool_admits_only_the_configured_peers() {
        let g = mixed_ownership_graph();
        let mut graph = jzombie_graph::graph::Graph::from_json(&g).expect("fixture parses");
        // Two more nodes the filters must reject: the profile README
        // repo, and a repo with nothing in the trailing 30d.
        let mut profile = graph.nodes[0].clone();
        profile.id = "me/me".into();
        profile.name = "me".into();
        graph.nodes.push(profile);
        let mut quiet = graph.nodes[1].clone();
        quiet.id = "me/quiet".into();
        quiet.name = "quiet".into();
        quiet.commits_30d = 0;
        graph.nodes.push(quiet);
        let cfg = RadarConfig {
            width: 120,
            pool: RadarPool {
                label: "peers".into(),
                own_work: true,
                min_commits_30d: Some(1),
                exclude_profile: true,
            },
            axes: vec![],
        };
        let names: Vec<&str> = radar_pool(&graph, &cfg)
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        // `pristine` is someone else's code, `secret` is private, the
        // profile repo documents rather than works, `quiet` sat still.
        // The graph stores nodes sorted by id, so the survivors read
        // in id order rather than insertion order.
        assert_eq!(names, vec!["diverged", "mine", "renamed"]);
    }

    #[test]
    fn shipped_radar_config_loads_and_resolves() {
        let plan = RadarPlan::load().expect("radar.toml parses; every metric exists");
        assert!(!plan.axes.is_empty(), "axes are declared");
        assert!(
            !plan.pool_label().trim().is_empty(),
            "the peer group is named"
        );
        for (label, _) in &plan.axes {
            assert!(!label.trim().is_empty(), "axis labels read as words");
        }
    }

    #[test]
    fn an_unknown_metric_names_the_keys_that_exist() {
        let cfg = RadarConfig {
            width: 120,
            pool: RadarPool {
                label: "peers".into(),
                own_work: false,
                min_commits_30d: None,
                exclude_profile: false,
            },
            axes: vec![RadarAxisCfg {
                metric: "commits".into(),
                label: "Commits".into(),
            }],
        };
        let err = RadarPlan::resolve(cfg).expect_err("a typo must not render");
        let msg = format!("{err:#}");
        assert!(msg.contains("unknown metric \"commits\""), "{msg}");
        assert!(
            msg.contains("commits_30d"),
            "the error lists real keys: {msg}"
        );
    }
}
