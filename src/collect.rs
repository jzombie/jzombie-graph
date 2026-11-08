use crate::graph::{Edge, EdgeKind};
use regex::Regex;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct ManifestDep {
    pub package: String,
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
    path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct CargoWorkspace {
    #[serde(default)]
    members: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CargoPackage {
    #[serde(default)]
    #[allow(dead_code)]
    name: Option<String>,
}

/// Parse Cargo.toml: `[dependencies]` + `[build-dependencies]`
/// (dev-deps excluded — test-only coupling isn't architectural).
/// One manifest = one consumer: a package counts once no matter how many
/// sections name it. A `[workspace.dependencies]` pin plus a
/// `.workspace = true` use is a single relationship, not two; pins alone
/// (never used by this manifest) count as zero. Presence-union across
/// manifests (root + members) is the caller's job.
pub fn parse_cargo_manifest(text: &str) -> anyhow::Result<ManifestSet> {
    let m: CargoManifest = toml::from_str(text)?;
    let mut deps: BTreeMap<String, ManifestDep> = BTreeMap::new();
    let mut insert = |name: String| {
        deps.entry(name.clone())
            .or_insert(ManifestDep { package: name });
    };
    for (name, dep) in m.dependencies {
        // path-only deps are workspace-internal; skip (collapsed to repo node).
        let is_path = matches!(&dep, CargoDep::Detailed(d) if d.path.is_some());
        if !is_path {
            insert(normalize_pkg(&name));
        }
    }
    for (name, dep) in m.build_dependencies {
        let is_path = matches!(&dep, CargoDep::Detailed(d) if d.path.is_some());
        if !is_path {
            insert(normalize_pkg(&name));
        }
    }
    let mut members = vec![];
    if let Some(ws) = m.workspace {
        // Workspace pins are version pins, not use — the manifests that
        // inherit them declare their own use. Never counted here.
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

/// Registry/package names for alias-table generation (machine-generated,
/// never hand-edited). Each returns the normalized name, if the manifest
/// declares one. Workspace-only roots declare none (members carry names).
pub fn cargo_package_name(text: &str) -> Option<String> {
    let v: toml::Value = toml::from_str(text).ok()?;
    v.get("package")?.get("name")?.as_str().map(normalize_pkg)
}

/// package.json `name` (npm scope prefix preserved: `@scope/pkg`).
pub fn package_json_name(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    v.get("name")?.as_str().map(normalize_pkg)
}

/// pyproject `[project] name`, else `[tool.poetry] name`.
pub fn pyproject_name(text: &str) -> Option<String> {
    let v: toml::Value = toml::from_str(text).ok()?;
    v.get("project")
        .and_then(|p| p.get("name"))
        .or_else(|| v.get("tool")?.get("poetry")?.get("name"))
        .and_then(|n| n.as_str())
        .map(normalize_pkg)
}

/// go.mod `module` line, short (last path segment) form for alias mapping.
pub fn go_mod_name(text: &str) -> Option<String> {
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("module ") {
            let module = rest.split_whitespace().next().unwrap_or("");
            if !module.is_empty() {
                return Some(module.rsplit('/').next().unwrap_or(module).to_lowercase());
            }
        }
    }
    None
}

/// Registry of manifest kinds. Adding an ecosystem = one row here.
/// Both passes (provider discovery, structure collection) read this table —
/// no per-ecosystem filename branches anywhere else.
pub struct ManifestKind {
    /// Root filename (`Cargo.toml`, `package.json`, ...).
    pub file: &'static str,
    /// Full manifest -> dep set (structure pass).
    pub parse: fn(&str) -> anyhow::Result<ManifestSet>,
    /// Manifest -> provided package name (provider pass).
    pub package_name: fn(&str) -> Option<String>,
    /// Whether member manifests of this kind are walked (Cargo workspaces).
    pub walk_members: bool,
}

pub static MANIFEST_KINDS: &[ManifestKind] = &[
    ManifestKind {
        file: "Cargo.toml",
        parse: parse_cargo_manifest,
        package_name: cargo_package_name,
        walk_members: true,
    },
    ManifestKind {
        file: "package.json",
        parse: parse_package_json,
        package_name: package_json_name,
        walk_members: false,
    },
    ManifestKind {
        file: "pyproject.toml",
        parse: parse_pyproject,
        package_name: pyproject_name,
        walk_members: false,
    },
    ManifestKind {
        file: "go.mod",
        parse: parse_go_mod,
        package_name: go_mod_name,
        walk_members: false,
    },
];

/// Root manifest? Exact filename hit in the registry.
pub fn manifest_kind_for_root(path: &str) -> Option<&'static ManifestKind> {
    MANIFEST_KINDS.iter().find(|k| k.file == path)
}

/// Member manifest of a walked kind (`<dir>/Cargo.toml`, never the root)?
pub fn manifest_kind_for_member(path: &str) -> Option<&'static ManifestKind> {
    MANIFEST_KINDS.iter().find(|k| {
        k.walk_members
            && path
                .strip_suffix(k.file)
                .is_some_and(|pre| pre.ends_with('/'))
    })
}

/// package.json: `dependencies` + peer + optional, all uniform presence.
/// No scope distinctions: they aren't comparable across ecosystems.
pub fn parse_package_json(text: &str) -> anyhow::Result<ManifestSet> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    let mut deps = BTreeMap::new();
    let mut insert = |obj: Option<&serde_json::Value>| {
        if let Some(serde_json::Value::Object(map)) = obj {
            for name in map.keys() {
                deps.insert(
                    normalize_pkg(name),
                    ManifestDep {
                        package: normalize_pkg(name),
                    },
                );
            }
        }
    };
    insert(v.get("dependencies"));
    insert(v.get("peerDependencies"));
    insert(v.get("optionalDependencies"));
    Ok(ManifestSet {
        deps,
        workspace_members: vec![],
    })
}

/// pyproject.toml: `[project] dependencies` + poetry deps, uniform presence.
/// No extras-marker heuristics: they aren't reliable enough to score on.
pub fn parse_pyproject(text: &str) -> anyhow::Result<ManifestSet> {
    let v: toml::Value = toml::from_str(text).unwrap_or(toml::Value::Table(Default::default()));
    let mut deps = BTreeMap::new();
    let mut push = |req: &str| {
        let pkg = req
            .split([' ', ';', '[', '>', '<', '=', '!', '~'])
            .next()
            .unwrap_or("")
            .trim()
            .to_lowercase()
            .replace('_', "-");
        if !pkg.is_empty() {
            deps.insert(pkg.clone(), ManifestDep { package: pkg });
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
    // poetry groups land here too, same uniform presence.
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

/// go.mod: every `require` is a dep, uniform presence.
pub fn parse_go_mod(text: &str) -> anyhow::Result<ManifestSet> {
    let mut deps = BTreeMap::new();
    let re = Regex::new(r"(?m)^\s*([a-zA-Z0-9._~/+-]+)\s+v[0-9]").unwrap();
    let in_block = text.contains("require (");
    if in_block {
        for cap in re.captures_iter(text) {
            let module = cap[1].to_string();
            // Last path segment is the package-ish name for alias mapping.
            let short = module.rsplit('/').next().unwrap_or(&module).to_lowercase();
            deps.insert(short.clone(), ManifestDep { package: short });
        }
    } else {
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("require ") {
                let module = rest.split_whitespace().next().unwrap_or("");
                if !module.is_empty() {
                    let short = module.rsplit('/').next().unwrap_or(module).to_lowercase();
                    deps.insert(short.clone(), ManifestDep { package: short });
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

/// Build manifest edges for one repo: providers map package -> portfolio
/// repos. Presence is binary: the dep is declared or it isn't — no scope,
/// no per-manifest counting (those measured manifest style, not architecture).
///
/// - `lockfile: None` -> no lockfile, manifest suffices.
/// - `lockfile: Some(set)` -> package present confirms; absent deletes edge.
/// - `use_sites: package -> referenced?` (None = no sources scanned -> match).
/// - Ambiguous packages (2+ providers): no edge — asserting one would be a
///   coin flip. Recorded in `ambiguities` (package -> candidate repos) for
///   the caller's digest; never silently dropped.
#[allow(clippy::too_many_arguments)]
pub fn build_manifest_edges(
    source_repo: &str,
    branches: &[BranchDecl],
    manifest: &ManifestSet,
    providers: &HashMap<String, Vec<String>>,
    portfolio: &BTreeSet<String>,
    lockfile: Option<&BTreeSet<String>>,
    use_sites: &HashMap<String, bool>,
    lock_versions: &HashMap<String, String>,
    ambiguities: &mut Vec<(String, Vec<String>)>,
) -> Vec<Edge> {
    let mut out = vec![];
    for dep in manifest.deps.values() {
        let pkg = normalize_pkg(&dep.package);
        let Some(targets) = providers.get(&pkg) else {
            continue; // external package, not a portfolio edge
        };
        // Providers are portfolio repos by construction; belt-and-suspenders.
        let targets: Vec<&String> = targets.iter().filter(|t| portfolio.contains(*t)).collect();
        if targets.is_empty() {
            continue;
        }
        if targets.len() > 1 {
            let mut names: Vec<String> = targets.iter().map(|t| (*t).clone()).collect();
            names.sort();
            names.dedup();
            if !ambiguities.iter().any(|(p, _)| *p == pkg) {
                ambiguities.push((pkg, names));
            }
            continue; // ambiguous: no edge rather than a wrong edge
        }
        let target = targets[0];
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
        let referenced = *use_sites.get(&pkg).unwrap_or(&true);
        if !referenced {
            continue; // caller logs the nullification in the digest
        }
        let recency = branches.iter().filter_map(|b| b.recency).max();
        out.push(Edge {
            source: source_repo.into(),
            target: target.clone(),
            kind: EdgeKind::Manifest,
            declaring_branches: branches.iter().map(|b| b.branch.clone()).collect(),
            recency,
            version: lock_versions.get(&pkg).cloned(),
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
tokio = "1"
local = { path = "../local" }

[dev-dependencies]
criterion = "0.5"

[build-dependencies]
cc = "1"
"#;
        let m = parse_cargo_manifest(text).unwrap();
        assert!(m.deps.contains_key("serde"));
        assert!(m.deps.contains_key("tokio"));
        assert!(
            !m.deps.contains_key("local"),
            "path dep is workspace-internal"
        );
        assert!(!m.deps.contains_key("criterion"), "dev-deps excluded");
        assert!(m.deps.contains_key("cc"));
    }

    #[test]
    fn cargo_workspace_pins_are_not_use() {
        let text = r#"
[workspace]
members = ["crates/a"]

[workspace.dependencies]
serde = "1"
"#;
        let m = parse_cargo_manifest(text).unwrap();
        assert!(
            !m.deps.contains_key("serde"),
            "pins are versions, not use — members declare their own"
        );
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
    fn manifest_edges_gate_and_use_site() {
        let mut manifest = ManifestSet::default();
        manifest.deps.insert(
            "foo".into(),
            ManifestDep {
                package: "foo".into(),
            },
        );
        manifest.deps.insert(
            "phantom".into(),
            ManifestDep {
                package: "phantom".into(),
            },
        );
        let providers: HashMap<String, Vec<String>> = [
            ("foo".into(), vec!["me/lib".into()]),
            ("phantom".into(), vec!["me/other".into()]),
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
        let mut ambiguities = vec![];
        let edges = build_manifest_edges(
            "me/app",
            &branches,
            &manifest,
            &providers,
            &portfolio,
            None,
            &use_sites,
            &HashMap::new(),
            &mut ambiguities,
        );
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target, "me/lib");
        assert!(ambiguities.is_empty());
        // Absent-with-lockfile deletes the edge.
        let locked: BTreeSet<String> = ["bar".into()].into_iter().collect();
        let edges2 = build_manifest_edges(
            "me/app",
            &branches,
            &manifest,
            &providers,
            &portfolio,
            Some(&locked),
            &HashMap::new(),
            &HashMap::new(),
            &mut ambiguities,
        );
        assert!(edges2.is_empty());
    }

    #[test]
    fn ambiguous_package_yields_no_edge_but_records() {
        let mut manifest = ManifestSet::default();
        manifest.deps.insert(
            "dup".into(),
            ManifestDep {
                package: "dup".into(),
            },
        );
        let providers: HashMap<String, Vec<String>> =
            [("dup".into(), vec!["me/a".into(), "me/b".into()])]
                .into_iter()
                .collect();
        let portfolio: BTreeSet<String> = ["me/app".into(), "me/a".into(), "me/b".into()]
            .into_iter()
            .collect();
        let branches = vec![BranchDecl {
            branch: "main".into(),
            recency: None,
        }];
        let mut ambiguities = vec![];
        let edges = build_manifest_edges(
            "me/app",
            &branches,
            &manifest,
            &providers,
            &portfolio,
            None,
            &HashMap::new(),
            &HashMap::new(),
            &mut ambiguities,
        );
        assert!(edges.is_empty(), "ambiguous: no edge, never a wrong one");
        assert_eq!(
            ambiguities,
            vec![("dup".into(), vec!["me/a".into(), "me/b".into()])]
        );
    }

    #[test]
    fn registry_package_names() {
        let cargo = "[package]\nname = \"reqwest-drive\"\nversion = \"0.1.0\"\n";
        assert_eq!(cargo_package_name(cargo).as_deref(), Some("reqwest-drive"));
        let ws = "[workspace]\nmembers = [\"crates/a\"]\n";
        assert_eq!(cargo_package_name(ws), None);
        let npm = r#"{"name": "@scope/pkg", "version": "1.0.0"}"#;
        assert_eq!(package_json_name(npm).as_deref(), Some("@scope/pkg"));
        let py = "[project]\nname = \"my-pkg\"\nversion = \"1\"\n";
        assert_eq!(pyproject_name(py).as_deref(), Some("my-pkg"));
        let poetry = "[tool.poetry]\nname = \"poetry-pkg\"\n";
        assert_eq!(pyproject_name(poetry).as_deref(), Some("poetry-pkg"));
        let go = "module github.com/me/my-mod\n\ngo 1.21\n";
        assert_eq!(go_mod_name(go).as_deref(), Some("my-mod"));
        let go_single = "module my-tool\n";
        assert_eq!(go_mod_name(go_single).as_deref(), Some("my-tool"));
    }

    #[test]
    fn cargo_workspace_inheritance_counts_once() {
        // Pin + `.workspace = true` use in one manifest: one relationship.
        let text = "[package]\nname = \"demo\"\n[workspace.dependencies]\nfoo = \"1\"\n[dependencies]\nfoo.workspace = true\n";
        let m = parse_cargo_manifest(text).unwrap();
        assert!(m.deps.contains_key("foo"));
        // Same package in [dependencies] + [build-dependencies]: still one.
        let text2 = "[package]\nname = \"demo\"\n[dependencies]\nfoo = \"1\"\n[build-dependencies]\nfoo = \"1\"\n";
        let m2 = parse_cargo_manifest(text2).unwrap();
        assert!(m2.deps.contains_key("foo"));
        // Pins alone (never used by this manifest): zero, not one.
        let pins = "[workspace]\nmembers = [\"a\"]\n[workspace.dependencies]\nfoo = \"1\"\n";
        let m3 = parse_cargo_manifest(pins).unwrap();
        assert!(!m3.deps.contains_key("foo"));
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
