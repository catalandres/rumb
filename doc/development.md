# Development

How to build, run, and test rumb during development. For the structure behind
these pieces, see [Architecture](architecture.md).

## Prerequisites

- A recent stable Rust toolchain (`cargo`).
- `git` on `PATH` — required for claim worktrees and for `doctor`'s ignore check,
  and used by the test suite, which creates temporary git repositories.

DuckDB is vendored via the `bundled` feature; no system DuckDB is needed.

## Building

```sh
cargo build                       # build the crate and both binaries (debug)
cargo build --release             # optimized

scripts/build.sh                  # both binaries, debug
scripts/build.sh --release        # both binaries, release
```

`scripts/build.sh` is a thin wrapper that runs
`cargo build --bin rumb --bin rumb-mcp` against the repo's manifest, forwarding
any extra arguments.

## Running locally

Use the repo shims so you do not have to remember binary paths. They build the
debug binary on first use and set the environment variables the launcher needs:

```sh
bin/rumb doctor
bin/rumb item create --kind feature --title "Example" --parent RUMB-0000
bin/rumb-mcp            # starts the MCP stdio server
```

Or run the built binaries directly:

```sh
./target/debug/rumb ready
```

## Testing

```sh
cargo test
```

The suite covers:

- **Core unit tests** ([`src/core.rs`](../src/core.rs)) — schema/migrations, ID
  allocation, depth, dependency-aware readiness, lifecycle transitions, claim
  exclusivity and TTL expiry, claim rollback on git failure, run pass/fail
  recording, and event coverage for every mutation. Tests that exercise claims
  create real temporary git repositories.
- **CLI parsing** ([`src/cli.rs`](../src/cli.rs)) — argument parsing for each
  command.
- **Output formatting** ([`src/main.rs`](../src/main.rs)) — deterministic tree
  and item-detail rendering.
- **MCP install** ([`src/mcp_install.rs`](../src/mcp_install.rs)) — writing and
  updating `.mcp.json`, preserving other servers, and the `--force` requirement.
- **MCP smoke** ([`tests/mcp_smoke.rs`](../tests/mcp_smoke.rs)) — starts
  `rumb-mcp`, lists tools, and drives the full create → ready → claim → run →
  release → log flow over JSON-RPC in a temporary repo.

You can verify your own build through rumb itself:

```sh
rumb run <id> --actor operator -- cargo test
```

## Installing release shims

`scripts/install.sh` builds the release binaries and writes shims into a prefix:

```sh
scripts/install.sh              # installs to $HOME/.local/bin
scripts/install.sh /opt/homebrew
```

The shims export `RUMB_HOME` and the appropriate `RUMB_SHIM` /
`RUMB_MCP_SHIM` and exec the release binaries. Make sure the target `bin`
directory is on your `PATH`.

## Dependencies

Declared in [`Cargo.toml`](../Cargo.toml):

- `clap` (derive) — CLI parsing.
- `duckdb` (bundled) — storage.
- `rmcp` (transport-io) — the MCP server.
- `serde` / `serde_json` — serialization.
- `thiserror` — the `RumbError` enum.
- `tokio` (macros, rt-multi-thread) — async runtime for `rumb-mcp`.
- `tempfile` (dev) — temporary repos in tests.
