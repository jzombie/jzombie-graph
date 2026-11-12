//! Published-asset paths: one source of truth for where generated files live.
//!
//! Layout rule (see plan: Publishing): directory names carry the type,
//! file names carry the graph's own name. Everything lives in the repo
//! and commits — no gitignored working copies, no separate publish step.
//! The nightly sweep regenerates these files and commits them back;
//! local runs write the same paths, so what you see is what ships.

/// The graph this repo publishes. Every related asset shares the stem.
pub const GRAPH_NAME: &str = "portfolio-graph";

/// Collector output: node/edge JSON. Committed — the nightly sweep
/// regenerates it and commits it back.
pub const GRAPH_JSON: &str = "json/portfolio-graph.json";

/// Schema lane output: the generated contract, paired beside the data
/// it validates. The drift test guards this file: whatever the types
/// derive is what ships.
pub const SCHEMA_JSON: &str = "json/portfolio-graph-schema.json";

/// Cartographer output: the verbatim fragment. Was `ARCHITECTURE.md`
/// at the repo root.
pub const FRAGMENT_MD: &str = "markdown/fragments/portfolio-graph.md";

/// Commit-data table fragment: the machine-readable section of the
/// proof page, computed from the fresh JSON. Rendered by the charts
/// preview lane, assembled by `oxdock-render` — never hand-edited.
pub const COMMIT_DATA_MD: &str = "markdown/fragments/commit-data.md";

/// Commit-share pie SVG.
pub const SHARE_SVG: &str = "svg/portfolio-graph-share.svg";

/// Developer Focus bars SVG.
pub const DEVELOPER_FOCUS_SVG: &str = "svg/portfolio-graph-developer-focus.svg";

/// Heat flame SVG: portfolio → repos → branches → embers, widths pro-rata
/// 30d share, color + motion from the heat bands.
pub const FLAME_SVG: &str = "svg/portfolio-graph-flame.svg";

/// Status-indicator strip preview SVG. The per-repo badges below are what
/// tables embed; the strip above is the look-check.
pub const INDICATORS_SVG: &str = "svg/portfolio-graph-indicators.svg";

/// One-off badge files, one per heat band (`<band>.svg`).
/// Table cells embed these; the strip above is the preview sheet.
pub const INDICATORS_DIR: &str = "svg/indicators";

/// Committed render proof: OxDock-templated table embedding the badges,
/// so github.com shows exactly what readers see. Regenerate, never edit.
pub const PREVIEW_INDICATORS_MD: &str = "preview/indicators.md";

/// Committed badge copies the preview table embeds. Same bytes as the
/// `svg/indicators` originals; copied at preview time, never hand-placed.
pub const PREVIEW_INDICATORS_DIR: &str = "preview/indicators";

/// Checked-in schema expectation the drift test guards. Same file the
/// schema lane writes — generation and expectation are one path.
pub const COMMITTED_SCHEMA: &str = "json/portfolio-graph-schema.json";
