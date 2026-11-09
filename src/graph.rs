use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Stable schema version. Bump only with a documented migration.
pub const SCHEMA_VERSION: u32 = 7;

fn is_false(b: &bool) -> bool {
    !*b
}

/// Mechanically collected edge kinds. Dependence kinds count toward fan-in;
/// association kinds display only (see plan: workflow/docs/xrefs).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    Manifest,
    Submodule,
    Registry,
    Workflow,
    Docs,
    Xrefs,
}

impl EdgeKind {
    /// Per spec: manifest/submodule/registry are dependence (count toward
    /// fan-in); lockfile is a gate (not a kind); workflow/docs/xrefs are
    /// associations (displayed only).
    pub fn is_dependence(self) -> bool {
        matches!(
            self,
            EdgeKind::Manifest | EdgeKind::Submodule | EdgeKind::Registry
        )
    }
}

/// Structural roles from *dependence* fan-in/fan-out
/// (`EdgeKind::is_dependence`: manifest/submodule/registry) plus the
/// default-binary tag. A node holds every tag it earns — position and
/// product are independent, so `term-wm` is both middleware (transit
/// positionally) and application (has a default binary).
///
/// - `Foundation`: depended upon, depends on nothing.
/// - `Middleware`: both used and using (`in > 0, out > 0`).
/// - `Application`: has a default binary — nothing else earns it.
/// - `Isolated`: no dependence edges.
/// - A consuming endpoint with no default binary and no dependents here
///   falls back to middleware (assumed transit — the private graph may
///   depend on it where this data cannot see).
///
/// The contract is the stored `Node.layer` tag set, set once by
/// `apply_layers` and never recomputed: identical in the briefing lane
/// and the public file, regardless of render. Dependence edges
/// (docs/workflow/xrefs) never count.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Foundation,
    Middleware,
    Application,
    #[default]
    Isolated,
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Layer::Foundation => "foundation",
            Layer::Middleware => "middleware",
            Layer::Application => "application",
            Layer::Isolated => "isolated",
        };
        write!(f, "{s}")
    }
}

/// Gauge scope: all branches or default-branch-only. CLI-selectable;
/// frozen into the JSON so audits know what the headlines sum.
/// `branch_heat` always carries every branch either way.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum VelocityScope {
    #[default]
    All,
    Default,
}

impl VelocityScope {
    /// In-scope check: everything under `All`, only the default under `Default`.
    pub fn includes(self, default_branch: &str, branch: &str) -> bool {
        matches!(self, VelocityScope::All) || branch == default_branch
    }
}

/// Per-branch velocity, hottest first. Stored raw counts (same rule as
/// the top-level gauges: inputs frozen, heat derivable on read).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BranchHeat {
    pub branch: String,
    pub is_default: bool,
    pub commits_1d: u32,
    pub commits_5d: u32,
    pub commits_15d: u32,
    pub commits_30d: u32,
}

/// A recent pull request, branch-associated via the head ref name.
/// Display only, never scored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RecentPr {
    pub title: String,
    pub branch: String,
    pub base: String,
    pub state: String,
}

/// A sampled planning thread: what it is, where it stands, how tagged
/// and how discussed. Display plus agent roadmap signal — the plan
/// outside the code, minable straight from the JSON. Never scored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RecentIssue {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub comments: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Node {
    /// Canonical id: `owner/repo` full name.
    pub id: String,
    pub owner: String,
    pub name: String,
    /// API-native fork flag. Rides along as fact, not verdict.
    pub fork: bool,
    pub private: bool,
    /// Upstream full name when `fork == true` (if known).
    pub upstream: Option<String>,
    /// Commits ahead of upstream authored by you (`compare` API + authorship
    /// filter). Rule (a): divergence re-admits forks. Raw ahead counts lie:
    /// a fork frozen while upstream rewrote history shows ahead commits
    /// authored entirely by upstream — those count as pristine, not diverged.
    pub ahead_by: Option<u32>,
    /// Last push to the repo (API `pushed_at`). Display fact only, never scored.
    /// Free in the enumerate/get responses — staleness/heartbeat signal.
    pub pushed_at: Option<DateTime<Utc>>,
    /// Repo-about facts (API-native, free in the same responses).
    /// Display/classification only, never scored. Stripped with private nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Tier-0 magnet signals: free in enumerate/get (+ tree for has_*).
    /// Display/classification only, never scored. Stripped with private nodes.
    #[serde(default)]
    pub stars: u32,
    #[serde(default)]
    pub forks_count: u32,
    #[serde(default)]
    pub open_issues: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub archived: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(default)]
    pub size_kb: u32,
    /// Agent-parseability audit from the default-branch tree (free).
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_readme: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_llms_txt: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_agents_md: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_license_file: bool,
    /// Commit velocity: 1d/5d/15d/30d commit counts summed across
    /// in-scope branches in one GraphQL call; fail-open 0. Owner-authored
    /// when `Graph::velocity_owner` is set, all authors otherwise.
    /// `commit_heat()` means the three sub-window pro-rata heats against
    /// the 30d baseline, so a flat repo reads 1.0 in every window.
    #[serde(default)]
    pub commits_1d: u32,
    #[serde(default)]
    pub commits_5d: u32,
    #[serde(default)]
    pub commits_15d: u32,
    #[serde(default)]
    pub commits_30d: u32,
    /// Issue touches: open/closed thread updates per window, repo-level
    /// (same GraphQL call as velocity, no new requests). Open vs closed
    /// ride separately — resolution traffic never hides as backlog
    /// attention. Owner-authored when `Graph::velocity_owner` is set only
    /// insofar as the sweep owner works the threads; bots that only bump
    /// PRs never land here (`issues` excludes pull requests).
    #[serde(default)]
    pub issues_open_1d: u32,
    #[serde(default)]
    pub issues_open_5d: u32,
    #[serde(default)]
    pub issues_open_15d: u32,
    #[serde(default)]
    pub issues_open_30d: u32,
    #[serde(default)]
    pub issues_closed_1d: u32,
    #[serde(default)]
    pub issues_closed_5d: u32,
    #[serde(default)]
    pub issues_closed_15d: u32,
    #[serde(default)]
    pub issues_closed_30d: u32,
    /// Owner-authored issue comments per window (counted in the pipeline
    /// from the sampled threads; caps in the query). This is the volume
    /// the issue score reads — opening an issue plus commenting on it is
    /// focus; an untouched thread is just backlog.
    #[serde(default)]
    pub issue_comments_1d: u32,
    #[serde(default)]
    pub issue_comments_5d: u32,
    #[serde(default)]
    pub issue_comments_15d: u32,
    #[serde(default)]
    pub issue_comments_30d: u32,
    /// Label census over the sampled recent threads (`enhancement: 3`).
    /// Logged so scheduling mix (bugs vs features vs research) is minable
    /// from the JSON; never scored, never weighted.
    #[serde(default)]
    pub issue_labels: BTreeMap<String, u32>,
    /// Captured issue heat as of `generated_at` (same pro-rata shape as
    /// `commit_heat`, over open+closed touches). Stored so the JSON
    /// carries the gauge directly; frozen at collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_heat: Option<f32>,
    /// Captured commit heat as of `generated_at` (same formula as
    /// `commit_heat()`). Stored so the JSON carries the gauge directly;
    /// frozen at collection — recompute live for today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_heat: Option<f32>,
    /// Captured focus as of `generated_at` (same formula as `focus()`).
    /// Frozen at collection; the offline `--format focus` path recomputes
    /// it live from the inputs above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<f32>,
    /// Latest commit date under the sweep's authorship rule (newest 30d
    /// date, else the overall newest). Idle and focus measure from here —
    /// feature work counts, so a live branch keeps the repo warm while
    /// main sits still. A bot-only push never warms an owner-filtered repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_commit_at: Option<DateTime<Utc>>,
    /// Per-branch velocity, hottest first. The headline gauges below sum
    /// across branches (feature work counts — a weeks-long branch is real
    /// velocity); this breakdown shows where it landed and flags the
    /// default branch. Replaces the old branch-name list. Sums double-count
    /// merged commits (reachable from two refs); it reads activity, not
    /// unique work.
    #[serde(default)]
    pub branch_heat: Vec<BranchHeat>,
    /// Recent PRs (GraphQL, updated-desc, capped). Branch association
    /// rides `branch` (head ref); titles are work descriptions.
    /// Display only, never scored.
    #[serde(default)]
    pub recent_prs: Vec<RecentPr>,
    /// Sampled planning threads (GraphQL, updated-desc, capped at 10).
    /// Titles plus labels plus state say what is being planned, prepped,
    /// or scheduled outside the code — bugs vs enhancements vs research
    /// at a glance. Display plus agent roadmap signal, never scored.
    #[serde(default)]
    pub recent_issues: Vec<RecentIssue>,
    /// Whether the repo has a default binary: explicit bin/script targets
    /// in a manifest, or Cargo autobin layout (`src/main.rs`, `src/bin/`),
    /// or a Go `package main`. Helper-only binaries (name matches neither
    /// the package nor `default-run`) do NOT count. Set at ingestion from
    /// manifest text plus the default tree (no extra calls except
    /// tree-gated Go candidates); unioned across workspace members.
    /// Independent application tag: rendered with the layer cell
    /// (`· application`) — it never moves the positional word.
    #[serde(default)]
    pub has_default_binary: bool,
    /// Structural tags, set once at ingestion by `apply_layers` on the
    /// true graph (private transit included) and preserved verbatim by
    /// `public_subgraph` — so the values are identical in the briefing
    /// lane and the committed file, regardless of render. Persisted so
    /// tags are minable straight from the JSON (`jq` on `.layer[]`).
    /// Stated cost: a public tag can reflect a private dependent
    /// (never which one, no counts).
    #[serde(default)]
    pub layer: BTreeSet<Layer>,
    /// Computed visibility: renderers filter blindly on this, never re-decide.
    pub visible: bool,
    /// Why visible / hidden — machine-readable for the briefing digest.
    pub visible_reason: Option<String>,
}

/// Pro-rata heat from raw window counts: mean of the 1d/5d/15d heats
/// against the 30d baseline. Single formula — `Node::commit_heat` and the
/// collect path both call this, never a copy.
pub fn commit_heat_from_windows(c1: u32, c5: u32, c15: u32, c30: u32) -> Option<f32> {
    if c30 == 0 {
        return None;
    }
    let base = c30 as f32 / 30.0;
    let h = |c: u32, w: f32| c as f32 / (w * base);
    Some((h(c1, 1.0) + h(c5, 5.0) + h(c15, 15.0)) / 3.0)
}

impl Node {
    /// Commit heat (self-normalized, never scored): mean of the 1d/5d/15d
    /// pro-rata heats against the trailing-30d baseline (a flat repo puts
    /// ~w/30 of its 30d commits in any w-day slice, so every window reads
    /// 1.0 and the mean stays 1.0). Above 1.5 heating, ~1.0 steady, below
    /// 0.5 cooling. None means no 30d baseline — dormant when the windows
    /// are also 0, fresh otherwise.
    pub fn commit_heat(&self) -> Option<f32> {
        commit_heat_from_windows(
            self.commits_1d,
            self.commits_5d,
            self.commits_15d,
            self.commits_30d,
        )
    }

    /// Issue heat (same pro-rata shape, over owner-authored comment
    /// volume per window). None means no 30d comment baseline — an
    /// untouched backlog never scores, no matter how many threads it holds.
    pub fn issue_heat(&self) -> Option<f32> {
        commit_heat_from_windows(
            self.issue_comments_1d,
            self.issue_comments_5d,
            self.issue_comments_15d,
            self.issue_comments_30d,
        )
    }

    /// Whole days since the default branch last moved, if known.
    /// Prefers `last_commit_at`, falls back to repo-level `pushed_at`
    /// (pre-recency JSON), unknown when both are missing.
    pub fn idle_days(&self, now: &chrono::DateTime<chrono::Utc>) -> Option<i64> {
        let t = self.last_commit_at.or(self.pushed_at)?;
        Some((*now - t).num_days().max(0))
    }

    /// Focus gauge (transparent heuristic, never scored): what I'm
    /// working on now. Mean of the available heats (commits, issues) minus
    /// one point per idle month, so dormancy depth counts — a repo quiet
    /// for 8 months sinks below one quiet for a week, even when both read
    /// heat 0. A signal with no 30d baseline sits out (triage-free repos
    /// score on commits alone, and vice versa). None only when the last
    /// push is unknown.
    pub fn focus(&self, now: &chrono::DateTime<chrono::Utc>) -> Option<f32> {
        let idle = self.idle_days(now)? as f32;
        let heats: Vec<f32> = [self.commit_heat(), self.issue_heat()]
            .into_iter()
            .flatten()
            .collect();
        let base = if heats.is_empty() {
            0.0
        } else {
            heats.iter().sum::<f32>() / heats.len() as f32
        };
        Some(base - idle / 30.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
    /// Every branch whose manifest (or equivalent) declares this edge.
    pub declaring_branches: Vec<String>,
    /// Recency of the most recent declaring branch (RFC3339), if known.
    pub recency: Option<DateTime<Utc>>,
    /// Resolved version as briefing fact (lockfile), never scored.
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Graph {
    pub schema_version: u32,
    pub generated_at: DateTime<Utc>,
    /// Gauge scope the headlines were summed under. Always recorded so
    /// audits know whether `commits_*` cover all branches or the default.
    #[serde(default)]
    pub velocity_scope: VelocityScope,
    /// Whose authorship the velocity gauges count. `Some(login)` when the
    /// sweep resolved the owner's user id and filtered every `history`
    /// call by it; `None` means all authors counted (org scope or an
    /// unresolvable id — the briefing digest names which).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity_owner: Option<String>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Graph {
    /// Parse JSON metrics back into a `Graph` (downstream crates use this;
    /// same shape `render_json` emits). Rejects schema mismatches explicitly
    /// so drift surfaces as an error, not silent misread.
    pub fn from_json(s: &str) -> anyhow::Result<Self> {
        let g: Self = serde_json::from_str(s)?;
        anyhow::ensure!(
            g.schema_version == SCHEMA_VERSION,
            "schema v{} != crate v{SCHEMA_VERSION} (regenerate JSON)",
            g.schema_version
        );
        Ok(g)
    }

    pub fn new(nodes: Vec<Node>, mut edges: Vec<Edge>) -> Self {
        // Deterministic output: sort nodes by id, edges by (source, target, kind).
        let mut nodes = nodes;
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        edges.sort_by(|a, b| (&a.source, &a.target, a.kind).cmp(&(&b.source, &b.target, b.kind)));
        Self {
            schema_version: SCHEMA_VERSION,
            generated_at: Utc::now(),
            velocity_scope: VelocityScope::All,
            velocity_owner: None,
            nodes,
            edges,
        }
    }

    /// One rule for every layer call site: position earns its tag, and a
    /// default binary independently earns `Application` — nothing else
    /// does. Both directions is middleware; depended-upon is foundation;
    /// the edgeless are isolated. A consuming endpoint with no default
    /// binary and no dependents here claims no evidence either way, so
    /// it falls back to middleware (assumed transit — the private graph
    /// may depend on it where this data cannot see).
    pub fn classify_layers(fan_in: u32, fan_out: u32, has_default_binary: bool) -> BTreeSet<Layer> {
        let mut out = BTreeSet::new();
        if fan_in > 0 && fan_out > 0 {
            out.insert(Layer::Middleware);
        } else if fan_in > 0 {
            out.insert(Layer::Foundation);
        } else if fan_out == 0 {
            out.insert(Layer::Isolated);
        }
        if has_default_binary {
            out.insert(Layer::Application);
        }
        if out.is_empty() {
            out.insert(Layer::Middleware);
        }
        out
    }

    /// Joined tag display (`middleware · application`), enum order.
    pub fn layer_string(layer: &BTreeSet<Layer>) -> String {
        layer
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(" · ")
    }
    /// Node fan-in: count of incoming *dependence* edges (how many portfolio
    /// repos depend on this node). Associations never contribute.
    /// This is the "most depended-upon" ranking — no scope, no counting
    /// declaration sites.
    pub fn node_fan_in(&self) -> BTreeMap<String, u32> {
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for n in &self.nodes {
            counts.entry(n.id.clone()).or_insert(0);
        }
        for e in &self.edges {
            if e.kind.is_dependence() {
                *counts.entry(e.target.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Node fan-out: count of outgoing *dependence* edges (how many
    /// portfolio repos this node depends on). Mirror of `node_fan_in`;
    /// associations never contribute.
    pub fn node_fan_out(&self) -> BTreeMap<String, u32> {
        let mut counts: BTreeMap<String, u32> = BTreeMap::new();
        for n in &self.nodes {
            counts.entry(n.id.clone()).or_insert(0);
        }
        for e in &self.edges {
            if e.kind.is_dependence() {
                *counts.entry(e.source.clone()).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Ad-hoc layer recompute over `self`'s edges (same rule as
    /// `apply_layers`). The stored `Node.layer` tag set is the contract;
    /// this is for one-off queries on graphs that never went through
    /// ingestion.
    pub fn node_layers(&self) -> BTreeMap<String, BTreeSet<Layer>> {
        let fan_in = self.node_fan_in();
        let fan_out = self.node_fan_out();
        self.nodes
            .iter()
            .map(|n| {
                let i = fan_in.get(&n.id).copied().unwrap_or(0);
                let o = fan_out.get(&n.id).copied().unwrap_or(0);
                (
                    n.id.clone(),
                    Self::classify_layers(i, o, n.has_default_binary),
                )
            })
            .collect()
    }

    /// Single-node layer tags (same rule as `node_layers`).
    pub fn node_layer(&self, id: &str) -> BTreeSet<Layer> {
        let fan_in = self
            .edges
            .iter()
            .filter(|e| e.kind.is_dependence() && e.target == id)
            .count();
        let fan_out = self
            .edges
            .iter()
            .filter(|e| e.kind.is_dependence() && e.source == id)
            .count();
        let bin = self
            .nodes
            .iter()
            .find(|n| n.id == id)
            .is_some_and(|n| n.has_default_binary);
        Self::classify_layers(fan_in as u32, fan_out as u32, bin)
    }

    /// Ordering: fan-in desc, stability tiebreak (id asc). Pure helper for renderers.
    pub fn nodes_by_fan_in(&self) -> Vec<(&Node, u32)> {
        let counts = self.node_fan_in();
        let mut out: Vec<(&Node, u32)> = self
            .nodes
            .iter()
            .map(|n| (n, *counts.get(&n.id).unwrap_or(&0)))
            .collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
        out
    }
}

/// Own-work repos: everything you actually contributed to. Non-forks always
/// count; forks count only when diverged (`ahead_by > 0` own commits) or
/// renamed from the parent (name differs from the upstream repo name).
/// Pristine forks never count. This is the workspace-wide filter for "did I
/// work on this repo" — renderers use it instead of re-deciding. (The
/// `visible` flag answers a different question: what the map draws, where
/// isolation and staleness also apply.)
pub fn is_own_work(node: &Node) -> bool {
    if !node.fork {
        return true;
    }
    if node.ahead_by.unwrap_or(0) > 0 {
        return true;
    }
    let Some(up) = node.upstream.as_deref() else {
        return false;
    };
    let parent = up.split('/').nth(1).unwrap_or(up);
    !parent.eq_ignore_ascii_case(&node.name)
}

/// Compute `visible` for every node per the locked rules:
///
/// - Private repos: `visible = false` always (they compute as transit nodes,
///   stripped before render — no nodes, stubs, links, counts, or deltas).
/// - Forks: excluded by default; re-admitted by (a) divergence (own ahead
///   commits > 0) or (b) load-bearing fan-in (a portfolio repo depends on the fork).
/// - Tier 3 isolation: >= 1 portfolio edge (in or out, dependence *or*
///   association) or omit. Study-forks pointing at other people's repos
///   self-omit here because their only edges leave the portfolio.
pub fn apply_visibility(nodes: &mut [Node], edges: &[Edge]) {
    let portfolio: BTreeSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();

    // Portfolio-internal edges only (both endpoints in the collected set).
    // Private nodes still count as transit for fan-in purposes.
    let mut has_portfolio_edge: BTreeSet<String> = BTreeSet::new();
    let mut has_fanin: BTreeSet<String> = BTreeSet::new();
    for e in edges {
        let internal =
            portfolio.contains(e.source.as_str()) && portfolio.contains(e.target.as_str());
        if !internal {
            continue;
        }
        has_portfolio_edge.insert(e.source.clone());
        has_portfolio_edge.insert(e.target.clone());
        // Fan-in: someone in the portfolio depends *on* this node.
        // Any edge kind counts as structural fan-in for the fork rule;
        // the fan-in *count* (scoring) uses dependence edges only.
        has_fanin.insert(e.target.clone());
    }

    for n in nodes.iter_mut() {
        if n.private {
            n.visible = false;
            n.visible_reason = Some("private-transit: computes, stripped before render".into());
            continue;
        }
        if n.fork {
            let diverged = n.ahead_by.unwrap_or(0) > 0;
            let load_bearing = has_fanin.contains(n.id.as_str());
            if !diverged && !load_bearing {
                // Forks without divergence or fan-in are hidden even if they
                // would otherwise pass Tier 3 (e.g. a study fork whose only
                // portfolio edge is its upstream link out — not own work).
                // They still need a reason string for the digest.
                n.visible = false;
                n.visible_reason = Some("fork: neither diverged nor load-bearing".into());
                continue;
            }
            // Re-admitted forks still pass through Tier 3 below (labeled fork).
        }
        if has_portfolio_edge.contains(n.id.as_str()) {
            n.visible = true;
            n.visible_reason = Some(if n.fork {
                if n.ahead_by.unwrap_or(0) > 0 {
                    "fork: diverged (ahead of upstream), has portfolio edge".into()
                } else {
                    "fork: load-bearing fan-in, has portfolio edge".into()
                }
            } else {
                "tier3: >=1 portfolio edge".into()
            });
        } else {
            n.visible = false;
            n.visible_reason = Some("tier3: isolated, no portfolio edge".into());
        }
    }
}

/// Set `node.layer` for every node from *dependence* fan-in/fan-out
/// (`EdgeKind::is_dependence` only — workflow/docs/xrefs never count).
/// Call once on the true graph (private included) before `select_output`;
/// `public_subgraph` preserves the values verbatim, so the stored layer
/// is render-stable by construction. Re-running it on an already-stripped
/// public graph would give public-only values — don't; set once upstream.
pub fn apply_layers(nodes: &mut [Node], edges: &[Edge]) {
    use std::collections::BTreeMap;
    let mut fan_in: BTreeMap<&str, u32> = BTreeMap::new();
    let mut fan_out: BTreeMap<&str, u32> = BTreeMap::new();
    for e in edges.iter().filter(|e| e.kind.is_dependence()) {
        *fan_in.entry(e.target.as_str()).or_default() += 1;
        *fan_out.entry(e.source.as_str()).or_default() += 1;
    }
    for n in nodes.iter_mut() {
        let i = fan_in.get(n.id.as_str()).copied().unwrap_or(0);
        let o = fan_out.get(n.id.as_str()).copied().unwrap_or(0);
        n.layer = Graph::classify_layers(i, o, n.has_default_binary);
    }
}

/// Public subgraph: both endpoints public. Scoring sees the true graph;
/// all surfaces show this. Private nodes vanish entirely (no stubs).
pub fn public_subgraph(graph: &Graph) -> Graph {
    let public: BTreeSet<&str> = graph
        .nodes
        .iter()
        .filter(|n| !n.private)
        .map(|n| n.id.as_str())
        .collect();
    let nodes: Vec<Node> = graph.nodes.iter().filter(|n| !n.private).cloned().collect();
    let edges: Vec<Edge> = graph
        .edges
        .iter()
        .filter(|e| public.contains(e.source.as_str()) && public.contains(e.target.as_str()))
        .cloned()
        .collect();
    let mut out = Graph::new(nodes, edges);
    // Audit fields survive the strip: the public file must say what its
    // gauges summed and whose authorship they count.
    out.velocity_scope = graph.velocity_scope;
    out.velocity_owner = graph.velocity_owner.clone();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, fork: bool, private: bool, ahead_by: Option<u32>) -> Node {
        let (owner, name) = id.split_once('/').unwrap();
        Node {
            id: id.into(),
            owner: owner.into(),
            name: name.into(),
            fork,
            private,
            upstream: None,
            ahead_by,
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
            branch_heat: vec![],
            recent_prs: vec![],
            visible: false,
            visible_reason: None,
            layer: std::collections::BTreeSet::from([Layer::Isolated]),
            has_default_binary: false,
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
            focus: None,
            last_commit_at: None,
        }
    }

    fn edge(source: &str, target: &str, kind: EdgeKind) -> Edge {
        Edge {
            source: source.into(),
            target: target.into(),
            kind,
            declaring_branches: vec!["main".into()],
            recency: None,
            version: None,
        }
    }

    #[test]
    fn fork_excluded_by_default_diverged_readmitted() {
        let mut nodes = vec![
            node("me/a", false, false, None),
            node("me/study-fork", true, false, Some(0)),
            node("me/diverged-fork", true, false, Some(7)),
        ];
        let edges = vec![
            edge("me/a", "me/diverged-fork", EdgeKind::Manifest),
            // study-fork points at someone else's repo: no portfolio edge.
        ];
        apply_visibility(&mut nodes, &edges);
        let v: BTreeMap<_, _> = nodes.iter().map(|n| (n.id.as_str(), n.visible)).collect();
        assert!(v["me/a"]);
        assert!(!v["me/study-fork"]);
        // diverged fork with a portfolio edge (fan-in from me/a) is visible.
        assert!(v["me/diverged-fork"]);
    }

    #[test]
    fn load_bearing_fork_admitted_without_divergence() {
        let mut nodes = vec![
            node("me/app", false, false, None),
            node("me/load-bearing-fork", true, false, Some(0)),
        ];
        let edges = vec![edge("me/app", "me/load-bearing-fork", EdgeKind::Manifest)];
        apply_visibility(&mut nodes, &edges);
        assert!(
            nodes
                .iter()
                .find(|n| n.id == "me/load-bearing-fork")
                .unwrap()
                .visible
        );
    }

    #[test]
    fn tier3_isolates_edgeless_nodes() {
        let mut nodes = vec![
            node("me/a", false, false, None),
            node("me/lonely", false, false, None),
        ];
        let edges = vec![];
        apply_visibility(&mut nodes, &edges);
        assert!(!nodes.iter().find(|n| n.id == "me/lonely").unwrap().visible);
        assert!(!nodes.iter().find(|n| n.id == "me/a").unwrap().visible);
    }

    #[test]
    fn private_nodes_are_transit_but_stripped() {
        let mut nodes = vec![
            node("me/pub", false, false, None),
            node("me/secret", false, true, None),
        ];
        let edges = vec![edge("me/pub", "me/secret", EdgeKind::Manifest)];
        apply_visibility(&mut nodes, &edges);
        // Private computes (edge exists in the true graph) but is never visible.
        assert!(!nodes.iter().find(|n| n.id == "me/secret").unwrap().visible);
        let g = Graph::new(nodes, edges);
        let pub_g = public_subgraph(&g);
        assert!(pub_g.nodes.iter().all(|n| !n.private));
        assert!(pub_g.edges.is_empty(), "edge to private node must vanish");
    }

    #[test]
    fn fan_in_counts_dependence_only() {
        let g = Graph::new(
            vec![
                node("me/a", false, false, None),
                node("me/b", false, false, None),
            ],
            vec![
                edge("me/a", "me/b", EdgeKind::Manifest),
                edge("me/a", "me/b", EdgeKind::Workflow),
                edge("me/a", "me/b", EdgeKind::Docs),
            ],
        );
        assert_eq!(g.node_fan_in()["me/b"], 1);
    }

    #[test]
    fn layers_classify_position_plus_default_binary_tags() {
        // lib <- mid <- app, plus a loner with no dependence edges.
        // Position earns its tag; a binary-less chain top with no
        // dependents falls back to middleware (assumed transit).
        let g = Graph::new(
            vec![
                node("me/lib", false, false, None),
                node("me/mid", false, false, None),
                node("me/app", false, false, None),
                node("me/loner", false, false, None),
            ],
            vec![
                edge("me/mid", "me/lib", EdgeKind::Manifest),
                edge("me/app", "me/mid", EdgeKind::Manifest),
            ],
        );
        let layers = g.node_layers();
        assert_eq!(layers["me/lib"], BTreeSet::from([Layer::Foundation]));
        assert_eq!(layers["me/mid"], BTreeSet::from([Layer::Middleware]));
        assert_eq!(layers["me/app"], BTreeSet::from([Layer::Middleware]));
        assert_eq!(layers["me/loner"], BTreeSet::from([Layer::Isolated]));
        assert_eq!(g.node_layer("me/app"), BTreeSet::from([Layer::Middleware]));
    }

    #[test]
    fn layers_default_binary_adds_application_tag() {
        // Same shape, every node with a default binary: position tags
        // don't move, application joins them — including the loner.
        let bin = |id: &str| Node {
            has_default_binary: true,
            ..node(id, false, false, None)
        };
        let g = Graph::new(
            vec![bin("me/lib"), bin("me/mid"), bin("me/app"), bin("me/loner")],
            vec![
                edge("me/mid", "me/lib", EdgeKind::Manifest),
                edge("me/app", "me/mid", EdgeKind::Manifest),
            ],
        );
        let layers = g.node_layers();
        assert_eq!(
            layers["me/lib"],
            BTreeSet::from([Layer::Foundation, Layer::Application])
        );
        // term-wm case: transit positionally, default binary in hand —
        // both tags, which is the whole point of the set.
        assert_eq!(
            layers["me/mid"],
            BTreeSet::from([Layer::Middleware, Layer::Application])
        );
        assert_eq!(layers["me/app"], BTreeSet::from([Layer::Application]));
        assert_eq!(
            layers["me/loner"],
            BTreeSet::from([Layer::Isolated, Layer::Application])
        );
        assert_eq!(
            g.node_layer("me/loner"),
            BTreeSet::from([Layer::Isolated, Layer::Application])
        );
    }

    #[test]
    fn layers_ignore_association_kinds() {
        // A docs-only link must not promote either endpoint.
        let g = Graph::new(
            vec![
                node("me/a", false, false, None),
                node("me/b", false, false, None),
            ],
            vec![edge("me/a", "me/b", EdgeKind::Docs)],
        );
        assert_eq!(g.node_layer("me/a"), BTreeSet::from([Layer::Isolated]));
        assert_eq!(g.node_layer("me/b"), BTreeSet::from([Layer::Isolated]));
    }

    #[test]
    fn stored_layer_survives_public_strip_verbatim() {
        // Determinism contract: `apply_layers` runs once on the true graph
        // (private transit included); `public_subgraph` preserves the stored
        // value, so both lanes report the same layer. Here `me/app` is
        // middleware because the private `me/secret` uses it — that verdict
        // is identical before and after stripping.
        let mut nodes = vec![
            node("me/app", false, false, None),
            node("me/lib", false, false, None),
            node("me/secret", false, true, None),
        ];
        let edges = vec![
            edge("me/app", "me/lib", EdgeKind::Manifest),
            edge("me/secret", "me/app", EdgeKind::Manifest),
        ];
        apply_visibility(&mut nodes, &edges);
        apply_layers(&mut nodes, &edges);
        let g = Graph::new(nodes, edges);
        let stored =
            |gr: &Graph, id: &str| gr.nodes.iter().find(|n| n.id == id).unwrap().layer.clone();
        assert_eq!(stored(&g, "me/app"), BTreeSet::from([Layer::Middleware]));
        let pub_g = public_subgraph(&g);
        assert_eq!(
            stored(&pub_g, "me/app"),
            BTreeSet::from([Layer::Middleware])
        );
        assert_eq!(
            stored(&pub_g, "me/lib"),
            BTreeSet::from([Layer::Foundation])
        );
    }

    #[test]
    fn commit_heat_compares_5d_against_own_30d_baseline() {
        let mut n = node("me/a", false, false, None);
        // Recent-heavy: 2/1d, 10/5d, 20/15d vs 30/30d — heating.
        n.commits_1d = 2;
        n.commits_5d = 10;
        n.commits_15d = 20;
        n.commits_30d = 30;
        assert!((n.commit_heat().unwrap() - 1.7778).abs() < 1e-4);
        // Flat: every window at its pro-rata share — steady 1.0.
        n.commits_1d = 1;
        n.commits_5d = 5;
        n.commits_15d = 15;
        assert!((n.commit_heat().unwrap() - 1.0).abs() < 1e-6);
        // No 30d baseline: no ratio (dormant when windows are 0 too).
        n.commits_1d = 0;
        n.commits_5d = 0;
        n.commits_15d = 0;
        n.commits_30d = 0;
        assert_eq!(n.commit_heat(), None);
        // Fresh (activity, no baseline): still None — check commits_5d.
        n.commits_5d = 3;
        assert_eq!(n.commit_heat(), None);
    }

    #[test]
    fn focus_penalizes_idle_months() {
        use chrono::TimeZone;
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap();
        let mut n = node("me/a", false, false, None);
        // Flat baseline, pushed today: 1.0 - 0 = 1.0.
        n.commits_1d = 1;
        n.commits_5d = 5;
        n.commits_15d = 15;
        n.commits_30d = 30;
        n.pushed_at = Some(now);
        assert!((n.focus(&now).unwrap() - 1.0).abs() < 1e-6);
        // Same velocity, quiet 60 days: 0.0 - 2.0 = -2.0.
        n.commits_1d = 0;
        n.commits_5d = 0;
        n.commits_15d = 0;
        n.commits_30d = 4;
        n.pushed_at = Some(now - chrono::Duration::days(60));
        assert!((n.focus(&now).unwrap() + 2.0).abs() < 1e-6);
        // Dormant 250 days, no baseline: 0.0 - 8.33 sinks deep.
        n.commits_5d = 0;
        n.commits_30d = 0;
        n.pushed_at = Some(now - chrono::Duration::days(250));
        assert!(n.focus(&now).unwrap() < -8.0);
        // Unknown push: no focus.
        n.pushed_at = None;
        assert_eq!(n.focus(&now), None);
    }

    #[test]
    fn focus_averages_commit_and_issue_heats() {
        use chrono::TimeZone;
        let now = chrono::Utc.with_ymd_and_hms(2026, 11, 8, 12, 0, 0).unwrap();
        // Commits-only repo: unchanged behavior — triage-free scores on
        // commits alone (flat 1.0, pushed today).
        let mut n = node("me/a", false, false, None);
        n.commits_1d = 1;
        n.commits_5d = 5;
        n.commits_15d = 15;
        n.commits_30d = 30;
        n.pushed_at = Some(now);
        assert!((n.focus(&now).unwrap() - 1.0).abs() < 1e-6);
        // Triage-only week (no commits, hot comment volume): scores on
        // issues alone — more comments, more focus.
        n.commits_1d = 0;
        n.commits_5d = 0;
        n.commits_15d = 0;
        n.commits_30d = 0;
        n.issue_comments_1d = 2;
        n.issue_comments_5d = 6;
        n.issue_comments_15d = 9;
        n.issue_comments_30d = 9;
        let ih = n.issue_heat().unwrap();
        assert!((n.focus(&now).unwrap() - ih).abs() < 1e-6);
        assert!(ih > 1.0, "recent-heavy commenting reads hot");
        // Open threads without comments log but never score: untouched
        // backlog is not focus.
        n.issue_comments_1d = 0;
        n.issue_comments_5d = 0;
        n.issue_comments_15d = 0;
        n.issue_comments_30d = 0;
        n.issues_open_30d = 9;
        assert_eq!(n.issue_heat(), None);
        assert!(n.focus(&now).unwrap().abs() < 1e-6);
        // Both hot: the mean, never the sum — no double-counted spike.
        n.commits_1d = 1;
        n.commits_5d = 5;
        n.commits_15d = 15;
        n.commits_30d = 30;
        n.issue_comments_1d = 2;
        n.issue_comments_5d = 6;
        n.issue_comments_15d = 9;
        n.issue_comments_30d = 9;
        let ch = n.commit_heat().unwrap();
        let ih = n.issue_heat().unwrap();
        assert!((n.focus(&now).unwrap() - (ch + ih) / 2.0).abs() < 1e-6);
    }
}
