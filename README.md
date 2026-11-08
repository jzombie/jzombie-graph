# jzombie-graph

A map of my GitHub portfolio: which of my repos depend on which of my other repos. It collects the data (`graph.json`) and draws it (`ARCHITECTURE.md`).

Own repo, personal scope. `publish = false` — internal tool, never published to crates.io.

## Regenerate the JSON

```bash
export JZOMBIE_GRAPH_TOKEN=ghp_...
cargo run -- --user jzombie --format json --output graph.json
```

Needs a token for API rate limits (5000/hr vs 60/hr without). Public repos need no scopes; private repos need `repo` scope and are stripped from default output. Takes a while (~24 API calls per repo). Never run with `--include-private` output committed — that flag is local-briefing only.

## Regenerate the graph

```bash
cargo run -p jzombie-graph-cathedral -- graph.json --output ARCHITECTURE.md
```

Local only, no API calls. Reads the fresh `graph.json`, writes the clickable `<pre>` diagram. Commit both files together.

## License

MIT OR Apache-2.0. See [`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE).
