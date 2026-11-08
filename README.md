# jzombie-graph

A map of my GitHub portfolio: which of my repos depend on which of my other repos. It collects the data and draws it. Generated files live under `data/` (see `src/paths.rs` — one source of truth); `data/` is gitignored, so after the first publish the assets come from Pages, not the repo.

Own repo, personal scope. `publish = false` — internal tool, never published to crates.io.

## Regenerate the JSON

```bash
export JZOMBIE_GRAPH_TOKEN=ghp_...
cargo run -- --user jzombie
```

Defaults to `--format json` into `data/json/portfolio-graph.json` (see `src/paths.rs` — one source of truth). Needs a token for API rate limits (5000/hr vs 60/hr without). Public repos need no scopes; private repos need `repo` scope and are stripped from default output. Takes a while (~24 API calls per repo). Never run with `--include-private` output committed — that flag is local-briefing only. `--stdout` prints instead of writing the file; `--output PATH` overrides the path.

## Regenerate the graph

```bash
cargo run -p jzombie-graph-cartographer
```

Local only, no API calls. Reads `data/json/portfolio-graph.json`, writes the clickable `<pre>` fragment to `data/markdown/fragments/portfolio-graph.md`. Piped stdin still wins when present; `--stdout` prints instead.

Local only, no API calls. Reads the fresh JSON, writes the clickable `<pre>` fragment. Working copies stay in `data/` — never commit them; Pages serves them after publish.

## Regenerate the charts

```bash
cargo run -p jzombie-graph-charts
cargo run -p jzombie-graph-charts -- --chart focus
```

Local only, no API calls. Reads the fresh JSON, writes the commit-share pie and the focus bars to `data/svg/`. Colors come from the shared `charts::palette` module so every chart reads as one family; bytes pinned by snapshot tests.

Local only, no API calls. Reads the fresh JSON, writes the commit-share pie. Colors come from the shared `charts::palette` module so every chart reads as one family; bytes pinned by snapshot tests.

## Focus ranking (offline, no API calls)

```bash
cargo run -- --format focus
```

Same `focus` the dump lane shows — commit heat vs own 30d baseline minus idle months, hottest first. Needs a recent ingest; counts freeze at collection time.

## Regenerate the schema

```bash
cargo run -- --format schema
```

Static, no API calls and no selector — derived from the Rust types. To bless a contract change, copy the lane output over the committed `json/portfolio-graph-schema.json`; the drift test fails CI if the types and the file disagree.

## Daily publish (CI only)

`.github/workflows/publish.yml` builds the workspace, installs the oxdock CLI (tracks latest), and runs `.github/scripts/pipeline.oxfile` (collect → schema → render → charts, fail-closed), then deploys `data/` to Pages. Trigger: daily cron + manual `workflow_dispatch`.

## License

MIT OR Apache-2.0. See [`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE).
