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

/// Top-`n` repos by trailing-5d commits plus an `Other` bucket, with pie
/// geometry precomputed. Zero-commit repos never slice. Empty when nothing
/// committed in window (script emits a labeled empty canvas instead).
/// Colors come from the shared [`palette`] module — same values every chart.
fn share_rows(graph_json: &str, n: usize) -> anyhow::Result<Vec<SliceRow>> {
    let graph = jzombie_graph::graph::Graph::from_json(graph_json)?;
    let mut repos: Vec<(&str, u32)> = graph
        .nodes
        .iter()
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
        funcs: vec![PieSlices::registration()],
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

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_with(commits: &[(&str, u32)]) -> String {
        use jzombie_graph::graph::{Graph, Layer, Node};
        let nodes = commits
            .iter()
            .map(|(name, c)| Node {
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
                commits_5d: *c,
                commits_30d: *c * 6,
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
}
