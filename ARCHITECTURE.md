# The Zenosmosis Cathedral

Generated from `graph.json` (schema v5). Forks bear load only in the crypt.

<pre>
                                               ▲
                                               / \
[<a href="https://github.com/jzombie/rust-argtuner">argtuner</a>]  [<a href="https://github.com/jzombie/rust-cargo-pkg-info-struct-builder">cargo-pkg-info-struct-builder</a>]  [<a href="https://github.com/jzombie/rust-llkv">llkv</a>]  [<a href="https://github.com/jzombie/rust-sec-fetcher">sec-fetcher</a>]  [<a href="https://github.com/jzombie/rust-triplets">triplets</a>]  [<a href="https://github.com/jzombie/windows-headless">windows-headless</a>]
     │        │            │           │        │          │         │   │           │
     ┼────────┼────────────┼───────────┼────────┼──────────┼─────────┼───┼───────────┼
       [<a href="https://github.com/jzombie/rust-simd-r-drive">simd-r-drive</a>]──[<a href="https://github.com/jzombie/rust-muxio">muxio</a>]  [<a href="https://github.com/jzombie/rust-reqwest-drive">reqwest-drive</a>]  [<a href="https://github.com/jzombie/rust-oxdock">oxdock</a>]  [<a href="https://github.com/jzombie/rust-string-auto-indent">string-auto-indent</a>]  [<a href="https://github.com/jzombie/term-wm">term-wm</a>]
                                       ││            │  │            │               │
                                       ┼┼────────────┼──┼────────────┼───────────────┼
                                  [<a href="https://github.com/jzombie/rust-line-ending">line-ending</a>]  [<a href="https://github.com/jzombie/rust-cache-manager">cache-manager</a>]

\───────────────────────────────┐                                ┌───────────────────────────────/
                                ▼                                ▼
==================================================================================================
FOUNDATION: cache-manager & line-ending
CRYPT (LOAD-BEARING FORKS): ratatui (15 incoming)  ·  bitcode (6 incoming)  ·  bm25 (1 incoming)  ·  libtest-mimic (1 incoming)  ·  term-wm-vt100 (1 incoming)
==================================================================================================
</pre>
### Architectural Elevation

| Level | Element | Node | Load |
| --- | --- | --- | --- |
| **Spires** | A language-agnostic CLI tool for black-box hyperparameter optimization. | [`argtuner`](https://github.com/jzombie/rust-argtuner) · [docs](https://crates.io/crates/argtuner) | fan-in 0 · rests on line-ending (1.5.1), term-wm (0.10.15-alpha) |
| **Spires** | A Rust build-time dependency injector that provides structured access to Cargo package metadata at runtime. | [`cargo-pkg-info-struct-builder`](https://github.com/jzombie/rust-cargo-pkg-info-struct-builder) · [docs](https://crates.io/crates/cargo-pkg-info-struct-builder) | fan-in 0 · rests on string-auto-indent (0.1.0-alpha) |
| **Spires** | An Apache Arrow columnar storage layer with SQL for key-value storage systems in Rust. | [`llkv`](https://github.com/jzombie/rust-llkv) · [docs](https://docs.llkv.zenosmosis.com/) | fan-in 0 · rests on simd-r-drive (0.15.5-alpha) |
| **Spires** | Async Rust client for SEC EDGAR: fetch filings, fund holdings, XBRL data, and IPO registrations with compliant rate limiting. | [`sec-fetcher`](https://github.com/jzombie/rust-sec-fetcher) · [docs](https://crates.io/crates/sec-fetcher) | fan-in 0 · rests on reqwest-drive (0.13.2-alpha.2), simd-r-drive (0.16.0-alpha) |
| **Spires** | Composable data sampling primitives for deterministic multi-source ML/AI training-data orchestration. | [`triplets`](https://github.com/jzombie/rust-triplets) · [docs](https://crates.io/crates/triplets) | fan-in 0 · rests on cache-manager (0.6.0), line-ending (1.5.1), reqwest-drive (0.13.5-alpha.1), simd-r-drive (0.17.3-alpha) |
| **Spires** | Turn a Microsoft ISO into an SSH-ready Windows 11 VM in one command. Unattended installation. | [`windows-headless`](https://github.com/jzombie/windows-headless) · [docs](https://crates.io/crates/windows-headless) | fan-in 0 · rests on cache-manager (0.6.0) |
| **Vaults** | Zero-copy access, schema-less, append-only, SIMD-optimized binary storage engine for Rust. | [`simd-r-drive`](https://github.com/jzombie/rust-simd-r-drive) · [docs](https://crates.io/crates/simd-r-drive) | fan-in 4 · rests on muxio (0.17.2-alpha) |
| **Vaults** | Rust toolkit for layered stream multiplexing and schemaless RPC communication. | [`muxio`](https://github.com/jzombie/rust-muxio) · [docs](https://crates.io/crates/muxio) | fan-in 3 |
| **Vaults** | Middleware for the Rust `reqwest` crate that provides fast, SIMD-accelerated caching, throttling, and backoff control within a single-file container. | [`reqwest-drive`](https://github.com/jzombie/rust-reqwest-drive) · [docs](https://crates.io/crates/reqwest-drive) | fan-in 2 · rests on cache-manager (0.6.0), simd-r-drive (0.17.3-alpha) |
| **Vaults** | An experimental DSL used for building embeddable artifacts and orchestrating pipelines. | [`oxdock`](https://github.com/jzombie/rust-oxdock) · [docs](https://crates.io/crates/oxdock) | fan-in 1 · rests on line-ending (1.6.0), muxio (0.17.0-alpha) |
| **Vaults** | A Rust utility for automatically normalizing multi-line string indentation while preserving platform-specific line endings. | [`string-auto-indent`](https://github.com/jzombie/rust-string-auto-indent) · [docs](https://crates.io/crates/string-auto-indent) | fan-in 1 · rests on line-ending (1.2.0) |
| **Vaults** | Spatial Terminal Desktop Environment for Remote Workspaces | [`term-wm`](https://github.com/jzombie/term-wm) · [docs](https://crates.io/crates/term-wm) | fan-in 1 · rests on line-ending (1.5.1), muxio (0.17.0-alpha), oxdock (0.21.0-alpha) |
| **Buttresses** | A Rust crate to detect, normalize, and convert line endings across platforms. Ensures consistent handling of LF, CRLF, and CR line endings in text processing. | [`line-ending`](https://github.com/jzombie/rust-line-ending) · [docs](https://crates.io/crates/line-ending) | fan-in 5 |
| **Buttresses** | Managed caches, with optional eviction, for Rust project artifacts. | [`cache-manager`](https://github.com/jzombie/rust-cache-manager) · [docs](https://crates.io/crates/cache-manager) | fan-in 3 |
| **Crypt** | A Rust crate for cooking up terminal user interfaces (TUIs) 👨‍🍳🐀 https://ratatui.rs | [`ratatui`](https://github.com/jzombie/ratatui) · [docs](https://ratatui.rs) | fan-in 15 |
| **Crypt** | A binary encoder/decoder for Rust | [`bitcode`](https://github.com/jzombie/bitcode) · [docs](https://crates.io/crates/bitcode/) | fan-in 6 |
| **Crypt** | A BM25 embedder, scorer, and search engine, written in Rust. | [`bm25`](https://github.com/jzombie/bm25) · [docs](https://crates.io/crates/bm25) | fan-in 1 |
| **Crypt** | A small test framework to write your own test harness that looks and behaves like the built-in test harness used by `rustc --test` | [`libtest-mimic`](https://github.com/jzombie/libtest-mimic) · [docs](https://docs.rs/libtest-mimic) | fan-in 1 |
| **Crypt** | A fork of vt100 specifically for usage in term-wm. | [`term-wm-vt100`](https://github.com/jzombie/term-wm-vt100) · [docs](https://crates.io/crates/term-wm-vt100) | fan-in 1 |

### Side Chapels & Workshops

* **The Documentation Scriptorium:** [`deepwiki-to-mdbook`](https://github.com/jzombie/deepwiki-to-mdbook)
*Workflow ink from:* [`docker-mqtt-mosquitto-cloudflare-tunnel`](https://github.com/jzombie/docker-mqtt-mosquitto-cloudflare-tunnel) · [`llkv`](https://github.com/jzombie/rust-llkv) · [`muxio`](https://github.com/jzombie/rust-muxio) · [`sec-fetcher`](https://github.com/jzombie/rust-sec-fetcher) · [`simd-r-drive`](https://github.com/jzombie/rust-simd-r-drive) · [`sqlite-sqllogictest-corpus`](https://github.com/jzombie/sqlite-sqllogictest-corpus) · [`sshfs-mac-docker`](https://github.com/jzombie/sshfs-mac-docker)
* **Detached chapel:** [`ticker-sniffer`](https://github.com/jzombie/rust-ticker-sniffer) → [`embed-bytes`](https://github.com/jzombie/rust-embed-bytes)
* **Lone stones:** [`ratatui-example-counter-app`](https://github.com/jzombie/rust-ratatui-example-counter-app) · [`ratatui-example-json-editor-app`](https://github.com/jzombie/rust-ratatui-example-json-editor-app) · [`term-icon-lab`](https://github.com/jzombie/term-icon-lab)

