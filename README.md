# rumb

A small, local coordinator for agent work in a single git repository.

Rumb tracks a graph of work items, lets one operator and many agents claim ready
work into isolated git worktrees, runs and records verification commands, and
keeps a full event trail — all in one DuckDB file under `.rumb/`. It ships a CLI
(`rumb`) and an MCP server (`rumb-mcp`) built on the same core, so operators and
agents drive identical operations.

## Quick start

Requires a Rust toolchain and `git`. DuckDB is vendored.

```sh
bin/rumb init --name my-project        # create .rumb/ state, seed root RUMB-0000
bin/rumb item create --kind feature --title "First task" --parent RUMB-0000 --status ready
bin/rumb ready                          # show claimable work
bin/rumb claim RUMB-0001 --actor operator
bin/rumb run RUMB-0001 --actor operator -- cargo test
bin/rumb done RUMB-0001 --actor operator
```

`bin/rumb` and `bin/rumb-mcp` are dev shims that build on first use. To build or
install the binaries directly, see [doc/development.md](doc/development.md).

## Documentation

Full documentation lives in [`doc/`](doc/README.md):

- [Getting started](doc/getting-started.md) — install, init, and a full work cycle.
- [Concepts](doc/concepts.md) — items, lifecycle, depth, readiness, claims, runs, events.
- [CLI reference](doc/cli.md) — every `rumb` command and flag.
- [MCP server](doc/mcp.md) — running `rumb-mcp`, its tools, and registration.
- [Data model](doc/data-model.md) — the `.rumb/` layout and DuckDB schema.
- [Architecture](doc/architecture.md) and [Development](doc/development.md) — internals and building.

## Status

Version `0.1.0`. Rumb self-hosts: its own work is coordinated through rumb. Out of
scope for now: federation, approvals, auth, PR integration, a TUI, and daemon
runners.

## License

MIT.
