# Rumb documentation

Rumb is a small, local coordinator for agent work in a single git repository. It
tracks a graph of work items, lets one operator and many agents claim ready work
into isolated git worktrees, runs and records verification commands, and keeps a
full event trail — all in a single DuckDB file under `.rumb/`.

It exposes the same core through two interfaces:

- **`rumb`** — a CLI for operators and bootstrapping.
- **`rumb-mcp`** — an MCP stdio server so agents can drive the same operations.

## Start here

- [Getting started](getting-started.md) — install, initialize a repo, and walk
  the first claim → run → done cycle.

## Reference

- [Concepts](concepts.md) — the work graph, item lifecycle, depth, readiness,
  claims and leases, runs, and the event log.
- [CLI reference](cli.md) — every `rumb` command, its flags, and its output.
- [MCP server](mcp.md) — running `rumb-mcp`, the available tools, and how to
  register the server in a repo.
- [Data model](data-model.md) — the `.rumb/` layout, DuckDB schema, IDs, and
  migrations.

## Background

- [Architecture](architecture.md) — crate layout, the two binaries, the shared
  core, the dev shims, and environment variables.
- [Development](development.md) — building, testing, and the release shims.
- [MVP plan](mvp-plan.md) — the original P0 design and milestones that shaped the
  current implementation.

## Status

Rumb is at version `0.1.0` and self-hosts: its own work is coordinated through
rumb. Out of scope for now: federation, approvals, auth, PR integration, a TUI,
daemon runners, tracked config, and importing external plan files.
