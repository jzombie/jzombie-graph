use anyhow::{Context, Result};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub name: String,
    pub sha: String,
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
    http: reqwest::Client,
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
            "GitHub token is empty (set GITHUB_TOKEN)"
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .context("token is not a valid header value")?,
        );
        headers.insert(USER_AGENT, HeaderValue::from_static("jzombie-graph/0.1"));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(30))
            .build()
            .context("building HTTP client")?;
        Ok(Self {
            http,
            token: token.clone(),
            api_base: api_base.trim_end_matches('/').to_string(),
        })
    }

    /// GET with retries + exponential backoff on 429/5xx (the failure modes
    /// the spec assigns to Rust, not to DSL/bash).
    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> Result<T> {
        let mut delay = Duration::from_millis(500);
        let mut last_err = None;
        for attempt in 0..5 {
            let resp = self.http.get(url).send().await;
            match resp {
                Ok(r) => {
                    let status = r.status();
                    if status.is_success() {
                        return r
                            .json::<T>()
                            .await
                            .with_context(|| format!("decoding {url}"));
                    }
                    if status.as_u16() == 429 || status.is_server_error() {
                        last_err =
                            Some(anyhow::anyhow!("{url}: HTTP {status} (attempt {attempt})"));
                    } else {
                        anyhow::bail!("{url}: HTTP {status}");
                    }
                }
                Err(e) => last_err = Some(anyhow::anyhow!("{url}: {e} (attempt {attempt})")),
            }
            tokio::time::sleep(delay).await;
            delay *= 2;
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("{url}: request failed")))
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
        })
    }

    /// Recursive git tree (paths only). Used for workspace members
    /// (`crates/` subdirs collapse to the repo node) and use-site file discovery.
    pub async fn tree_paths(&self, full_name: &str, sha: &str) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Tree {
            tree: Vec<TreeEntry>,
            #[serde(default)]
            #[allow(dead_code)]
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
        Ok(t.tree.into_iter().map(|e| e.path).collect())
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
                    && let (Some(name), Some(path)) = (
                        item.get("name").and_then(|n| n.as_str()),
                        item.get("path").and_then(|p| p.as_str()),
                    )
                {
                    out.push(path.to_string());
                    let _ = name;
                }
            }
        }
        Ok(out)
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

    /// Fetch a file at a branch via the contents API. Returns None on 404.
    /// (404 surfaces as an error from `get_json`; so this uses a direct call.)
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
                // Base64 without a dedicated crate: minimal inline decoder.
                let bytes = base64_decode(&cleaned)?;
                String::from_utf8(bytes)
                    .map(Some)
                    .context("file is not UTF-8")
            }
            (Some("none"), _) | (None, _) => Ok(None),
            (Some(enc), _) => anyhow::bail!("unsupported contents encoding: {enc}"),
        }
    }

    /// Commits ahead of upstream (`compare` API) — fork divergence rule (a).
    /// Returns None when not a fork or comparison is unavailable.
    pub async fn ahead_by(&self, repo: &RepoMeta) -> Result<Option<u32>> {
        let Some(parent) = repo.parent_full_name.as_deref() else {
            return Ok(None);
        };
        if !repo.fork {
            return Ok(None);
        }
        let url = format!(
            "{}/repos/{}/compare/{}...{}:{}",
            self.api_base, parent, parent, repo.owner, repo.default_branch
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

fn base64_decode(s: &str) -> Result<Vec<u8>> {
    fn val(c: u8) -> Result<u8> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            b'=' => Ok(64), // padding
            _ => anyhow::bail!("invalid base64 char"),
        }
    }
    let bytes = s.as_bytes();
    anyhow::ensure!(bytes.len().is_multiple_of(4), "bad base64 length");
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut n = [0u8; 4];
        for (i, &c) in chunk.iter().enumerate() {
            n[i] = val(c)?;
        }
        out.push((n[0] << 2) | (n[1] >> 4));
        if n[2] != 64 {
            out.push(((n[1] & 15) << 4) | (n[2] >> 2));
        }
        if n[3] != 64 {
            out.push(((n[2] & 3) << 6) | n[3]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip_vectors() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("TQ==").unwrap(), b"M");
        assert!(base64_decode("!!!").is_err());
    }
}
