# jzombie-graph

Standalone Rust binary: point it at a GitHub user/org, get back visibility-flagged portfolio graph JSON.

Own repo (personal scope, `jzombie/jzombie-graph`). `publish = false` — internal tool, no crates.io claim.

## 60-second demo

```bash
export GITHUB_TOKEN=ghp_...
cargo run -- --user jzombie --format table
```

```
jzombie-graph: 42 nodes (18 visible), 57 edges [schema v1]
--- nodes (weight-desc) ---
...
--- heaviest edges (top 20) ---
...
```

Other formats:

```bash
# DOT for local eyeballing (dev-time only — Graphviz never enters the render pipeline)
cargo run -- --user jzombie --format dot > portfolio.dot && dot -Tsvg portfolio.dot -o portfolio.svg

# JSON-first, jq-queryable from day one
cargo run -- --user jzombie --format json > graph.json
```

## Config (`values.toml`)

Everything is config-driven. Copy the example and edit:

```bash
cp values.example.toml values.toml
cargo run -- --config values.toml --format table
```

Key knobs: `source.repos` (explicit list) XOR `source.user`/`source.org` (enumerate);
`aliases` (package name -> `owner/repo` — names diverge everywhere, this table is the fix);
`collection.extra_aliases` (same, per-sweep overrides).

Two input modes: `--repos a,b,c` (explicit) or `--user`/`--org` (enumerate-all, forks included).

## Token scopes (least privilege)

| Tier | Classic PAT | Fine-grained PAT | Notes |
|---|---|---|---|
| Public-only graph (default output today) | no scopes | public repos, Contents read | Token is rate-limit only (5000/hr vs 60/hr unauthenticated). Token required by default; empty only with explicit `--allow-unauthenticated` (public repos only, 60/hr — never silent) |
| Private transit (compute, stripped from default output) | `repo` | private repos, Contents read + Metadata read | Needed to read private manifests/branches for transit edges |
| Traffic (milestone 2) | `repo` | Administration read | GitHub requires push access for views/clones/referrers |

Provision the cron token for the highest tier you plan to use so it isn't re-scoped later.

## Documented jq recipes (no code, day one)

```bash
# who-depends-on-X
jq '.edges | map(select(.target == "me/some-lib"))' graph.json
# orphans (nodes with zero edges either way)
jq '.nodes | map(.id) as $n | $n - ([.edges[] | .source, .target] | unique)' graph.json
# heaviest edges, top 20
jq '[.edges | sort_by(-.weight) | .[:20]]' graph.json
# what-changed (yesterday vs today — assumes yesterday.json exists)
diff <(jq -S . yesterday.json) <(jq -S . graph.json)
```

## Collection rules (locked)

- Enumerate everything, forks included (`fork: true` rides as fact). Default views
  exclude forks; two rules re-admit: **divergence** (`ahead_by > 0`) and
  **load-bearing** (fan-in from portfolio repos). Tier 3 isolation (≥1 portfolio
  edge or omit) does the rest of the filtering.
- Default branch only, always. Unmerged branch deps are proposals, not structure;
  each edge carries the single declaring branch. (A repo developing on `next`
  while `main` lags needs a per-repo override, not a sweep.)
- Manifests: `Cargo.toml` first (deps + build-deps, dev-deps excluded), then
  package.json / pyproject / go.mod. Workspace members collapsed to the repo node.
- Edge kinds: `manifest` (weight = members × scope × use-site), `submodule` (1.0 flat),
  `registry` (reverse-deps), lockfile **gate** (confirms, never scores),
  `workflow` / `docs` / `xrefs` (associations, shown not scored).
- Use-site check: Rust via `syn` AST (`use` items incl. `pub use`; unparseable = match),
  regex fallback elsewhere. Default branch only, `.rs` files capped (budget bound).
  Partial coverage is fail-open (edges kept); only a full scan with zero refs
  nullifies. Every nullification and cap note is logged in the briefing digest (stderr).
- Privacy: private repos compute as transit, stripped by default. Default output
  is the public subgraph (store-safe); `--include-private` emits the true graph
  for the briefing lane only (never commit). `visible` flag is computed; renderers
  filter blindly.
- Budget: the binary prints a worst-case API call estimate before fetching
  (rate limit 5000/hr). Caps: workflows/member-manifests/`.rs` files each ≤10,
  use-sites on the default branch only. HTTP goes through `rust-reqwest-drive`
  (`DriveThrottleBackoff`: throttle + retries + backoff); 404 probes retry
  inside the middleware, accounted in the estimate.

## Privacy

Scoring sees the true graph; default output shows the public subgraph (both endpoints public).
No nodes, stubs, links, counts, or deltas for private repos. `--include-private`
overrides for local briefing only — never commit that output.

## Metrics storage (milestone 2 — after first edge-JSON ships)

Traffic API is a 14-day rolling window; unstored, it is deleted daily. JSONL store lives
in this repo, public-repos-only, one line per repo per sweep. See plan for the full spec.
Not implemented yet — enumeration + edge-JSON ships first.

## License

MIT OR Apache-2.0. See `LICENSE-MIT`, `LICENSE-APACHE`.
