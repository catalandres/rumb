# Getting started

This walks through installing rumb, initializing a repository, and running one
full work cycle: create an item, claim it into a worktree, record a verification
run, and mark it done.

## Prerequisites

- A recent stable Rust toolchain (`cargo`).
- `git` on `PATH`. Rumb shells out to git for worktrees and for ignore checks.

DuckDB is vendored (the `bundled` feature), so there is nothing else to install.

## Build

Rumb ships two binaries:

```text
rumb      # CLI
rumb-mcp  # MCP stdio server
```

For local development, use the repo shims. They compile the needed debug binary
on first use:

```sh
bin/rumb doctor
bin/rumb-mcp
```

To build both binaries directly:

```sh
scripts/build.sh            # debug
scripts/build.sh --release  # release
```

To install release shims into a prefix (`$HOME/.local` by default):

```sh
scripts/install.sh
scripts/install.sh /opt/homebrew
```

See [Development](development.md) for what the shims and scripts do.

## Initialize a repository

Run `init` from the root of a git repository:

```sh
rumb init --name my-project
```

This creates `.rumb/state.duckdb`, seeds the project root item `RUMB-0000`, and
adds `.rumb/` to `.git/info/exclude` so coordination state is never committed.

Verify the setup:

```sh
rumb doctor
```

`doctor` checks that the state directory and database exist and that `.rumb/` is
ignored by git. It exits non-zero if any check fails.

## Create and inspect work

Items form a tree under `RUMB-0000`. Create one and list the tree:

```sh
rumb item create --kind feature --title "Implement claim flow" --parent RUMB-0000 --status ready
rumb list
```

`ready` shows the items that can be claimed right now — `ready` status, all
dependencies satisfied, and no active claim:

```sh
rumb ready
```

You can wire up dependencies between items so readiness follows the graph:

```sh
rumb edge add RUMB-0002 RUMB-0001 --kind depends_on   # RUMB-0002 waits for RUMB-0001
```

## Claim work into a worktree

Claiming leases an item to an actor, creates a branch `rumb/<id>-<slug>`, and
checks out a git worktree under `.rumb/worktrees/<id>-<slug>`:

```sh
rumb claim RUMB-0001 --actor operator
```

Notes:

- The project root `RUMB-0000` cannot be claimed.
- Direct children of the root (depth 1, "foundation" items) require
  `--confirm-foundation`.
- A lease defaults to `4h`; pass `--ttl 30m`, `--ttl 2h`, etc. Renew with
  `rumb renew <claim-id> --actor operator` or give it up with
  `rumb release <claim-id> --actor operator`.

## Record verification

Run a command and capture its result. Output is logged to
`.rumb/runs/<run-id>.log` and the pass/fail status is recorded as an event:

```sh
rumb run RUMB-0001 --actor operator -- cargo test
```

## Move through review and done

```sh
rumb review RUMB-0001 --actor operator
rumb done   RUMB-0001 --actor operator
```

Marking an item `done` can unblock dependents: an item that `depends_on` it
becomes ready once it is done.

## See what happened

Every mutation appends an event. Inspect the whole log or just one item:

```sh
rumb log              # all events
rumb log RUMB-0001    # events for one item
rumb view item RUMB-0001   # full detail: children, edges, claims, runs, events
```

## Next steps

- [Concepts](concepts.md) explains the model behind these commands.
- [CLI reference](cli.md) documents every command and flag.
- [MCP server](mcp.md) shows how agents drive the same operations.
