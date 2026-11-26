//! End-to-end: fixture JSON in, the radar family out, and the fragment
//! floating exactly those files.
//!
//! Axes come from `radar.toml`, so nothing here pins the axis count or
//! the wording — the config is meant to be edited. What is pinned is
//! the contract: one vertex per configured axis, every embedded image
//! exists in the family, and the hover title and the alt text are the
//! same words (generated once, so they cannot drift apart).

use jzombie_graph::graph::{Edge, EdgeKind, Graph, Layer, Node, RecentPr};

/// One repo, distinct on every axis the shipped config reads: commits,
/// issue traffic, PRs, reuse (via the edges below), stars, focus.
fn repo(name: &str, stars: u32, focus: f32, commits: u32, prs: usize) -> Node {
    Node {
        id: format!("me/{name}"),
        owner: "me".into(),
        name: name.into(),
        fork: false,
        private: false,
        upstream: None,
        ahead_by: None,
        pushed_at: None,
        description: Some(format!("{name} does the work")),
        topics: vec![],
        language: Some("Rust".into()),
        homepage: None,
        stars,
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
        commits_30d: commits,
        issues_open_1d: 0,
        issues_open_5d: 0,
        issues_open_15d: 0,
        issues_open_30d: 0,
        issues_closed_1d: 0,
        issues_closed_5d: 0,
        issues_closed_15d: 0,
        issues_closed_30d: 0,
        issue_heat: None,
        issue_comments_1d: 0,
        issue_comments_5d: 0,
        issue_comments_15d: 0,
        issue_comments_30d: 0,
        issue_labels: Default::default(),
        recent_issues: vec![],
        commit_heat: None,
        focus: Some(focus),
        last_commit_at: None,
        branch_heat: vec![],
        recent_prs: (0..prs)
            .map(|i| RecentPr {
                title: format!("work {i}"),
                branch: "topic".into(),
                base: "main".into(),
                state: "MERGED".into(),
            })
            .collect(),
        visible: true,
        visible_reason: None,
        layer: std::collections::BTreeSet::from([Layer::Isolated]),
        has_default_binary: false,
        ci_usage_30d: Default::default(),
    }
}

/// Three repos ordered on everything the shipped config reads; two
/// dependence edges put `alpha` first on reuse as well. The sweep is
/// pinned so the fragment's bust token is deterministic.
fn fixture() -> Graph {
    let edge = |source: &str, target: &str| Edge {
        source: format!("me/{source}"),
        target: format!("me/{target}"),
        kind: EdgeKind::Manifest,
        declaring_branches: vec!["main".into()],
        recency: None,
        version: None,
    };
    let mut graph = Graph::new(
        vec![
            repo("alpha", 30, 3.0, 30, 3),
            repo("beta", 20, 0.0, 20, 2),
            repo("gamma", 10, -3.0, 10, 1),
        ],
        vec![edge("beta", "alpha"), edge("gamma", "alpha")],
    );
    graph.generated_at = "2026-11-14T23:03:00Z".parse().expect("fixed time");
    graph
}

fn graph_json() -> String {
    jzombie_graph::output::render_json(&fixture()).expect("fixture serializes")
}

/// Axis count out of the shipped config: tests follow the config
/// instead of pinning it, because axes are meant to be edited. Header
/// lines only — the file's own comments mention `[[axes]]`.
fn axis_count() -> usize {
    include_str!("../radar.toml")
        .lines()
        .filter(|line| line.starts_with("[[axes]]"))
        .count()
}

/// The width the fragment should ask GitHub for, read from the config
/// rather than assumed.
fn shipped_width() -> u32 {
    #[derive(serde::Deserialize)]
    struct Cfg {
        width: u32,
    }
    toml::from_str::<Cfg>(include_str!("../radar.toml"))
        .expect("radar.toml parses")
        .width
}

#[test]
fn radar_family_covers_exactly_the_fragment_rows() {
    let g = graph_json();
    let radars = charts::render_radars(&g).expect("family renders");
    let md = charts::render_selected_md(&g).expect("fragment renders");

    let rows = md.matches("- **[").count();
    assert_eq!(radars.len(), rows, "one radar per listed row");
    assert!(rows >= 2, "fixture compares repos, it is not a singleton");
    for (stem, svg) in &radars {
        assert!(
            md.contains(&format!("dashboard/charts/radar/{stem}.svg")),
            "fragment embeds {stem}"
        );
        assert!(
            svg.starts_with("<svg") && svg.ends_with("</svg>"),
            "{stem} is a well-formed document"
        );
    }
}

/// Axis labels out of the shipped config — the test follows the
/// config rather than pinning wording, because labels are meant to be
/// edited.
fn configured_labels() -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Cfg {
        axes: Vec<Axis>,
    }
    #[derive(serde::Deserialize)]
    struct Axis {
        label: String,
    }
    toml::from_str::<Cfg>(include_str!("../radar.toml"))
        .expect("radar.toml parses")
        .axes
        .into_iter()
        .map(|a| a.label)
        .collect()
}

#[test]
fn radar_draws_one_vertex_per_configured_axis_inside_the_fitted_canvas() {
    let g = graph_json();
    let axes = axis_count();
    assert!(axes >= 3, "config sanity: {axes} axes declared");
    let svg = charts::render_radar_svg(&g, "me/alpha").expect("alpha renders");

    let points = svg
        .split("points=\"")
        .nth(1)
        .expect("polygon points")
        .split('"')
        .next()
        .expect("points value");
    assert_eq!(points.split(' ').count(), axes, "one vertex per axis");
    assert_eq!(svg.matches("r=\"2.5\"").count(), axes, "one dot per vertex");

    // The canvas is fitted to geometry plus labels rather than being a
    // fixed box, so read the viewBox and check against that.
    let vb: Vec<f64> = svg
        .split("viewBox=\"")
        .nth(1)
        .expect("viewBox")
        .split('"')
        .next()
        .expect("viewBox value")
        .split_whitespace()
        .map(|s| s.parse().expect("number"))
        .collect();
    let (vx, vy, vw, vh) = (vb[0], vb[1], vb[2], vb[3]);
    assert!(vw > 0.0 && vh > 0.0, "canvas has area");
    for pair in points.split(' ') {
        let (x, y) = pair.split_once(',').expect("x,y pair");
        let (x, y): (f64, f64) = (x.parse().expect("x"), y.parse().expect("y"));
        assert!(
            (vx..=vx + vw).contains(&x) && (vy..=vy + vh).contains(&y),
            "vertex inside the fitted canvas: {pair}"
        );
    }
    // Intrinsic sizing equals the fitted canvas, so a bare embed
    // renders at the design size instead of the 300×150 default.
    assert!(
        svg.contains(&format!("width=\"{vw:.1}\" height=\"{vh:.1}\"")),
        "intrinsic size matches the viewBox:\n{svg}"
    );

    // Every configured axis is labeled on the diagram itself — matched
    // word by word, because a long name wraps onto two lines.
    for label in configured_labels() {
        for word in label.split_whitespace() {
            assert!(
                svg.contains(&format!(">{word}</text>")),
                "diagram draws {word:?} from label {label:?}"
            );
        }
    }
    insta::assert_snapshot!("radar_svg", svg);
}

#[test]
fn fragment_floats_the_radar_and_says_the_same_words_as_its_title() {
    let g = graph_json();
    let md = charts::render_selected_md(&g).expect("fragment renders");

    // Floated, config-sized, sweep-stamped: the legacy `align` is the
    // only alignment GitHub's sanitizer keeps (`style` is stripped),
    // and the float keeps its width on mobile where a table column
    // would surrender it.
    assert!(
        md.contains(&format!("width=\"{}\" align=\"right\"", shipped_width())),
        "radar floated on every row:\n{md}"
    );
    assert!(
        md.contains("?v=202611142303"),
        "radar URLs carry the sweep bust token"
    );
    // Each radar is a click target for its own repo: a bare <img>
    // would be auto-linked to the raw SVG bytes instead, which is a
    // click nobody wants.
    for name in ["alpha", "beta", "gamma"] {
        assert!(
            md.contains(&format!(
                "<a href=\"https://github.com/me/{name}\"><img src=\"dashboard/charts/radar/me-{name}.svg"
            )),
            "{name}'s radar links to its repo:\n{md}"
        );
    }
    // The legend states the dial order and the peer group, so a reader
    // can name a spoke without counting from the top.
    assert!(md.contains("Each row carries a shape radar"), "legend");
    assert!(md.contains("percentile among"), "peer group stated");

    // Hover title and screen-reader alt are one readout, generated
    // once per row — if these ever disagree, one of them is lying.
    for (stem, svg) in charts::render_radars(&g).expect("family renders") {
        let title = svg
            .split("<title>")
            .nth(1)
            .expect("svg title")
            .split("</title>")
            .next()
            .expect("title text");
        let at = md
            .find(&format!("radar/{stem}.svg"))
            .unwrap_or_else(|| panic!("{stem} embedded"));
        let end = at + md[at..].find('>').expect("img tag closes");
        let tag = &md[at..end];
        let alt = tag
            .split("alt=\"")
            .nth(1)
            .expect("alt attribute")
            .trim_end_matches('"');
        assert_eq!(alt, title, "{stem}: alt text and hover title disagree");
    }
}
