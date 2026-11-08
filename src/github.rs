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
}

#[derive(Debug, Clone)]
pub struct GitHubClient {
    http: ClientWithMiddleware,
    token: String,
    api_base: String,
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
                out.push(RepoMeta {
                    full_name: r.full_name,
                    owner: r.owner.login,
                    name: r.name,
                    fork: r.fork,
                    private: r.private,
                    default_branch: r.default_branch.unwrap_or_else(|| "main".into()),
                    parent_full_name: r.parent.map(|p| p.full_name),
                    pushed_at: r.pushed_at,
                });
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
        Ok(RepoMeta {
            full_name: r.full_name,
            owner: r.owner.login,
            name: r.name,
            fork: r.fork,
            private: r.private,
            default_branch: r.default_branch.unwrap_or_else(|| "main".into()),
            parent_full_name: r.parent.map(|p| p.full_name),
            pushed_at: r.pushed_at,
        })
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

    /// Commits ahead of upstream (`compare` API) — fork divergence rule (a).
    /// Base is the *parent default branch*, head is `owner:default_branch`.
    /// Returns None when not a fork or comparison is unavailable (fail-open:
    /// private/deleted upstream). Callers log the fail-open to the digest
    /// so diverged forks don't silently read as `ahead_by: null`.
    pub async fn ahead_by(&self, repo: &RepoMeta) -> Result<Option<u32>> {
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
        let url = format!(
            "{}/repos/{}/compare/{}...{}:{}",
            self.api_base, parent, parent_default, repo.owner, repo.default_branch
        );
        // Compare failures (private upstream, deleted parent) are fail-open: None.
        match self.get_json::<ApiCompare>(&url).await {
            Ok(c) => Ok(c.ahead_by),
            Err(_) => Ok(None),
        }
    }

    #[allow(dead_code)]
    pub fn token_len(&self) -> usize {
        self.token.len()
    }
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
}
