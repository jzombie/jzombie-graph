## Appendix: graph collector binary spec (extract into standalone issue when the repo is created)

This comment is written to be lifted whole into a new issue on the collector's own repo once it exists. It compiles every collector decision from the design thread.

### What it is
A standalone Rust binary: point it at a GitHub user/org, get back visibility-flagged portfolio graph JSON. Name locked: `jzombie-graph` (`jzombie/jzombie-graph`, explicitly personal-scope — not magnet-branded, no crates.io claim implied; rename only if a generic extraction ever ships). It becomes its own Tier 2 proof artifact: tested, versioned, released, with a row on this profile earned by serving the pipeline that renders it.

### Why a Rust binary (not DSL, not gh+jq)
- **Not OxDock DSL:** collection is retries, backoff, pagination cursors, malformed-JSON tolerance — I/O failure modes DSLs handle worst. DSL owns scoring/layout/render (data logic); the binary owns fetching (thin layer grown up). HTTP layer reuses `rust-reqwest-drive` (existing retry/backoff) — no new client logic. Unless OxDock ever grows a real HTTP module, DSL-side fetching is reimplementing reqwest badly.
- **Not `gh`+`jq` bash:** untestable glue with exactly the pagination/rate-limit failure modes tests catch — recreates what OxDock exists to replace.
- **Rust specifically:** the principal's stack (dogfooded), proper error handling, parallel fetches.

### Where it lives and runs
Own repo (extraction, per the term-wm precedent — tooling outlives the project that births it). Run by the Actions thin layer on the daily cron: Actions owns trigger + token + execution; the binary owns enumeration and parsing; OxDock owns everything downstream. The profile repo stays presentation + config (values, templates, DSL logic) with fast renders and readable source. Amusing structural note: the profile pipeline *uses* the collector, which is itself a workflow edge in the graph — the collector appears in the map it builds.

### Inputs: fail-open collection
- Enumerate everything owned by the user/org, **forks included** — fork status (`fork: true`, API-native) rides along as a node fact, not a verdict. Default views exclude forks; two mechanical rules re-admit them: **(a) divergence** — fork `ahead_by` N commits vs upstream (compare API, computable) counts as own work and includes it, still labeled fork (`term-wm-vt100` is the reference case); **(b) load-bearing** — a fork with fan-in from portfolio repos is structurally load-bearing and includes regardless (a portfolio repo depending on your fork is a real edge, not a study copy). No allowlist: new repos join by existing. The Tier 3 isolation rule (≥1 portfolio edge or omit) does the remaining junk filtering automatically — most study-forks point upstream at other people's repos, have no portfolio edges, and self-omit. Collect everything; let the graph decide what shows.
- **All branches**, not default-only (verified: GitHub's SBOM API is default-branch-only and therefore unusable for this). Union edges across branches; each edge carries the recency of the most recent branch declaring it.
- Manifests: `Cargo.toml` first (deps + build-deps; **dev-deps excluded** — test-only coupling isn't architectural), then package.json / pyproject / go.mod where present. Workspace members (`crates/` subdirs) walked recursively via git trees API and collapsed to the repo node — internal structure, not portfolio edges.

### Edge taxonomy (all mechanically collected; dependence weighed, associations shown)
| kind | source | contributes to node weight? |
|---|---|---|
| `manifest` | Cargo.toml etc., all branches | **yes** — declaring workspace-member count × scope (`optional` → 0.5, else 1.0) × use-site Boolean (0/1 from AST parsing in the collector; zero references nullifies; unparseable files count as matches; every nullification logged in the briefing digest; regex scan documented fallback), unioned across branches |
| `submodule` | `.gitmodules` | **yes** — 1.0 flat (indivisible pin) |
| `registry` | crates.io reverse-deps API (npm/PyPI equivalents where free) | **yes** — as manifest (published coupling counts like declared coupling) |
| `lockfile gate` | Cargo.lock / package-lock / poetry.lock, all branches | **gate, not weight** — package present at any version confirms the edge; absent-with-lockfile deletes it as unresolvable; no lockfile → manifest suffices. Resolved versions recorded as briefing facts (technical lag), never scored |
| `workflow` | `uses: owner/repo`, sibling checkouts, matrix refs | no — displayed association (pipeline depends, repo doesn't; revisit on logged counter-evidence) |
| `docs` | README/llms.txt links to siblings | no — displayed association |
| `xrefs` | issue/PR bodies referencing sibling repos | no — displayed association |

### Per-ecosystem implementation (same shape, local tools)
| mechanism | Rust | Node | Python |
|---|---|---|---|
| manifest parse | Cargo.toml (`[dependencies]`, `[build-deps]`, `[workspace]`); dev-deps excluded | package.json (`dependencies`; `peerDependencies`/`optionalDependencies` → 0.5 scope) | pyproject.toml / requirements.txt (extras markers → 0.5 scope) |
| member counting | workspace members declaring | npm-workspaces members declaring | packages declaring (monorepo: per-package) |
| lockfile gate | Cargo.lock (package present?) | package-lock.json / yarn.lock / pnpm-lock.yaml | poetry.lock / Pipfile.lock / frozen requirements |
| use-site check | **syn AST walk** (`use` items incl. `pub use`; unparseable file = match); regex identifier scan documented fallback | **swc_ecma_parser AST walk** (import decls + `require` calls); regex fallback | **rustpython-parser AST walk** (`Import`/`ImportFrom` nodes); regex fallback |
| alias mapping | package name → portfolio repo, in values.toml (all ecosystems — names diverge everywhere, this table is the fix) |

All three use-site tools run in the thin collector layer (fast, no full builds). A dep with zero use-sites anywhere is phantom regardless of ecosystem — nullified before fan-in. False positives (generated code, macros) keep the edge, which is the safe direction.

No kind-weights, no tuning protocol — superseded by counted dependence (locked decision 9): edge weight is manifest arithmetic × use-site Boolean, cross-repo edges pass the lockfile existence gate, node weight is summed incoming dependence, ordering is weight-desc/stability-tiebreak. The would-be calibration constants never existed. Workflow-as-association is the one parked judgment, flagged for revisit if a logged disagreement ever traces to it. Complexity rule: VFI and out-degree dampening are implemented alongside 1-level fan-in; whichever flips a Tier 1 boundary ships, the rest stay logged as checked alternatives.

### Privacy rule (locked)
Private repos participate in graph computation as transit nodes (no orphaned chains, true centrality) and are stripped before render: no nodes, stubs, links, counts, or deltas. Scoring sees the true graph; all surfaces show the public subgraph (both endpoints public).

### Output contract
Node/edge JSON with a stable documented schema. Every node carries a computed `visible` flag (ticking-or-load-bearing per decisions 6/9/12, compiled to data); every edge carries kind, declaring branches, and recency. Renderers filter blindly on the flag — no surface re-decides. JSON-first, OxDock renders; one optional convenience for adoption: `--format table` printing a human-readable summary to stdout (ten-second value for strangers). Inspection before building renders: `--format dot` emitting DOT notation for local eyeballing (dev-time only — Graphviz never enters the render pipeline), plus `jq`-queryable JSON from day one. Snapshot-test requirement: ship with `insta`-style snapshot tests asserting known truths (simd-r-drive fan-in floor, term-wm-vt100 promoted, zero private nodes in public output, llkv present-with-stale-heartbeat) — regressions show as diffs, and the suite is the continuous form of having inspected the graph. The four `jq` recipes that ship as documented examples (no code, day one): who-depends-on-X (filter edges by target), orphans (nodes with zero edges either way), heaviest edges (sort by weight desc, top 20), what-changed (diff yesterday's JSON against today's). Build/run sequence: table with the binary, DOT flag next (one afternoon, see the shape), snapshot tests at implementation, jq recipes the moment the first JSON exists. The private briefing digest (decision 13) inherits all four as its sources — it is the automation of inspections already done by hand, not a new inspection method.

### Why handrolled (verified Nov 7)
`cargo tree`/`guppy` (archived): single-workspace only. GitHub SBOM API: default-branch only. `rgctl` (cited in one report): no independent footprint, unverifiable, and a second engine regardless. Nothing does cross-repo × all-branches × custom edge kinds × private-transit × custom weights — the job is custom by definition. The collector is ~100 lines of enumeration feeding edge-JSON; scoring is counting; layout is BFS. No library needed because libraries solve single-workspace depth and this problem is portfolio breadth.

### Open-source checklist (ship day one)
Internal tool, no crates.io publish planned — naming unconstrained by registry. `publish = false` in Cargo.toml to make that mechanical. MIT/Apache dual license. Clippy + fmt + tests in CI. README that demos `--format table` in under a minute. Personal naming (`jzombie-graph`), config-driven everything else (repo source, edge kinds, branch scope, window days, kind weights, private handling), two input modes (explicit list + enumerate-org).

### Relationship to the rest of #1
This binary feeds the edge-JSON that decisions 9 (counted dependence) and 10 (pipeline inference) consume. Tier 1's hand-picked pin list is bootstrap until the inference ships on this binary's output. The decision-10 disagreement log records algorithm-vs-hand mismatches as signal (wrong graph or wrong mental model — both worth knowing), not as training data; there is nothing left to tune. The `tagline`/`system_blurb` precondition is independent and unchanged.

### Metrics storage (milestone 2 — after first edge-JSON ships)
The traffic API is a 14-day rolling window: unstored, it is deleted daily. Storage here is preservation, not caching. The graph repo is the single home for it — JSONLs live inside `jzombie/jzombie-graph`, never inside the profile repo, the blog, or any portfolio repo. Presentation repos hold config and templates; the collector's own history stays with the collector.

- **Format:** append-only JSONL, one line per repo per sweep. JSON-first, jq-queryable, no database until queries demand one. The `what-changed` recipe already assumes yesterday's file exists — this extends the snapshot concept from structure to metrics.
- **Atomic unit = daily array, not total.** Views and clones both return per-day breakdowns; totals are derived, days are stored. Overlapping 14-day windows then reconcile instead of double-counting, and one snapshot per day reconstructs true daily history. This is the difference between a trend and a pile of samples.
- **Fetch in the binary, read in the DSL.** Traffic fetching is retries, backoff, pagination, rate limits — the exact failure modes the spec assigns to Rust. The daily cron already exists as the trigger; one fetch per day serves every query that day. `GH_TRAFFIC_SWEEP` becomes a reader of the store, not a caller of the API.
- **Private repos are never recorded in the metrics store.** The JSONL is written public-repos-only, filtered at write time — not stripped at render. Rationale: traffic feeds trends, not graph computation (edge weight is manifest arithmetic x use-site Boolean; node weight is summed incoming dependence — no traffic input), so private repos have no reason to be in the file. Structure keeps the original rule (private computes in memory, stripped before render); metrics exclude entirely. Because the store contains only public repos, it commits safely to the public repo, git tracks it as history, and README stats published from it need no filter pass — every count in the file is already public. A private repo's own traffic, if ever wanted, belongs to the decision-13 briefing lane: separate gitignored file, never in this repo, local append plus encrypted vaultclone backup on the `archive-logs.sh` pattern.
- **Honest ceiling:** referrers and paths are top-10 with no daily breakdown. Storing them yields history, not reconstruction. Views and clones are the parts that become true time series.

Sequencing is locked: enumeration + edge-JSON first (unblocks Tier 1 and the hub), metrics append second. The binary ships milestone 1 before growing scope — the `~100 lines of enumeration` claim holds until the first JSON exists.
