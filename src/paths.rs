//! Published-asset paths: one source of truth for where generated files live.
//!
//! Layout rule: everything the nightly sweep regenerates lives under
//! `proof/` and commits — no gitignored working copies, no separate
//! publish step, no duplicated trees. Local runs write the same paths,
//! so what you see is what ships.

/// The graph this repo publishes. Every related asset shares the stem.
pub const GRAPH_NAME: &str = "portfolio-graph";

/// Collector output: node/edge JSON. Committed — the nightly sweep
/// regenerates it and commits it back.
pub const GRAPH_JSON: &str = "data/portfolio-graph.json";

/// Schema lane output: the generated contract, paired beside the data
/// it validates. The drift test guards this file: whatever the types
/// derive is what ships.
pub const SCHEMA_JSON: &str = "data/portfolio-graph-schema.json";

/// Cartographer output: the verbatim fragment.
pub const FRAGMENT_MD: &str = "proof/fragments/portfolio-graph.md";

/// Commit-data table fragment: the data section of the proof page,
/// computed from the fresh JSON. Rendered by the charts lane,
/// assembled by `oxdock-render` — never hand-edited.
pub const COMMIT_DATA_MD: &str = "proof/fragments/commit-data.md";

/// Commit-share pie SVG.
pub const SHARE_SVG: &str = "proof/charts/portfolio-graph-share.svg";

/// Commit-share section fragment (generated with cache-bust, like pins).
pub const SHARE_FRAG_MD: &str = "proof/fragments/commit-share.md";

/// Developer Focus bars SVG.
pub const DEVELOPER_FOCUS_SVG: &str = "proof/charts/portfolio-graph-developer-focus.svg";

/// Developer-focus section fragment (generated with cache-bust).
pub const FOCUS_FRAG_MD: &str = "proof/fragments/developer-focus.md";

/// Commit activity flame SVGs (strip + per-band badges live under
/// `proof/charts/` and `proof/indicators/`): portfolio → repos → branches
/// → embers, widths pro-rata 30d share, color + motion from the heat bands.
pub const FLAME_SVG: &str = "proof/charts/portfolio-graph-flame.svg";

/// Flame strip section fragment (generated with cache-bust).
pub const FLAME_FRAG_MD: &str = "proof/fragments/heat-flame.md";

/// Pinned repo cards live one-per-file under this dir, slugged by repo
/// name: `portfolio-graph-pin-{slug}.svg`.
pub const CHARTS_DIR: &str = "proof/charts";

/// Popular repositories fragment: the 2-col table embedding the pin
/// cards, regenerated with the family — never hand-edited.
pub const PINS_FRAG_MD: &str = "proof/fragments/pins.md";

/// Status-indicator strip SVG. The per-repo badges below are what
/// tables embed; the strip above is the look-check.
pub const INDICATORS_SVG: &str = "proof/charts/portfolio-graph-indicators.svg";

/// One-off badge files, one per heat band (`<band>.svg`).
/// Table cells embed these; the strip above is the preview sheet.
pub const INDICATORS_DIR: &str = "proof/indicators";

/// Committed render proof: OxDock-templated table embedding the badges,
/// so github.com shows exactly what readers see. Regenerate, never edit.
pub const PROOF_INDICATORS_MD: &str = "proof/indicators.md";

/// Assembled proof page. `oxdock-render` owns it — never hand-edited.
pub const PROOF_README_MD: &str = "proof/README.md";

/// Checked-in schema expectation the drift test guards. Same file the
/// schema lane writes — generation and expectation are one path.
pub const COMMITTED_SCHEMA: &str = "data/portfolio-graph-schema.json";
