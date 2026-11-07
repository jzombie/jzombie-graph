use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Everything the binary needs is config-driven (spec: open-source checklist).
/// `values.toml` owns repo source, edge kinds, branch scope, alias mapping,
/// and private handling. CLI flags override the source selector.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub collection: Collection,
    #[serde(default)]
    pub aliases: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Explicit repo list (`owner/repo`). When non-empty, used as-is.
    #[serde(default)]
    pub repos: Vec<String>,
    /// Otherwise enumerate this user/org (exactly one should be set).
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub org: Option<String>,
    /// Include forks in enumeration (default true — filtering is the
    /// visibility pass's job, not collection's).
    #[serde(default = "default_true")]
    pub include_forks: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    /// Max branches per repo to union edges across (0 = default branch only).
    /// Spec wants all branches; cap exists to bound API cost.
    #[serde(default = "default_max_branches")]
    pub max_branches: usize,
    /// Window days for recency/xrefs/traffic-adjacent queries.
    #[serde(default = "default_window_days")]
    pub window_days: u32,
    /// package name -> portfolio repo id (`owner/repo`).
    /// Names diverge everywhere; this table is the fix (all ecosystems).
    #[serde(default)]
    pub extra_aliases: HashMap<String, String>,
}

fn default_true() -> bool {
    true
}
fn default_max_branches() -> usize {
    20
}
fn default_window_days() -> u32 {
    30
}

impl Default for Source {
    fn default() -> Self {
        Self {
            repos: vec![],
            user: None,
            org: None,
            include_forks: true,
        }
    }
}

impl Default for Collection {
    fn default() -> Self {
        Self {
            max_branches: default_max_branches(),
            window_days: default_window_days(),
            extra_aliases: HashMap::new(),
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))
    }

    /// Resolve `package name -> repo id`, merging top-level aliases with
    /// per-collection extras (extras win on conflict).
    pub fn alias_map(&self) -> HashMap<String, String> {
        let mut m = self.aliases.clone();
        m.extend(self.collection.extra_aliases.clone());
        m
    }

    /// Validate the source selector: explicit list XOR user/org.
    pub fn validate(&self) -> Result<()> {
        let has_list = !self.source.repos.is_empty();
        let has_enum = self.source.user.is_some() || self.source.org.is_some();
        anyhow::ensure!(
            has_list ^ has_enum,
            "config must set exactly one of: source.repos OR source.user/source.org"
        );
        if self.source.user.is_some() && self.source.org.is_some() {
            anyhow::bail!("config must not set both source.user and source.org");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_needs_explicit_source() {
        assert!(Config::default().validate().is_err());
    }

    #[test]
    fn explicit_list_validates() {
        let mut c = Config::default();
        c.source.repos = vec!["me/a".into()];
        assert!(c.validate().is_ok());
    }

    #[test]
    fn alias_merge_extras_win() {
        let mut c = Config::default();
        c.aliases.insert("foo".into(), "me/old".into());
        c.collection
            .extra_aliases
            .insert("foo".into(), "me/new".into());
        assert_eq!(c.alias_map()["foo"], "me/new");
    }
}
