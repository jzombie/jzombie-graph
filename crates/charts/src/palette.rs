//! Shared categorical palette: every chart in this crate draws from these,
// so the portfolio's charts read as one family.
//!
//! Okabe-Ito (colorblind-safe) + gray `Other`. Ink passes WCAG AA on white
//! (4.54) and black (4.62) — charts must read in GitHub light and dark mode
//! from a single static file. Keep values stable: snapshots pin bytes.

/// Category colors, cycled by series index.
pub const CATEGORICAL: [&str; 8] = [
    "#E69F00", "#56B4E9", "#009E73", "#F0E442", "#0072B2", "#D55E00", "#CC79A7", "#999999",
];

/// The overflow bucket. Always gray, never a category color.
pub const OTHER: &str = "#BBBBBB";

/// Body/legend text.
pub const INK: &str = "#767676";

/// Secondary text (empty states).
pub const MUTED: &str = "#767676";

/// Category color for series `i`, cycling the palette.
pub fn categorical(i: usize) -> &'static str {
    CATEGORICAL[i % CATEGORICAL.len()]
}
