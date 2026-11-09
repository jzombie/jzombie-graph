//! Published-asset paths: one source of truth for where generated files live.
//!
//! Layout rule (see plan: Publishing): directory names carry the type,
//! file names carry the graph's own name. The local `data/` tree mirrors
//! the Pages staging layout exactly, so a local run and a CI run produce
//! the same relative paths — only the tree root differs (`data/` vs the
//! workflow's upload directory). `data/` is gitignored: after the first
//! publish the assets are retrievable from Pages, never from the repo.
//!
//! The one exception is [`COMMITTED_SCHEMA`]: the checked-in expectation
//! the schema drift test guards. It is source, not generated data.

/// The graph this repo publishes. Every related asset shares the stem.
pub const GRAPH_NAME: &str = "portfolio-graph";

/// Local root for generated working copies. Gitignored.
pub const DATA_DIR: &str = "data";

/// Collector output: node/edge JSON. Was `graph.json` at the repo root.
pub const GRAPH_JSON: &str = "data/json/portfolio-graph.json";

/// Schema lane output: generated copy of the contract. The committed
/// expectation lives at [`COMMITTED_SCHEMA`]; publishing ships this copy.
pub const SCHEMA_JSON: &str = "data/json/portfolio-graph-schema.json";

/// Cartographer output: the verbatim fragment. Was `ARCHITECTURE.md`
/// at the repo root.
pub const FRAGMENT_MD: &str = "data/markdown/fragments/portfolio-graph.md";

/// Commit-share pie SVG. Published asset once wired; until then a local
/// working copy under gitignored `data/`.
pub const SHARE_SVG: &str = "data/svg/portfolio-graph-share.svg";

/// Focus ranking bars SVG. Same deal: local working copy until wired.
pub const FOCUS_SVG: &str = "data/svg/portfolio-graph-focus.svg";

/// Heat flame SVG: portfolio → repos → branches → embers, widths pro-rata
/// 30d share, color + motion from the heat bands. Same working-copy deal.
pub const FLAME_SVG: &str = "data/svg/portfolio-graph-flame.svg";

/// Status-indicator strip preview SVG. Working copy until the per-repo badges
/// land; not a published asset yet.
pub const INDICATORS_SVG: &str = "data/svg/portfolio-graph-indicators.svg";

/// One-off badge files, one per heat band (`<band>.svg`).
/// Table cells embed these; the strip above is the preview sheet.
pub const INDICATORS_DIR: &str = "data/svg/indicators";

/// Committed render proof: OxDock-templated table embedding the badges,
/// so github.com shows exactly what readers see. Regenerate, never edit.
pub const PREVIEW_INDICATORS_MD: &str = "preview/indicators.md";

/// Committed badge copies the preview table embeds. Same bytes as the
/// working copies; copied at preview time, never hand-placed.
pub const PREVIEW_INDICATORS_DIR: &str = "preview/indicators";

/// Committed proof page: every visual artifact on one page, as github.com
/// renders it. Assembled by the preview lane, never hand-edited.
pub const PREVIEW_PORTFOLIO_MD: &str = "preview/portfolio-graph.md";

/// Checked-in schema expectation the drift test guards. Source, not data:
/// changing the types means consciously copying the lane output over this.
pub const COMMITTED_SCHEMA: &str = "json/portfolio-graph-schema.json";
