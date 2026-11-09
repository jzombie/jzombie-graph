//! Token readers. Values live in `style.oxfile`; this module only parses.
//! Okabe-Ito bands + gray dormant. Snapshots pin bytes, so tokens are stable.

use std::collections::HashMap;
use std::sync::OnceLock;

fn tokens() -> &'static HashMap<String, String> {
    static TOKENS: OnceLock<HashMap<String, String>> = OnceLock::new();
    TOKENS.get_or_init(|| {
        // The $tokens map literal, parsed once as JSON.
        let text = include_str!("../style.oxfile");
        let at = text
            .lines()
            .position(|l| l.trim_start().starts_with("LET $tokens: TOKENS"))
            .expect("TOKENS map missing in style.oxfile");
        let mut depth = 0;
        let mut body = String::new();
        for l in text.lines().skip(at) {
            depth += l.chars().filter(|c| *c == '{').count();
            depth -= l.chars().filter(|c| *c == '}').count();
            body.push_str(l);
            body.push('\n');
            if depth == 0 && body.contains('{') {
                break;
            }
        }
        let open = body.find('{').expect("map open");
        serde_json::from_str(&body[open..]).expect("TOKENS parses as JSON")
    })
}

fn token(name: &str) -> String {
    tokens()
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("token ${name} missing in style.oxfile"))
}

/// Token names in the file, for the drift test.
pub fn token_names() -> Vec<&'static str> {
    tokens().keys().map(String::as_str).collect()
}

pub fn heating() -> String {
    token("heating")
}
pub fn steady() -> String {
    token("steady")
}
pub fn cooling() -> String {
    token("cooling")
}
pub fn dormant() -> String {
    token("dormant")
}
pub fn other() -> String {
    token("dormant")
}
pub fn focus_negative() -> String {
    token("cat7")
}
pub fn ink() -> String {
    token("ink")
}
pub fn muted() -> String {
    token("ink")
}
pub fn ink_on_bar() -> String {
    token("ink_on_bar")
}
pub fn dim() -> String {
    token("dim")
}
pub fn portfolio_bar() -> String {
    token("portfolio_bar")
}
pub fn family() -> String {
    token("family")
}

/// Pulse class for a heat: heating fast, steady slow, rest static.
pub fn heat_pulse(h: Option<f32>) -> &'static str {
    match h {
        Some(v) if v > 1.5 => "pulse",
        Some(v) if v > 0.5 => "pulse slow",
        _ => "",
    }
}

/// Category color for series `i`, cycling the shared stops.
pub fn categorical(i: usize) -> String {
    const ORDER: [&str; 8] = [
        "cooling", "steady", "heating", "cat3", "cat4", "cat5", "cat6", "cat7",
    ];
    token(ORDER[i % ORDER.len()])
}
