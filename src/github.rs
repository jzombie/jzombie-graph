use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use reqwest_drive::{ClientBuilder, ClientWithMiddleware, DriveThrottleBackoff, ThrottlePolicy};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Minimal repo metadata from the REST API. `fork` rides along as fact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoMeta {
    pub full_name: String,
    pub owner: String,
    pub name: String,
    pub fork: bool,
    pub private: bool,
    pub default_branch: String,
    pub parent_full_name: Option<String>,
    /// Last push to the repo (API `pushed_at`). Display fact only, never scored.
    /// Free in the enumerate/get responses — zero extra calls.
    pub pushed_at: Option<DateTime<Utc>>,
    /// Repo-about facts (API-native, free in the same responses).
    /// Display only, never scored. Stripped with private nodes.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    // Tier-0 magnet signals: free in enumerate/get, display only, never scored.
    #[serde(default)]
    pub stars: u32,
    #[serde(default)]
    pub forks_count: u32,
    #[serde(default)]
    pub open_issues: u32,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub size_kb: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub name: String,
    pub sha: String,
}

/// Recursive tree listing plus the API `truncated` flag.
#[derive(Debug, Clone)]
pub struct TreeListing {
    pub paths: Vec<String>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiRepo {
    full_name: String,
    name: String,
    owner: ApiOwner,
    fork: bool,
    private: bool,
    default_branch: Option<String>,
    parent: Option<ApiParent>,
    pushed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    topics: Option<Vec<String>>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    // Tier-0 magnet signals: free in the same responses, display only.
    #[serde(default)]
    stargazers_count: u32,
    #[serde(default)]
    forks_count: u32,
    #[serde(default)]
    open_issues_count: u32,
    #[serde(default)]
    license: Option<ApiLicense>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    size: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiLicense {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    spdx_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiParent {
    full_name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiOwner {
    login: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiBranch {
    name: String,
    commit: ApiCommit,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiCommit {
    sha: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiContents {
    encoding: Option<String>,
    content: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiCompare {
    ahead_by: Option<u32>,
    #[serde(default)]
    total_commits: u32,
    #[serde(default)]
    commits: Vec<ApiCompareCommit>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiCompareCommit {
    sha: String,
    author: Option<ApiSimpleUser>,
    committer: Option<ApiSimpleUser>,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiSimpleUser {
    login: Option<String>,
}

/// Own-ahead commits: ahead commits authored (or committed) by the repo
/// owner. Pure function over one compare page — unit-tested directly.
fn own_ahead_commits<'a>(
    commits: impl IntoIterator<Item = &'a ApiCompareCommit>,
    owner: &str,
) -> usize {
    commits
        .into_iter()
        .filter(|c| is_owner_commit(c, owner))
        .count()
}

fn is_owner_commit(c: &ApiCompareCommit, owner: &str) -> bool {
    c.author
        .as_ref()
        .and_then(|a| a.login.as_deref())
        .or_else(|| c.committer.as_ref().and_then(|a| a.login.as_deref()))
        .is_some_and(|login| login.eq_ignore_ascii_case(owner))
}

/// Own-ahead divergence for forks: `(raw_ahead, owner_authored)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AheadBy {
    pub raw: u32,
    pub own: u32,
}

#[derive(Debug, Clone)]
pub struct GitHubClient {
    http: ClientWithMiddleware,
    token: String,
    api_base: String,
}

fn repo_meta_from_api(r: ApiRepo) -> RepoMeta {
    let license = r.license.as_ref().and_then(|l| {
        l.spdx_id
            .clone()
            .or_else(|| l.key.clone())
            .or_else(|| l.name.clone())
            .filter(|s| !s.is_empty() && s != "NOASSERTION")
    });
    RepoMeta {
        full_name: r.full_name,
        owner: r.owner.login,
        name: r.name,
        fork: r.fork,
        private: r.private,
        default_branch: r.default_branch.unwrap_or_else(|| "main".into()),
        parent_full_name: r.parent.map(|p| p.full_name),
        pushed_at: r.pushed_at,
        description: r.description,
        topics: r.topics.unwrap_or_default(),
        language: r.language,
        homepage: r.homepage.filter(|h| !h.is_empty()),
        stars: r.stargazers_count,
        forks_count: r.forks_count,
        open_issues: r.open_issues_count,
        license,
        archived: r.archived,
        disabled: r.disabled,
        size_kb: r.size,
    }
}

impl GitHubClient {
    pub fn new(token: String) -> Result<Self> {
        Self::with_base(token, "https://api.github.com".into())
    }

    pub fn with_base(token: String, api_base: String) -> Result<Self> {
        anyhow::ensure!(
            !token.is_empty(),
            "GitHub token is empty (set JZOMBIE_GRAPH_TOKEN or pass --allow-unauthenticated explicitly)"
        );
        Self::with_base_optional(token, api_base, false)
    }

    /// Empty token only when the caller explicitly opts in
    /// (`--allow-unauthenticated`). Never silent: default path still bails.
    pub fn with_base_optional(
        token: String,
        api_base: String,
        allow_unauthenticated: bool,
    ) -> Result<Self> {
        anyhow::ensure!(
            !token.is_empty() || allow_unauthenticated,
            "GitHub token is empty (set JZOMBIE_GRAPH_TOKEN or pass --allow-unauthenticated explicitly)"
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        if !token.is_empty() {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}"))
                    .context("token is not a valid header value")?,
            );
        }
        headers.insert(USER_AGENT, HeaderValue::from_static("jzombie-graph/0.1"));
        let inner = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(30))
            .build()
            .context("building HTTP client")?;
        // Reuse `rust-reqwest-drive` for retries, backoff, and concurrency
        // limiting (spec: no new client logic). Throttle-only mode — no cache
        // file; the store/privacy rule keeps all cache state out of this repo.
        let throttle = DriveThrottleBackoff::without_cache(ThrottlePolicy {
            base_delay_ms: 200,
            adaptive_jitter_ms: 100,
            max_concurrent: 5,
            max_retries: 3,
        });
        let http = ClientBuilder::new(inner).with(throttle).build();
        Ok(Self {
            http,
            token: token.clone(),
            api_base: api_base.trim_end_matches('/').to_string(),
        })
    }

    /// GET via the drive middleware (retries + exponential backoff on
    /// failures, concurrency-limited). Note: the middleware retries any
    /// non-2xx including 404s, so fail-open 404 probes (missing manifests)
    /// cost `1 + max_retries` calls worst-case — accounted in the budget
    /// estimate printed before fetching. A 404 that survives retries is
    /// returned here as an error; `file_content`/`list_dir` map it to None/empty.
    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self.http.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("{url}: HTTP {status}");
        }
        resp.json::<T>()
            .await
            .with_context(|| format!("decoding {url}"))
    }

    /// Enumerate everything owned by the user/org, forks included.
    /// Paginates (`per_page=100`) until a short page.
    pub async fn enumerate_repos(
        &self,
        user: Option<&str>,
        org: Option<&str>,
        include_forks: bool,
    ) -> Result<Vec<RepoMeta>> {
        let (kind, login) = match (user, org) {
            (Some(u), None) => ("users", u),
            (None, Some(o)) => ("orgs", o),
            _ => anyhow::bail!("enumerate needs exactly one of user/org"),
        };
        let mut out = vec![];
        let mut page = 1u32;
        loop {
            let url = format!(
                "{}/{}/{}/repos?per_page=100&page={page}&type=all&sort=full_name",
                self.api_base, kind, login
            );
            let batch: Vec<ApiRepo> = self.get_json(&url).await?;
            let n = batch.len();
            for r in batch {
                if r.fork && !include_forks {
                    continue;
                }
                out.push(repo_meta_from_api(r));
            }
            if n < 100 {
                break;
            }
            page += 1;
        }
        out.sort_by(|a, b| a.full_name.cmp(&b.full_name));
        Ok(out)
    }

    pub async fn get_repo(&self, full_name: &str) -> Result<RepoMeta> {
        let url = format!("{}/repos/{full_name}", self.api_base);
        let r: ApiRepo = self.get_json(&url).await?;
        Ok(repo_meta_from_api(r))
    }

    /// Recursive git tree (paths only). Used for workspace members
    /// (`crates/` subdirs collapse to the repo node) and use-site file discovery.
    /// Returns paths plus the API `truncated` flag — callers log truncation
    /// to the briefing digest (silent truncation hid workspace members).
    pub async fn tree_paths(&self, full_name: &str, sha: &str) -> Result<TreeListing> {
        #[derive(Deserialize)]
        struct Tree {
            tree: Vec<TreeEntry>,
            #[serde(default)]
            truncated: bool,
        }
        #[derive(Deserialize)]
        struct TreeEntry {
            path: String,
            #[serde(rename = "type")]
            _type: String,
        }
        let url = format!(
            "{}/repos/{full_name}/git/trees/{sha}?recursive=1",
            self.api_base
        );
        let t: Tree = self.get_json(&url).await?;
        Ok(TreeListing {
            paths: t.tree.into_iter().map(|e| e.path).collect(),
            truncated: t.truncated,
        })
    }

    /// Directory listing via contents API (workflows dir, etc.).
    /// Returns file paths relative to the dir. Empty on 404.
    pub async fn list_dir(&self, full_name: &str, dir: &str, branch: &str) -> Result<Vec<String>> {
        let url = format!(
            "{}/repos/{full_name}/contents/{dir}?ref={branch}",
            self.api_base
        );
        let resp = self.http.get(&url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }
        if !resp.status().is_success() {
            anyhow::bail!("{url}: HTTP {}", resp.status());
        }
        let v: serde_json::Value = resp.json().await?;
        let mut out = vec![];
        if let Some(arr) = v.as_array() {
            for item in arr {
                if item.get("type").and_then(|t| t.as_str()) == Some("file")
                    && let (Some(_name), Some(path)) = (
                        item.get("name").and_then(|n| n.as_str()),
                        item.get("path").and_then(|p| p.as_str()),
                    )
                {
                    out.push(path.to_string());
                }
            }
        }
        Ok(out)
    }

    /// Resolve one branch name to its head SHA. Used for the default branch
    /// only (collection is default-branch-only): the SHA feeds the single
    /// tree call that gates every existence probe. Fail-open to empty.
    pub async fn branch_sha(&self, full_name: &str, branch: &str) -> Result<String> {
        let url = format!("{}/repos/{full_name}/branches/{branch}", self.api_base);
        let b: ApiBranch = self.get_json(&url).await?;
        Ok(b.commit.sha)
    }

    pub async fn list_branches(&self, full_name: &str, max: usize) -> Result<Vec<Branch>> {
        let url = format!("{}/repos/{full_name}/branches?per_page=100", self.api_base);
        let mut branches: Vec<ApiBranch> = self.get_json(&url).await?;
        branches.truncate(max.max(1));
        Ok(branches
            .into_iter()
            .map(|b| Branch {
                name: b.name,
                sha: b.commit.sha,
            })
            .collect())
    }

    /// Fetch a file at a branch via the contents API. Returns None on 404
    /// (missing manifest/lockfile/doc — the normal fail-open case).
    /// Other errors propagate; callers treat them per-file, never fatal.
    pub async fn file_content(
        &self,
        full_name: &str,
        path: &str,
        branch: &str,
    ) -> Result<Option<String>> {
        let url = format!(
            "{}/repos/{full_name}/contents/{path}?ref={branch}",
            self.api_base
        );
        let resp = self.http.get(&url).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            anyhow::bail!("{url}: HTTP {}", resp.status());
        }
        let body: ApiContents = resp.json().await?;
        match (body.encoding.as_deref(), body.content) {
            (Some("base64"), Some(b)) => {
                let cleaned: String = b.chars().filter(|c| !c.is_whitespace()).collect();
                use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
                let bytes = B64
                    .decode(&cleaned)
                    .with_context(|| format!("base64 decoding {path}"))?;
                String::from_utf8(bytes)
                    .map(Some)
                    .context("file is not UTF-8")
            }
            (Some("none"), _) | (None, _) => Ok(None),
            (Some(enc), _) => anyhow::bail!("unsupported contents encoding: {enc}"),
        }
    }

    /// Own-ahead divergence for forks: how many of the fork's ahead commits
    /// were authored by the repo owner (`compare` API + authorship check).
    /// Base is the *parent default branch*, head is `owner:default_branch`.
    /// Returns None when not a fork or comparison is unavailable (fail-open:
    /// private/deleted upstream). Callers log the fail-open to the digest
    /// so diverged forks don't silently read as `ahead_by: null`.
    ///
    /// Raw `ahead_by` alone is not divergence: a fork frozen while upstream
    /// rewrote history shows ahead commits authored entirely by upstream
    /// (verified live: 46-ahead and 8-ahead forks with zero owner commits).
    /// Only owner-authored ahead commits count — anything else is a stale
    /// fork, not own work. Pages beyond the first are walked only when the
    /// compare truncates (`total_commits` exceeds the returned page);
    /// termination is by SHA-set (no new SHAs, error, empty, or a page cap),
    /// so an endpoint that ignores paging still terminates.
    pub async fn ahead_by(&self, repo: &RepoMeta) -> Result<Option<AheadBy>> {
        let Some(parent) = repo.parent_full_name.as_deref() else {
            return Ok(None);
        };
        if !repo.fork {
            return Ok(None);
        }
        // Parent default branch is required for a correct basehead;
        // the earlier URL omitted it (base was the repo, not a branch).
        let parent_default = match self.get_repo(parent).await {
            Ok(p) => p.default_branch,
            Err(_) => return Ok(None),
        };
        let base = format!(
            "{}/repos/{}/compare/{}...{}:{}",
            self.api_base, parent, parent_default, repo.owner, repo.default_branch
        );
        // First page: no paging params, same URL as before.
        let first: ApiCompare = match self.get_json(&base).await {
            Ok(c) => c,
            Err(_) => return Ok(None),
        };
        let raw = first.ahead_by.unwrap_or(0);
        if raw == 0 {
            return Ok(Some(AheadBy { raw: 0, own: 0 }));
        }
        // Walk pages until the ahead set is covered. Termination never
        // depends on paging support: empty pages, errors, repeated SHAs
        // (an endpoint ignoring ?page repeats page 1), and a page cap all
        // stop the walk. Uncovered remainder + no match yet = unknown.
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut own = 0u32;
        let mut total = first.total_commits.max(raw);
        let mut page = 1;
        let mut current = first;
        let mut complete = true;
        loop {
            let fresh: Vec<&ApiCompareCommit> = current
                .commits
                .iter()
                .filter(|c| !seen.contains(&c.sha))
                .collect();
            if fresh.is_empty() {
                complete = seen.len() as u32 >= total;
                break;
            }
            for c in &fresh {
                seen.insert(c.sha.clone());
            }
            own += own_ahead_commits(fresh, &repo.owner) as u32;
            if seen.len() as u32 >= total {
                break;
            }
            if page >= 10 {
                complete = false;
                break;
            }
            page += 1;
            let url = format!("{base}?per_page=100&page={page}");
            match self.get_json::<ApiCompare>(&url).await {
                Ok(c) => {
                    total = total.max(c.total_commits);
                    current = c;
                }
                Err(_) => {
                    complete = false;
                    break;
                }
            }
        }
        if !complete && own == 0 {
            // Truncated view, no owner commits seen: authorship unknown,
            // not proven-absent. Fail-open to hidden (same as unavailable),
            // callers log it via the digest note below.
            return Ok(None);
        }
        Ok(Some(AheadBy { raw, own }))
    }

    /// Trailing-window commit velocity (default branch): total commits
    /// with `since`, via `per_page=1` — the count comes off the
    /// `Link: rel="last"` page number, no pagination walks. Also returns
    /// the newest commit's date in the window (first item): whenever the
    /// window is non-empty that IS the latest default-branch commit.
    /// `since=None` means unbounded (latest commit overall). One call per
    /// window per repo. Empty repos (409) and errors are the caller's
    /// fail-open, never fatal here.
    pub async fn commit_count_since(
        &self,
        full_name: &str,
        branch: &str,
        since: Option<&DateTime<Utc>>,
    ) -> Result<(u32, Option<DateTime<Utc>>)> {
        let mut url = format!(
            "{}/repos/{full_name}/commits?sha={branch}&per_page=1",
            self.api_base
        );
        if let Some(s) = since {
            url.push_str("&since=");
            url.push_str(&s.to_rfc3339());
        }
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("{url}: HTTP {}", resp.status());
        }
        let link = resp
            .headers()
            .get(reqwest::header::LINK)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body: serde_json::Value = resp.json().await?;
        let items = body.as_array().cloned().unwrap_or_default();
        let n = parse_commit_total(link.as_deref(), items.len());
        Ok((n, latest_commit_date(&items)))
    }

    #[allow(dead_code)]
    pub fn token_len(&self) -> usize {
        self.token.len()
    }
}

/// Total commits from a `per_page=1` commits listing: the `rel="last"`
/// page number IS the count (one commit per page). No Link header means
/// a single short page — the body length (0 or 1) is the total.
pub fn parse_commit_total(link: Option<&str>, body_len: usize) -> u32 {
    if let Some(h) = link {
        for part in h.split(',') {
            let mut segs = part.split(';');
            let url = segs.next().unwrap_or("").trim();
            if !segs.any(|s| s.trim() == r#"rel="last""#) {
                continue;
            }
            let u = url.trim_start_matches('<').trim_end_matches('>');
            for q in u.split(&['?', '&'][..]) {
                if let Some(n) = q.strip_prefix("page=")
                    && let Ok(v) = n.parse::<u32>()
                {
                    return v;
                }
            }
        }
    }
    body_len.min(1) as u32
}

/// Newest commit date from a `per_page=1` commits listing: the first item
/// is the latest commit, preferring the committer date (what landed) with
/// author-date fallback. None when the window is empty.
pub fn latest_commit_date(items: &[serde_json::Value]) -> Option<DateTime<Utc>> {
    let commit = items.first()?.get("commit")?;
    let s = commit
        .get("committer")
        .and_then(|c| c.get("date"))
        .or_else(|| commit.get("author").and_then(|a| a.get("date")))?
        .as_str()?;
    s.parse::<DateTime<Utc>>().ok()
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD as B64};

    #[test]
    fn base64_vectors() {
        assert_eq!(B64.decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(B64.decode("TQ==").unwrap(), b"M");
        assert!(B64.decode("!!!").is_err());
    }

    #[test]
    fn commit_total_from_link_last_page() {
        use super::parse_commit_total;
        let link = r#"<https://api.github.com/repos/me/r/commits?since=x&per_page=1&page=2>; rel="next", <https://api.github.com/repos/me/r/commits?since=x&per_page=1&page=17>; rel="last""#;
        assert_eq!(parse_commit_total(Some(link), 1), 17);
        assert_eq!(parse_commit_total(None, 1), 1);
        assert_eq!(parse_commit_total(None, 0), 0);
        assert_eq!(parse_commit_total(Some(r#"<u?page=3>; rel="next""#), 1), 1);
    }

    #[test]
    fn commit_date_prefers_committer() {
        use super::latest_commit_date;
        let items = serde_json::json!([
            {"commit": {"committer": {"date": "2026-11-07T10:00:00Z"}, "author": {"date": "2026-11-06T10:00:00Z"}}}
        ]);
        let got = latest_commit_date(items.as_array().unwrap()).unwrap();
        assert_eq!(got.to_rfc3339(), "2026-11-07T10:00:00+00:00");
        let author_only = serde_json::json!([
            {"commit": {"author": {"date": "2026-11-06T10:00:00Z"}}}
        ]);
        let arr = author_only.as_array().unwrap();
        assert!(latest_commit_date(arr).is_some());
        assert_eq!(latest_commit_date(&[]), None);
    }

    #[test]
    fn own_ahead_counts_only_owner_commits() {
        use super::{ApiCompareCommit, ApiSimpleUser, own_ahead_commits};
        let commit = |login: Option<&str>| ApiCompareCommit {
            sha: format!("sha-{login:?}"),
            author: login.map(|l| ApiSimpleUser {
                login: Some(l.into()),
            }),
            committer: None,
        };
        // Mixed authorship: only the owner's commits count.
        // Case-insensitive login match; unlinked (null-author) never counts.
        let commits = vec![
            commit(Some("jzombie")),
            commit(Some("JZOMBIE")),
            commit(Some("someone-else")),
            commit(None),
        ];
        assert_eq!(own_ahead_commits(&commits, "jzombie"), 2);
        assert_eq!(own_ahead_commits(&[], "jzombie"), 0);
    }

    #[test]
    fn committer_login_counts_when_author_missing() {
        use super::{ApiCompareCommit, ApiSimpleUser, is_owner_commit};
        let c = ApiCompareCommit {
            sha: "abc".into(),
            author: None,
            committer: Some(ApiSimpleUser {
                login: Some("jzombie".into()),
            }),
        };
        assert!(is_owner_commit(&c, "jzombie"));
    }
}
