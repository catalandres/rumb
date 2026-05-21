# Architecture

Rumb is a single Rust crate with shared core logic and two binaries built on top
of it. All behavior lives in the core library; the binaries are thin adapters —
one for humans (CLI), one for agents (MCP).

## Crate layout

```text
src/
├── lib.rs            # crate root; re-exports the public API
├── core.rs           # RumbProject + all coordination logic and storage
├── cli.rs            # clap command/argument definitions (parsing only)
├── mcp_install.rs    # writing/updating .mcp.json entries
├── main.rs           # the `rumb` CLI binary: dispatch + output formatting
└── bin/
    └── rumb-mcp.rs   # the `rumb-mcp` MCP stdio server
```

- **`core.rs`** is the heart. `RumbProject` owns project discovery, schema
  migrations, ID allocation, the item/edge/event APIs, depth and readiness
  computation, the claim/proposal/run lifecycles, and git interaction. It is
  synchronous and has no knowledge of the CLI or MCP.
- **`cli.rs`** defines the command tree with `clap` derive and does nothing but
  parse. `main.rs` matches on it, calls into `RumbProject`, and formats the
  tab-separated output (including the ASCII tree for `list` and the sectioned
  detail for `view item`).
- **`rumb-mcp.rs`** uses `rmcp` and `tokio` to expose the same `RumbProject`
  methods as JSON tools. Each tool is a thin wrapper that deserializes
  arguments, calls core, and serializes the result.
- **`mcp_install.rs`** is pure JSON-config manipulation, shared by the CLI's
  `mcp install` command.

This shared-core design is what keeps the CLI and MCP behaviorally identical:
there is one implementation of each operation.

## The two binaries

| Binary | Entry point | Audience | Transport |
| --- | --- | --- | --- |
| `rumb` | `src/main.rs` | operators, bootstrapping | argv → stdout/stderr |
| `rumb-mcp` | `src/bin/rumb-mcp.rs` | agents | MCP over stdio |

`rumb mcp serve` does not run the server in-process; it locates and execs the
`rumb-mcp` binary. This keeps the synchronous CLI free of the async MCP runtime.
See [How `rumb mcp serve` finds the server](mcp.md#how-rumb-mcp-serve-finds-the-server).

## Project discovery

Every operation except `init` resolves the project by walking up from the current
directory until it finds a `.rumb/` directory (`RumbProject::discover`). `init`
operates on the current directory directly. This is why rumb commands work from
any subdirectory of the repo.

## Git integration

Rumb shells out to `git` rather than linking a git library:

- `git worktree add -b <branch> <path>` to materialize a claim's worktree.
- `git rev-parse --abbrev-ref HEAD` to record a proposal's base ref.
- `git check-ignore --quiet .rumb/` for the `doctor` ignore check, with a
  fallback to reading `.git/info/exclude` directly.

Git must be on `PATH` for claims and for accurate `doctor` results.

## Storage

State is one DuckDB database (`duckdb` crate, `bundled` feature, so no system
DuckDB is required). Writes are transactional with short retry-on-busy backoff.
The schema is versioned through a `migrations` table and brought current on every
connection. See [Data model](data-model.md).

## Shims

Two layers of shell shims make the binaries convenient to run without remembering
cargo invocations or paths:

- **Dev shims** — [`bin/rumb`](../bin/rumb) and [`bin/rumb-mcp`](../bin/rumb-mcp)
  live in the repo. They set `RUMB_HOME`/`RUMB_SHIM`/`RUMB_MCP_SHIM`, build the
  debug binary on first use, and exec it. The checked-in `.mcp.json` points at
  `bin/rumb` so the MCP server works straight from a clone.
- **Release shims** — `scripts/install.sh` writes equivalent shims into a prefix
  (`$HOME/.local/bin` by default) that exec the release binaries.

See [Development](development.md) for usage.

## Environment variables

| Variable | Set by | Used for |
| --- | --- | --- |
| `RUMB_HOME` | shims | locating `target/{release,debug}` and `bin/` binaries |
| `RUMB_SHIM` | shims | the command path `mcp install` records in `.mcp.json` |
| `RUMB_MCP_SHIM` | shims | the `rumb-mcp` binary that `mcp serve` execs |

You normally never set these by hand; the shims manage them.
