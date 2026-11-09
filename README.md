# jzombie-graph

A map of my GitHub portfolio: which of my repos depend on which of my other repos. It collects the data and draws it. Generated files live in the repo (see `src/paths.rs` — one source of truth): `json/` for data, `svg/` + `markdown/` for rendering, `preview/` for the proof page. The nightly sweep regenerates and commits them.

Own repo, personal scope. `publish = false` — internal tool, never published to crates.io.

## Regenerate the JSON

```bash
export JZOMBIE_GRAPH_TOKEN=ghp_...
cargo run -- --user jzombie
```

Defaults to `--format json` into `json/portfolio-graph.json` (see `src/paths.rs` — one source of truth). The schema (`json/portfolio-graph-schema.json`) refreshes on the same run — one ingest, both copies, never a stale contract. Needs a token for API rate limits (5000/hr vs 60/hr without). Public repos need no scopes; private repos need `repo` scope and are stripped from default output. Takes a while (~24 API calls per repo). Never run with `--include-private` output committed — that flag is local-briefing only. `--stdout` prints instead of writing the file; `--output PATH` overrides the path.

## Regenerate the graph

```bash
cargo run -p jzombie-graph-cartographer
```

Local only, no API calls. Reads `json/portfolio-graph.json`, writes the clickable `<pre>` fragment to `markdown/fragments/portfolio-graph.md`. Piped stdin still wins when present; `--stdout` prints instead.

Local only, no API calls. Reads the fresh JSON, writes the clickable `<pre>` fragment. Generated files commit like source — the nightly rebuilds them.

## Regenerate the charts

```bash
cargo run -p jzombie-graph-charts
cargo run -p jzombie-graph-charts -- --chart focus
```

Local only, no API calls. Reads the fresh JSON, writes the commit-share pie and the focus bars to `svg/`. Colors come from the shared `charts::palette` module so every chart reads as one family; bytes pinned by snapshot tests.

Local only, no API calls. Reads the fresh JSON, writes the commit-share pie. Colors come from the shared `charts::palette` module so every chart reads as one family; bytes pinned by snapshot tests.

## Developer Focus (offline, no API calls)

```bash
cargo run -- --format focus
```

Same `focus` the dump lane shows — commit heat vs own 30d baseline minus idle months, hottest first. Needs a recent ingest; counts freeze at collection time.

## Render proof (what GitHub actually shows)

```bash
oxdock scripts/preview-indicators.oxfile
```

Collects fresh, renders every lane, and assembles the committed `preview/portfolio-graph.md` — map fragment verbatim, all charts embedded, machine-readable commit-data table (role/proof/reuse per repo, layer-labeled, tiny topic search links, top-10 + roll-up), indicator table with badges. That page is the visual proof; `preview/` is generated, never hand-edited. All markup emission is [OxDock](https://github.com/jzombie/rust-oxdock)-templated (tables via the shared `MARKDOWN` module); the script only orders the lanes.

## Regenerate the schema

```bash
cargo run -- --format schema
```

Static, no API calls and no selector — derived from the Rust types. The nightly lands the regenerated copy automatically; locally, copy the lane output over the committed `json/portfolio-graph-schema.json` yourself. Either way the drift test fails CI if the types and the file disagree.

## Nightly rebuild (CI only)

`.github/workflows/nightly.yml` builds the workspace, installs the oxdock CLI (tracks latest), and runs `.github/scripts/pipeline.oxfile` (collect → schema → render → charts → preview → commit, fail-closed). Trigger: daily cron + manual `workflow_dispatch`. Nothing publishes: the rebuilt `preview/` commits back to the repo, gated on change — quiet sweeps push nothing.

## License

MIT OR Apache-2.0. See [`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE).
