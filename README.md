# jzombie-graph

A map of my GitHub portfolio: which of my repos depend on which of my other repos. It collects the data and draws it. Generated files live under `data/` (see `src/paths.rs` — one source of truth); `data/` is gitignored, so after the first publish the assets come from Pages, not the repo.

Own repo, personal scope. `publish = false` — internal tool, never published to crates.io.

## Regenerate the JSON

```bash
export JZOMBIE_GRAPH_TOKEN=ghp_...
cargo run -- --user jzombie --format json --output data/json/portfolio-graph.json
```

Needs a token for API rate limits (5000/hr vs 60/hr without). Public repos need no scopes; private repos need `repo` scope and are stripped from default output. Takes a while (~24 API calls per repo). Never run with `--include-private` output committed — that flag is local-briefing only.

## Regenerate the graph

```bash
cargo run -p jzombie-graph-cartographer -- data/json/portfolio-graph.json --output data/markdown/fragments/portfolio-graph.md
```

Local only, no API calls. Reads the fresh JSON, writes the clickable `<pre>` fragment. Working copies stay in `data/` — never commit them; Pages serves them after publish.

## Focus ranking (offline, no API calls)

```bash
cargo run -- --input data/json/portfolio-graph.json --format focus
```

Same `focus` the dump lane shows — commit heat vs own 30d baseline minus idle months, hottest first. Needs a recent ingest; counts freeze at collection time.

## Regenerate the schema

```bash
cargo run -- --format schema --output data/json/portfolio-graph-schema.json
```

Static, no API calls and no selector — derived from the Rust types. To bless a contract change, copy the lane output over the committed `json/portfolio-graph-schema.json`; the drift test fails CI if the types and the file disagree.

## License

MIT OR Apache-2.0. See [`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE).
