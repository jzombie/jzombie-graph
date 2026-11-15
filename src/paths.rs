//! Published-asset paths: one source of truth for where generated files live.
//!
//! Layout rule: everything the nightly sweep regenerates lives under
//! `proof/` and commits — no gitignored working copies, no separate
//! publish step, no duplicated trees. Local runs write the same paths,
//! so what you see is what ships.
//!
//! Splitting rule: directories and file names live once, as constants;
//! every full path below is composed from them by a helper. Moving a
//! directory or renaming a stem touches exactly one line — no base
//! directory is ever repeated inside a full path.

/// Single stem shared by every portfolio-graph asset.
pub const GRAPH_NAME: &str = "portfolio-graph";

/// Link target for chart strips: the owner's repo tab. Fragments embed
/// this instead of templating it, so generated markdown carries no
/// template syntax and every `.md` file is final.
pub const REPOS_URL: &str = "https://github.com/jzombie?tab=repositories";

/// Directories: the only place a location is spelled out.
pub const PROOF_DIR: &str = "proof";
pub const DATA_DIR: &str = "data";
/// Single vendored bundle: everything the profile sync copies lives
/// under here, so the sync wipes and replaces one directory — no
/// dangling artifacts, no sibling dirs to reason about.
pub const DASHBOARD_DIR: &str = "proof/dashboard";
pub const CHARTS_DIR: &str = "proof/dashboard/charts";
pub const FRAGMENTS_DIR: &str = "proof/dashboard/fragments";
pub const INDICATORS_DIR: &str = "proof/dashboard/indicators";

/// File names (no directories): the only place a file name is spelled out.
pub const GRAPH_FILE: &str = "portfolio-graph.json";
pub const SCHEMA_FILE: &str = "portfolio-graph-schema.json";
pub const MAP_FRAG_FILE: &str = "portfolio-graph.md";
pub const DATA_FRAG_FILE: &str = "commit-data.md";
pub const FOOTER_FRAG_FILE: &str = "footer.md";
pub const SHARE_FILE: &str = "portfolio-graph-share.svg";
pub const SHARE_FRAG_FILE: &str = "commit-share.md";
pub const FOCUS_FILE: &str = "portfolio-graph-developer-focus.svg";
pub const FOCUS_FRAG_FILE: &str = "developer-focus.md";
pub const LANGUAGES_FILE: &str = "portfolio-graph-languages.svg";
pub const LANGUAGES_FRAG_FILE: &str = "languages.md";
pub const FLAME_FILE: &str = "portfolio-graph-flame.svg";
pub const FLAME_FRAG_FILE: &str = "heat-flame.md";
pub const PINS_FRAG_FILE: &str = "pins.md";
pub const INDICATORS_FILE: &str = "portfolio-graph-indicators.svg";
pub const PROOF_README_FILE: &str = "README.md";

/// Collector output: node/edge JSON. Committed — the nightly sweep
/// regenerates it and commits it back.
pub fn graph_json() -> String {
    format!("{DATA_DIR}/{GRAPH_FILE}")
}

/// Schema lane output: the generated contract, paired beside the data
/// it validates. The drift test guards this file: whatever the types
/// derive is what ships.
pub fn schema_json() -> String {
    format!("{DATA_DIR}/{SCHEMA_FILE}")
}

/// Cartographer output: the verbatim fragment.
pub fn map_fragment() -> String {
    format!("{FRAGMENTS_DIR}/{MAP_FRAG_FILE}")
}

/// Commit-data table fragment: the data section of the proof page,
/// computed from the fresh JSON. Rendered by the charts lane,
/// assembled by `oxdock-render` — never hand-edited.
pub fn commit_data_fragment() -> String {
    format!("{FRAGMENTS_DIR}/{DATA_FRAG_FILE}")
}

/// Footer fragment: render credit plus the sweep timestamp, so the
/// bottom of the page always says when the dashboard was rebuilt.
pub fn footer_fragment() -> String {
    format!("{FRAGMENTS_DIR}/{FOOTER_FRAG_FILE}")
}

/// Commit-share pie SVG.
pub fn share_svg() -> String {
    format!("{CHARTS_DIR}/{SHARE_FILE}")
}

/// Commit-share section fragment (generated with cache-bust, like pins).
pub fn share_fragment() -> String {
    format!("{FRAGMENTS_DIR}/{SHARE_FRAG_FILE}")
}

/// Developer Focus bars SVG.
pub fn developer_focus_svg() -> String {
    format!("{CHARTS_DIR}/{FOCUS_FILE}")
}

/// Developer-focus section fragment (generated with cache-bust).
pub fn focus_fragment() -> String {
    format!("{FRAGMENTS_DIR}/{FOCUS_FRAG_FILE}")
}

/// Top-languages strip SVG: share of portfolio repos by language.
pub fn languages_svg() -> String {
    format!("{CHARTS_DIR}/{LANGUAGES_FILE}")
}

/// Top-languages section fragment (generated with cache-bust).
pub fn languages_fragment() -> String {
    format!("{FRAGMENTS_DIR}/{LANGUAGES_FRAG_FILE}")
}

/// Commit activity flame strip SVG (per-band badges live under
/// `proof/dashboard/indicators/`): portfolio → repos → branches → embers.
pub fn flame_svg() -> String {
    format!("{CHARTS_DIR}/{FLAME_FILE}")
}

/// Flame strip section fragment (generated with cache-bust).
pub fn flame_fragment() -> String {
    format!("{FRAGMENTS_DIR}/{FLAME_FRAG_FILE}")
}

/// Popular repositories fragment: the flowing paragraph embedding the
/// pin cards, regenerated with the family — never hand-edited.
pub fn pins_fragment() -> String {
    format!("{FRAGMENTS_DIR}/{PINS_FRAG_FILE}")
}

/// Status-indicator strip SVG. The per-repo badges below are what
/// tables embed; the strip above is the look-check.
pub fn indicators_svg() -> String {
    format!("{CHARTS_DIR}/{INDICATORS_FILE}")
}

/// One-off badge file for a heat band. Table cells embed these; the
/// strip above is the preview sheet.
pub fn indicator_badge(name: &str) -> String {
    format!("{INDICATORS_DIR}/{name}.svg")
}

/// Assembled proof page. `oxdock-render` owns it — never hand-edited.
pub fn proof_readme() -> String {
    format!("{PROOF_DIR}/{PROOF_README_FILE}")
}

/// Pinned repo card file, slugged by repo name.
/// One SVG per repo, each linking to its own repo.
pub fn pin_file_name(slug: &str) -> String {
    format!("{GRAPH_NAME}-pin-{slug}.svg")
}

/// Committed path for a pinned repo card.
pub fn pin_svg_path(slug: &str) -> String {
    format!("{CHARTS_DIR}/{}", pin_file_name(slug))
}

/// Relative chart URL for assembled markdown (`proof/README.md` sits in
/// `proof/`, so the prefix is the charts dir without it). Every rendered
/// asset URL carries the sweep it came from: GitHub's image proxy caches
/// by URL, so a fresh `?v=` per nightly is what refetches.
pub fn chart_url(file: &str, bust: &str) -> String {
    let prefix = CHARTS_DIR.strip_prefix("proof/").unwrap_or(CHARTS_DIR);
    format!("{prefix}/{file}?v={bust}")
}

/// Relative badge URL for assembled markdown: same `proof/`-relative
/// rule as charts, so the vendored profile README resolves it under
/// its own `dashboard/` copy.
pub fn indicator_url(name: &str) -> String {
    let prefix = INDICATORS_DIR
        .strip_prefix("proof/")
        .unwrap_or(INDICATORS_DIR);
    format!("{prefix}/{name}.svg")
}
