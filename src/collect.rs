use crate::graph::{Edge, EdgeKind};
use regex::Regex;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct ManifestDep {
    #[allow(dead_code)]
    pub package: String,
    pub optional: bool,
    pub declaring_members: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ManifestSet {
    /// package name -> dep (unioned across workspace members/branches by caller).
    pub deps: BTreeMap<String, ManifestDep>,
    pub workspace_members: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct CargoManifest {
    #[serde(default)]
    dependencies: HashMap<String, CargoDep>,
    #[serde(default, rename = "build-dependencies")]
    build_dependencies: HashMap<String, CargoDep>,
    #[serde(default)]
    workspace: Option<CargoWorkspace>,
    #[serde(default)]
    package: Option<CargoPackage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
#[allow(dead_code)]
enum CargoDep {
    Simple(String),
    Detailed(CargoDepDetailed),
}

#[derive(Debug, Clone, Deserialize)]
struct CargoDepDetailed {
    #[serde(default)]
    #[allow(dead_code)]
    version: Option<String>,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct CargoWorkspace {
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    dependencies: HashMap<String, CargoDep>,
}

#[derive(Debug, Clone, Deserialize)]
struct CargoPackage {
    #[serde(default)]
    #[allow(dead_code)]
    name: Option<String>,
}

/// Parse Cargo.toml: `[dependencies]` + `[build-dependencies]` +
/// `[workspace.dependencies]` (dev-deps excluded — test-only coupling
/// isn't architectural). Returns per-package optional flag.
pub fn parse_cargo_manifest(text: &str) -> anyhow::Result<ManifestSet> {
    let m: CargoManifest = toml::from_str(text)?;
    let mut deps: BTreeMap<String, ManifestDep> = BTreeMap::new();
    let mut insert = |name: String, optional: bool| {
        deps.entry(name.clone())
            .and_modify(|d: &mut ManifestDep| {
                d.optional = d.optional && optional;
                d.declaring_members += 1;
            })
            .or_insert(ManifestDep {
                package: name,
                optional,
                declaring_members: 1,
            });
    };
    for (name, dep) in m.dependencies {
        let optional = matches!(&dep, CargoDep::Detailed(d) if d.optional);
        // path-only deps are workspace-internal; skip (collapsed to repo node).
        let is_path = matches!(&dep, CargoDep::Detailed(d) if d.path.is_some());
        if !is_path {
            insert(normalize_pkg(&name), optional);
        }
    }
    for (name, dep) in m.build_dependencies {
        let optional = matches!(&dep, CargoDep::Detailed(d) if d.optional);
        let is_path = matches!(&dep, CargoDep::Detailed(d) if d.path.is_some());
        if !is_path {
            insert(normalize_pkg(&name), optional);
        }
    }
    let mut members = vec![];
    if let Some(ws) = m.workspace {
        for (name, dep) in ws.dependencies {
            let optional = matches!(&dep, CargoDep::Detailed(d) if d.optional);
            insert(normalize_pkg(&name), optional);
        }
        members = ws.members;
    }
    Ok(ManifestSet {
        deps,
        workspace_members: members,
    })
}

fn normalize_pkg(name: &str) -> String {
    name.trim().to_lowercase().replace('_', "-")
}

/// package.json: `dependencies` full weight; peer/optional -> 0.5 scope.
pub fn parse_package_json(text: &str) -> anyhow::Result<ManifestSet> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    let mut deps = BTreeMap::new();
    let mut insert = |obj: Option<&serde_json::Value>, optional: bool| {
        if let Some(serde_json::Value::Object(map)) = obj {
            for name in map.keys() {
                deps.insert(
                    normalize_pkg(name),
                    ManifestDep {
                        package: normalize_pkg(name),
                        optional,
                        declaring_members: 1,
                    },
                );
            }
        }
    };
    insert(v.get("dependencies"), false);
    insert(v.get("peerDependencies"), true);
    insert(v.get("optionalDependencies"), true);
    Ok(ManifestSet {
        deps,
        workspace_members: vec![],
    })
}

/// pyproject.toml / requirements.txt markers -> 0.5 scope on extras.
/// Minimal: PEP 508 `; extra ==` markers flag optional.
pub fn parse_pyproject(text: &str) -> anyhow::Result<ManifestSet> {
    let v: toml::Value = toml::from_str(text).unwrap_or(toml::Value::Table(Default::default()));
    let mut deps = BTreeMap::new();
    let mut push = |req: &str| {
        let optional = req.contains("extra ==") || req.contains("extra==");
        let pkg = req
            .split([' ', ';', '[', '>', '<', '=', '!', '~'])
            .next()
            .unwrap_or("")
            .trim()
            .to_lowercase()
            .replace('_', "-");
        if !pkg.is_empty() {
            deps.insert(
                pkg.clone(),
                ManifestDep {
                    package: pkg,
                    optional,
                    declaring_members: 1,
                },
            );
        }
    };
    if let Some(arr) = v
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for item in arr.iter().filter_map(|i| i.as_str()) {
            push(item);
        }
    }
    // poetry groups land here too; extras markers carry the 0.5 scope.
    if let Some(deps_tbl) = v
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for name in deps_tbl.keys() {
            if name != "python" {
                deps.insert(
                    normalize_pkg(name),
                    ManifestDep {
                        package: normalize_pkg(name),
                        optional: false,
                        declaring_members: 1,
                    },
                );
            }
        }
    }
    Ok(ManifestSet {
        deps,
        workspace_members: vec![],
    })
}

/// go.mod: every `require` counts 1.0 (no optional scope in Go).
pub fn parse_go_mod(text: &str) -> anyhow::Result<ManifestSet> {
    let mut deps = BTreeMap::new();
    let re = Regex::new(r"(?m)^\s*([a-zA-Z0-9._~/+-]+)\s+v[0-9]").unwrap();
    let in_block = text.contains("require (");
    if in_block {
        for cap in re.captures_iter(text) {
            let module = cap[1].to_string();
            // Last path segment is the package-ish name for alias mapping.
            let short = module.rsplit('/').next().unwrap_or(&module).to_lowercase();
            deps.insert(
                short.clone(),
                ManifestDep {
                    package: short,
                    optional: false,
                    declaring_members: 1,
                },
            );
        }
    } else {
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("require ") {
                let module = rest.split_whitespace().next().unwrap_or("");
                if !module.is_empty() {
                    let short = module.rsplit('/').next().unwrap_or(module).to_lowercase();
                    deps.insert(
                        short.clone(),
                        ManifestDep {
                            package: short,
                            optional: false,
                            declaring_members: 1,
                        },
                    );
                }
            }
        }
    }
    Ok(ManifestSet {
        deps,
        workspace_members: vec![],
    })
}

/// `.gitmodules` -> (path, url) pairs. URL-to-repo resolution happens in
/// edge building (github.com:owner/repo.git and https forms).
pub fn parse_gitmodules(text: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut path: Option<String> = None;
    let mut url: Option<String> = None;
    let flush =
        |path: &mut Option<String>, url: &mut Option<String>, out: &mut Vec<(String, String)>| {
            if let (Some(p), Some(u)) = (path.take(), url.take()) {
                out.push((p, u));
            }
        };
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("[submodule") {
            flush(&mut path, &mut url, &mut out);
        } else if let Some(rest) = t.strip_prefix("path") {
            path = Some(rest.trim().trim_start_matches('=').trim().to_string());
        } else if let Some(rest) = t.strip_prefix("url") {
            url = Some(rest.trim().trim_start_matches('=').trim().to_string());
        }
    }
    flush(&mut path, &mut url, &mut out);
    out
}

/// Normalize a git URL to `owner/repo` when it points at GitHub.
pub fn github_repo_from_url(url: &str) -> Option<String> {
    let u = url.trim().trim_end_matches(".git").trim_end_matches('/');
    for prefix in [
        "https://github.com/",
        "http://github.com/",
        "git@github.com:",
        "ssh://git@github.com/",
    ] {
        if let Some(rest) = u.strip_prefix(prefix) {
            let rest = rest.strip_prefix('/').unwrap_or(rest);
            let mut it = rest.split('/');
            if let (Some(o), Some(r)) = (it.next(), it.next())
                && !o.is_empty()
                && !r.is_empty()
            {
                return Some(format!("{o}/{}", r.split('/').next().unwrap_or(r)));
            }
        }
    }
    None
}

fn uses_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)uses:\s*([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)").unwrap())
}

/// Workflow scan: `uses: owner/repo` (+ sibling checkouts/matrix refs are
/// caller's job — this returns the raw owner/repo tokens).
pub fn scan_workflow_uses(text: &str) -> Vec<String> {
    uses_re()
        .captures_iter(text)
        .map(|c| c[1].to_string())
        .collect()
}

fn md_link_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"https?://github\.com/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)").unwrap())
}

/// Docs scan: README/llms.txt links to `github.com/owner/repo`.
pub fn scan_docs_links(text: &str) -> Vec<String> {
    md_link_re()
        .captures_iter(text)
        .map(|c| c[1].trim_end_matches(".git").to_string())
        .collect()
}

// ---- use-site check (Rust via syn; regex fallback elsewhere) ----

fn ident_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap())
}

/// Rust use-site check: true when any `use` item (incl. `pub use`)
/// references the crate, or any path segment matches. Unparseable file =
/// match (safe direction: keep the edge). Every nullification is logged
/// by the caller in the briefing digest.
pub fn rust_crate_used(crate_name: &str, rs_source: &str) -> bool {
    let file = match syn::parse_file(rs_source) {
        Ok(f) => f,
        Err(_) => return true,
    };
    let target = crate_name.replace('-', "_");
    for item in &file.items {
        if let syn::Item::Use(u) = item
            && use_tree_refers(&u.tree, &target)
        {
            return true;
        }
    }
    // Also catch fully-qualified paths (`foo::bar`) outside `use` items:
    // cheap token scan over the whole file as backup within the AST path.
    for m in ident_re().find_iter(rs_source) {
        if m.as_str().replace('-', "_") == target {
            return true;
        }
    }
    false
}

fn use_tree_refers(tree: &syn::UseTree, target: &str) -> bool {
    match tree {
        syn::UseTree::Path(p) => p.ident == target || use_tree_refers(&p.tree, target),
        syn::UseTree::Name(n) => n.ident == target,
        syn::UseTree::Rename(r) => r.ident == target,
        syn::UseTree::Glob(_) => false,
        syn::UseTree::Group(g) => g.items.iter().any(|t| use_tree_refers(t, target)),
    }
}

/// Regex identifier scan — documented fallback for all ecosystems
/// (and the primary scan for Node/Python until their AST walkers land).
pub fn regex_identifier_used(crate_name: &str, source: &str) -> bool {
    let variants = [
        crate_name.to_string(),
        crate_name.replace('-', "_"),
        crate_name.replace('_', "-"),
    ];
    let re_str = format!(r"(?m)\b({})\b", variants.join("|").replace('-', "\\-"));
    Regex::new(&re_str)
        .map(|re| re.is_match(source))
        .unwrap_or(true)
}

// ---- edge building ----

pub struct BranchDecl {
    pub branch: String,
    pub recency: Option<chrono::DateTime<chrono::Utc>>,
}

/// Build manifest edges for one repo: alias-map package -> portfolio repo,
/// apply scope (optional 0.5), use-site Boolean, lockfile gate.
///
/// - `lockfile: None` -> no lockfile, manifest suffices.
/// - `lockfile: Some(set)` -> package present confirms; absent deletes edge.
/// - `use_sites: package -> referenced?` (None = no sources scanned -> match).
#[allow(clippy::too_many_arguments)]
pub fn build_manifest_edges(
    source_repo: &str,
    branches: &[BranchDecl],
    manifest: &ManifestSet,
    alias_map: &HashMap<String, String>,
    portfolio: &BTreeSet<String>,
    lockfile: Option<&BTreeSet<String>>,
    use_sites: &HashMap<String, bool>,
    lock_versions: &HashMap<String, String>,
) -> Vec<Edge> {
    let mut out = vec![];
    for dep in manifest.deps.values() {
        let Some(target) = alias_map.get(&normalize_pkg(&dep.package)) else {
            continue;
        };
        if !portfolio.contains(target) {
            continue;
        }
        if source_repo == target {
            continue; // self-dep (workspace root alias) is not a portfolio edge
        }
        // Lockfile gate.
        if let Some(locked) = lockfile
            && !locked.contains(&normalize_pkg(&dep.package))
        {
            continue; // absent-with-lockfile: unresolvable, delete edge
        }
        // Use-site Boolean: zero references nullifies (phantom dep).
        let referenced = *use_sites.get(&normalize_pkg(&dep.package)).unwrap_or(&true);
        if !referenced {
            continue; // caller logs the nullification in the digest
        }
        let scope: f32 = if dep.optional { 0.5 } else { 1.0 };
        let weight = (dep.declaring_members.max(1) as f32) * scope;
        let recency = branches.iter().filter_map(|b| b.recency).max();
        out.push(Edge {
            source: source_repo.into(),
            target: target.clone(),
            kind: EdgeKind::Manifest,
            declaring_branches: branches.iter().map(|b| b.branch.clone()).collect(),
            recency,
            weight,
            version: lock_versions.get(&normalize_pkg(&dep.package)).cloned(),
            note: None,
        });
    }
    // Deterministic: one edge per (source, target) — callers union branches.
    out.sort_by(|a, b| (&a.source, &a.target).cmp(&(&b.source, &b.target)));
    out.dedup_by(|a, b| a.source == b.source && a.target == b.target);
    out
}

/// Parse a lockfile's package-name set (gate input). Supports Cargo.lock
/// (`name = "..."`) and package-lock (`"node_modules/<name>"` keys +
/// `"packages"` map). Unknown formats -> None (manifest suffices).
pub fn parse_lockfile_names(filename: &str, text: &str) -> Option<BTreeSet<String>> {
    if filename.ends_with("Cargo.lock") {
        let mut set = BTreeSet::new();
        for line in text.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("name = ") {
                let name = rest.trim().trim_matches('"').to_string();
                if !name.is_empty() {
                    set.insert(normalize_pkg(&name));
                }
            }
        }
        return Some(set);
    }
    if filename.ends_with("package-lock.json") {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let mut set = BTreeSet::new();
        if let Some(deps) = v.get("dependencies").and_then(|d| d.as_object()) {
            set.extend(deps.keys().map(|k| normalize_pkg(k)));
        }
        if let Some(pkgs) = v.get("packages").and_then(|p| p.as_object()) {
            for key in pkgs.keys() {
                if let Some(name) = key.strip_prefix("node_modules/")
                    && !name.contains("node_modules/")
                    && !name.is_empty()
                {
                    set.insert(normalize_pkg(name));
                }
            }
        }
        return Some(set);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_parses_deps_build_excludes_dev() {
        let text = r#"
[package]
name = "demo"

[dependencies]
serde = "1"
tokio = { version = "1", optional = true }
local = { path = "../local" }

[dev-dependencies]
criterion = "0.5"

[build-dependencies]
cc = "1"
"#;
        let m = parse_cargo_manifest(text).unwrap();
        assert!(m.deps.contains_key("serde"));
        assert!(m.deps.contains_key("tokio"));
        assert!(m.deps["tokio"].optional);
        assert!(
            !m.deps.contains_key("local"),
            "path dep is workspace-internal"
        );
        assert!(!m.deps.contains_key("criterion"), "dev-deps excluded");
        assert!(m.deps.contains_key("cc"));
    }

    #[test]
    fn cargo_workspace_deps_counted() {
        let text = r#"
[workspace]
members = ["crates/a"]

[workspace.dependencies]
serde = "1"
"#;
        let m = parse_cargo_manifest(text).unwrap();
        assert!(m.deps.contains_key("serde"));
        assert_eq!(m.workspace_members, vec!["crates/a".to_string()]);
    }

    #[test]
    fn gitmodules_and_url_forms() {
        let text = "[submodule \"x\"]\n\tpath = vendor/x\n\turl = https://github.com/me/x.git\n";
        let pairs = parse_gitmodules(text);
        assert_eq!(pairs.len(), 1);
        assert_eq!(github_repo_from_url(&pairs[0].1).as_deref(), Some("me/x"));
        assert_eq!(
            github_repo_from_url("git@github.com:me/y"),
            Some("me/y".to_string())
        );
    }

    #[test]
    fn workflow_and_docs_scans() {
        let wf = "steps:\n - uses: actions/checkout@v4\n - uses: me/reusable/.github/workflows/x.yml@v1\n";
        let uses = scan_workflow_uses(wf);
        assert!(uses.contains(&"actions/checkout".to_string()));
        let docs = "see https://github.com/me/sibling for details";
        assert_eq!(scan_docs_links(docs), vec!["me/sibling".to_string()]);
    }

    #[test]
    fn rust_use_site_and_unparseable_is_match() {
        assert!(rust_crate_used("serde", "use serde::Serialize; fn f() {}"));
        assert!(rust_crate_used("tokio", "pub use tokio::fs;"));
        assert!(!rust_crate_used("serde", "fn main() { println!(\"hi\"); }"));
        assert!(rust_crate_used("serde", "this is not rust {{{"));
    }

    #[test]
    fn manifest_edges_weight_gate_and_use_site() {
        let mut manifest = ManifestSet::default();
        manifest.deps.insert(
            "foo".into(),
            ManifestDep {
                package: "foo".into(),
                optional: true,
                declaring_members: 3,
            },
        );
        manifest.deps.insert(
            "phantom".into(),
            ManifestDep {
                package: "phantom".into(),
                optional: false,
                declaring_members: 1,
            },
        );
        let aliases: HashMap<String, String> = [
            ("foo".into(), "me/lib".into()),
            ("phantom".into(), "me/other".into()),
        ]
        .into_iter()
        .collect();
        let portfolio: BTreeSet<String> = ["me/app".into(), "me/lib".into(), "me/other".into()]
            .into_iter()
            .collect();
        let branches = vec![BranchDecl {
            branch: "main".into(),
            recency: None,
        }];
        let use_sites: HashMap<String, bool> = [("foo".into(), true), ("phantom".into(), false)]
            .into_iter()
            .collect();
        let edges = build_manifest_edges(
            "me/app",
            &branches,
            &manifest,
            &aliases,
            &portfolio,
            None,
            &use_sites,
            &HashMap::new(),
        );
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].weight, 3.0 * 0.5);
        // Absent-with-lockfile deletes the edge.
        let locked: BTreeSet<String> = ["bar".into()].into_iter().collect();
        let edges2 = build_manifest_edges(
            "me/app",
            &branches,
            &manifest,
            &aliases,
            &portfolio,
            Some(&locked),
            &HashMap::new(),
            &HashMap::new(),
        );
        assert!(edges2.is_empty());
    }

    #[test]
    fn lockfile_parsers() {
        let cargo = "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n";
        let set = parse_lockfile_names("Cargo.lock", cargo).unwrap();
        assert!(set.contains("serde"));
        let pl = r#"{"dependencies": {"left-pad": {"version": "1.0.0"}}}"#;
        let set2 = parse_lockfile_names("package-lock.json", pl).unwrap();
        assert!(set2.contains("left-pad"));
    }
}
