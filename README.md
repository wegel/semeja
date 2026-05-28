# semeja

**Fast, accurate code search for agents — a native Rust implementation.**

`semeja` indexes a codebase and returns the exact code snippets relevant to a
query, whether that query is natural language (*"how is authentication
handled"*) or a bare symbol (*"HybridRetriever"*). It blends lexical (BM25) and
semantic (vector) retrieval, runs entirely on CPU with no API keys or GPU, and
indexes a large repository end-to-end in about a second.

It is a ground-up Rust port of [**semble**](https://github.com/MinishLab/semble)
by the [MinishLab](https://github.com/MinishLab) team — see
[Credits](#credits).

---

## Quickstart

Build and install the CLI (requires a Rust toolchain):

```bash
cargo build --release
install -m755 target/release/semeja ~/.local/bin/semeja
```

Search any directory or git URL:

```bash
semeja search "how are database queries built" ./my-project
semeja search "BiteSizedRetriever" ./my-project --top-k 10
semeja search "parse the config file" https://github.com/org/repo
```

The first run downloads the embedding model (~16 MB) from the Hugging Face hub
and caches it locally; subsequent runs are offline.

---

## CLI

```
semeja search <query> [path] [-k N] [-m MODE] [-t | --model NAME] [--include-text-files]
semeja find-related <file_path> <line> [path] [-k N] [-t | --model NAME]
semeja init [--force]
semeja savings [--verbose]
```

- **`search`** — find code by intent or by symbol. `path` defaults to the
  current directory and may be a local path or a git URL. `MODE` is `hybrid`
  (default), `semantic`, or `bm25`.
- **`find-related`** — given a `file_path` and `line` from a prior result,
  return semantically similar code elsewhere in the repo.
- **`-t` / `--model`** — which embedding model to use: `code` (default, for
  source) or `text` (for prose/docs), or any Hugging Face `model2vec` name.
  **`-t` is the document-search mode**: it selects the text model, turns on
  `--include-text-files`, *and* relaxes the file-diversity penalty so multiple
  matching sections of the same document surface. `semeja search -t "<query>"`
  is all you need to search docs. See [Embedding models](#embedding-models).
  Also settable via `SEMEJA_MODEL`.
- **`--per-file N`** — how many chunks from one file may rank at full score
  before the diversity penalty kicks in. Defaults to `1` for code (spread
  results across files) and unbounded for `-t` (concentrate on the best file).
- **`--include-text-files`** — also index documentation files (`.md`, `.txt`,
  `.rst`, `.yaml`, `.json`, …), which are skipped by default.
- **`init`** — write `.claude/agents/semeja-search.md`, a Claude Code
  sub-agent definition that teaches an agent to use `semeja` instead of grep.
- **`savings`** — report how many tokens `semeja` has saved versus reading
  whole files.

Results are rendered as numbered, fenced code blocks with relevance scores —
ready to paste into an agent's context.

---

## Integrating with an agent

`semeja` is built to be driven by coding agents (Claude Code, Cursor, Codex,
OpenCode, …) in place of `grep`. There are three ways to wire it in.

### AGENTS.md / CLAUDE.md — manual, any agent

Install `semeja` on your `PATH`, then paste the block below into your project's
`AGENTS.md` (or `CLAUDE.md`). It tells the agent to reach for `semeja` before
grepping — no other setup required:

````md
## Code search

Use `semeja search` to find code by intent or by symbol name. It returns the
relevant snippets directly, so prefer it over `grep`/`glob` and reading whole
files:

```bash
semeja search "how is authentication handled" .
semeja search "HybridRetriever" .
```

Use `semeja find-related <file_path> <line>` — with values taken from a search
result — to find similar code elsewhere. Fall back to `grep` only when you need
exhaustive literal matches.
````

### Claude Code sub-agent — automatic

Run `semeja init` in the project root. It writes
`.claude/agents/semeja-search.md`, a dedicated Claude Code sub-agent restricted
to the `Bash` and `Read` tools that knows how to drive `semeja`. Claude Code
then delegates code-search and exploration questions to it automatically.
Re-run with `--force` to overwrite an existing file.

### MCP

`semeja` also implements the `search` and `find_related` tools as an MCP-style
server with a cached index — see [MCP server](#mcp-server). The tool and cache
logic are complete and tested; a stdio protocol transport for plugging into MCP
clients is not bundled in this build, so for now the AGENTS.md or sub-agent
route above is how you wire `semeja` into an agent.

---

## How it works

1. **Walk** the directory, honouring `.gitignore` and `.semejaignore` and
   skipping vendored/build directories.
2. **Chunk** each file along syntax boundaries using
   [tree-sitter](https://tree-sitter.github.io/): functions and classes stay
   intact rather than being cut at arbitrary line counts. Markdown is chunked
   by **section**, so each chunk leads with its heading and carries the text
   that heading introduces.
3. **Embed** every chunk with a `model2vec` static embedding model — a
   distilled, CPU-only model with no transformer inference at query time. The
   model is selectable (see [Embedding models](#embedding-models)).
4. **Index** the chunks into a BM25 lexical index and a cosine-similarity
   vector index.
5. **Search** by fusing both rankings with reciprocal-rank fusion, then
   applying query-aware boosts (symbol definitions, file-stem matches) and
   path penalties (test files, re-export barrels, compat shims).

The blend weight adapts to the query: symbol-like queries lean on BM25,
natural-language queries balance both.

---

## Embedding models

`semeja` ships with two `model2vec` static embedding models — both CPU-only,
fetched from the Hugging Face hub and cached locally on first use:

- **`code`** (default) — `minishlab/potion-code-16M`, tuned for source code.
- **`text`** — `minishlab/potion-retrieval-32M`, tuned for natural-language
  retrieval; better for prose, Markdown, and documentation.

Select per invocation with `-t` (shorthand for `--model text`) or
`--model code|text|<name>`, or set the `SEMEJA_MODEL` environment variable. Any
other value is treated as a Hugging Face `model2vec` repo name, so you can
point `semeja` at your own model.

A single index uses one model — query and chunk embeddings must share an
embedding space — so pick the model that matches the corpus. Code and document
search are tuned differently: code search **spreads results across files** (a
file-diversity penalty), while `-t` **concentrates on the best-matching file**
so several sections of one document can all surface. `-t` rolls together the
text model, doc-file indexing, and that ranking change:

```bash
semeja search -t "deployment rollback procedure" ./docs
```

Tune the spread/concentrate balance explicitly with `--per-file N`.

---

## Language support

Every file type is indexed and searchable. Files in a language with a bundled
tree-sitter grammar are chunked along **syntax boundaries**; the rest fall back
to line-based chunking.

Grammars are bundled for **50+ of the most popular languages**: Python,
JavaScript, TypeScript/TSX, Rust, Go, Java, C, C++, C#, Ruby, PHP, Bash, HTML,
CSS, JSON, Scala, Haskell, OCaml, Elixir, Lua, Kotlin, Swift, R, SQL, Dart,
Perl, Objective-C, Julia, Groovy, PowerShell, Markdown, YAML, TOML, Erlang,
Zig, Nix, Solidity, Svelte, Ada, CMake, Make, Elm, HCL, Gleam, Scheme, Common
Lisp, F#, Verilog/SystemVerilog, CUDA, GraphQL, and Protobuf.

The full set is the `GRAMMARS` table in `src/chunk.rs`; adding another language
is one line plus its `tree-sitter-*` crate.

---

## Performance

Benchmarked against the original `semble` on the **Django** source tree
(3,073 files → 18,678 chunks, identical for both implementations):

| Metric                          |   semeja |    semble |   speedup |
| -------------------------------- | --------:| ---------:| ---------:|
| Cold CLI (index + search)        | 1.41 s   |  7.73 s   |   **5.5×** |
| In-process index build           | ~1.15 s  |  7.50 s   |   **6.5×** |
| Warm search latency               | 1.56 ms  | 22.99 ms  |  **14.7×** |

`semeja` is faster on every axis. Indexing is parallelised across cores with
[rayon](https://crates.io/crates/rayon) (file reading, chunking, embedding, and
tokenization all run in parallel), and being a native binary it has none of an
interpreter's start-up cost. Run the included benchmark yourself:

```bash
cargo run --release --example bench -- /path/to/a/repo
```

---

## MCP server

`semeja` exposes the same `search` and `find_related` tools as an in-process
MCP-style server, with an LRU cache of indexed repositories. The server logic
lives in `src/mcp.rs`; a stdio protocol transport is not bundled in this build.

---

## Project layout

Flat, one module per concern:

| File              | Responsibility                                        |
| ----------------- | ----------------------------------------------------- |
| `chunk.rs`        | tree-sitter and line-based source chunking            |
| `walk.rs`         | gitignore-aware file discovery                        |
| `lang.rs` / `lang_table.rs` | language detection and the extension table  |
| `embed.rs`        | the embedding model and cosine vector search          |
| `bm25.rs`         | the BM25 lexical index                                |
| `rank/`           | query-aware boosts, path penalties, alpha weighting   |
| `search.rs`       | semantic, BM25, and hybrid search                     |
| `index.rs`        | the top-level `SemejaIndex`                           |
| `stats.rs`        | token-savings tracking                                |
| `cli.rs` / `mcp.rs` | command-line interface and MCP-style server         |

---

## Testing

```bash
cargo test                  # unit and integration tests
cargo test -- --ignored     # also exercises the real model download
```

The suite ports `semble`'s tests one-to-one — chunking, file walking, ranking,
search, indexing, stats, the CLI, and the MCP server.

---

## Differences from `semble`

- **tree-sitter coverage.** `semble` pulls in `tree-sitter-language-pack`
  (300+ grammars); `semeja` bundles the 50+ languages listed above and falls
  back to line-based chunking for the niche long tail.
- **MCP transport.** The cache and tool logic are implemented and tested in
  process; a stdio protocol transport is not bundled.

Everything else — the chunking algorithm, hybrid scoring, query-aware boosts,
path penalties, and CLI behaviour — is a faithful reimplementation, verified by
the ported test suite.

---

## Credits

`semeja` reimplements **[semble](https://github.com/MinishLab/semble)**,
created by **Thomas van Dongen** and **Stéphan Tulkens** at
[MinishLab](https://github.com/MinishLab). The design it follows — syntax-aware
chunking, hybrid BM25 + static-embedding retrieval, the query-aware ranking
heuristics, and the agent-first CLI — originates entirely with the MinishLab
team, and full credit for the idea and approach goes to them. `semeja` also
uses MinishLab's [`model2vec`](https://github.com/MinishLab/model2vec)
embedding model via the
[`model2vec-rs`](https://crates.io/crates/model2vec-rs) crate.

Copyright does not protect ideas or algorithms, so `semeja`'s Rust
reimplementation is original work. A few files, however, carry data tables,
regexes, or text copied directly from `semble`; those portions remain
Copyright (c) 2026 Thomas van Dongen and are itemised in
[`LICENSE`](LICENSE).

Built on [tree-sitter](https://tree-sitter.github.io/) and its grammar crates.

## License

MIT. `semeja` is Copyright (c) 2026 the semeja authors; the `semble`-derived
portions remain Copyright (c) 2026 Thomas van Dongen. Both are MIT-licensed —
see [`LICENSE`](LICENSE) for the file-by-file breakdown.
