# CLI reference

The `rumb` binary is the operator and bootstrapping interface. Every command
except `init` discovers the project by walking up from the current directory to
find a `.rumb/` directory, so you can run rumb from anywhere inside the repo.

Output is tab-separated and line-oriented, intended to be both readable and easy
to pipe into other tools. On error, rumb prints `error: <message>` to stderr and
exits with status 1.

For the concepts behind these commands, see [Concepts](concepts.md). Most
mutating commands take `--actor <name>`; see [Actors](concepts.md#actors).

## Setup

### `rumb init --name <name>`

Initialize rumb state in the current directory. Creates `.rumb/state.duckdb`,
seeds the root item `RUMB-0000` with the given name, and adds `.rumb/` to
`.git/info/exclude`. Safe to re-run; it will not duplicate the root.

```text
$ rumb init --name my-project
initialized	my-project
```

### `rumb doctor`

Report on the local setup and exit non-zero if anything is wrong. Checks that the
state directory exists, the database is initialized at the current schema
version, and `.rumb/` is ignored by git.

```text
$ rumb doctor
state_dir	ok
state_file	ok
git_ignore	ok
ok
```

## Items

### `rumb item create`

```text
rumb item create --kind <kind> --title <title> --parent <id>
                 [--status draft|ready|blocked|in_review|done|superseded|abandoned]
                 [--tier routine|standard|hard]
                 [--source <ref>]
```

Create an item under `--parent`. `--kind` and `--title` must be non-empty. The
parent must exist. `--status` defaults to `draft`; `--tier` defaults to
`standard`. `--source` is an optional free reference (for example, where the item
came from). Prints `id  kind  status  title`.

### `rumb item status <id> <status> --actor <actor>`

Set an item's status directly. This does not validate the transition — see the
[lifecycle notes](concepts.md#status-lifecycle). Appends an `item.status` event.

### `rumb list`

Print all items as an ASCII tree under their parents. Items whose parent is
missing are listed separately as orphans. Each line is
`<id> [<kind> <status>] <title>`.

```text
$ rumb list
RUMB-0000 [project ready] my-project
|-- RUMB-0001 [feature ready] Implement claim flow
`-- RUMB-0002 [task draft] Write docs
```

### `rumb view item <reference>`

Print full detail for one item: its fields and depth, then sections for
children, incoming and outgoing edges, claims, proposals, runs, and events.
`<reference>` accepts `7`, `0007`, or `RUMB-0007`.

## Edges

### `rumb edge add <from> <to> --kind <kind>`

Add a typed edge between two existing items. `--kind` is one of `depends_on`,
`blocks`, `relates_to`, `supersedes`, `implements`. Edges are unique by
`(from, to, kind)`. See [Edges](concepts.md#edges) for how `depends_on` and
`blocks` affect readiness. Prints `from  to  kind`.

## Work selection and claims

### `rumb ready`

List items that can be claimed right now. See
[Readiness](concepts.md#readiness). Each line is `id  kind  status  title`.

### `rumb claim <id> --actor <actor> [--ttl 4h] [--confirm-foundation]`

Claim an item: lease it, create branch `rumb/<id>-<slug>`, check out a worktree
under `.rumb/worktrees/<id>-<slug>`, and open a proposal. `--ttl` defaults to
`4h` (see [TTL format](concepts.md#claims-and-leases)). Depth-1 (foundation)
items require `--confirm-foundation`; the root cannot be claimed. Prints
`claim-id  item-id  actor  status  branch  worktree-path`.

### `rumb renew <claim-id> --actor <actor> [--ttl 4h]`

Extend an active claim's lease by `--ttl` from now. Only the owning actor can
renew, and only while the claim is still active and unexpired.

### `rumb release <claim-id> --actor <actor>`

Release a claim early. The owning actor sets it to `released`. The item's status
is unchanged; with no active claim left it appears in `rumb ready` again.

## Grooming

These verbs reshape the graph after items exist. Each is recorded as an undoable
changeset. See [Grooming](concepts.md#grooming) for the full semantics.

### `rumb reparent <id> --under <parent> --actor <actor> [--confirm]`

Move an item under a new parent. Rejects cycles and is blocked while the item has
an active claim. `--confirm` is required when the move lands the item at depth 1
(directly under the root). Prints `id  parent-id  status  title`.

### `rumb edit <id> [--title <title>] [--source <ref>] [--tier <tier>] --actor <actor>`

Set the item's title, source reference, and/or tier; at least one is required. A
non-empty title is enforced. `--tier` is one of `routine`, `standard`, `hard`.
Prints `id  kind  status  title`.

### `rumb recast <id> --kind <kind> --actor <actor>`

Change an item's kind. `--kind` must be non-empty; the root cannot be recast.
Prints `id  kind  status  title`.

### `rumb unlink <from> <to> --kind <kind> --actor <actor>`

Remove a graph edge. If removing a `depends_on`/`blocks` edge unblocks work, the
newly ready items are listed. Prints `unlinked  from  to  kind`, then a
`ready  id  kind  title` line per newly ready item.

### `rumb merge <from> --into <to> --actor <actor>`

Fold `from` into `to`: reparent `from`'s children under `to`, rewire `from`'s
edges to `to` (de-duplicated), record a `supersedes` edge `to -> from`, and mark
`from` superseded (never deleted). Blocked while `from` has an active claim; the
root cannot be merged away. Prints a `merged` line, a `moved` line per reparented
child, and a `supersedes` line.

## Capture

### `rumb capture "<text>"`

Drop a quick thought into the inbox as a draft `note` — no kind decision required
at intake. The full text is stored in the item's `body`; the `title` is a
whitespace-collapsed, truncated one-line summary. Captures land as `draft`, so
they do not appear in `rumb ready` until you groom them out of the inbox
(`reparent`/`recast`/`edit`). Prints `id  kind  status  title`.

## Verification

### `rumb run <id> --actor <actor> -- <command...>`

Run a command from the repository root, capture its output to
`.rumb/runs/<run-id>.log`, and record `passed`/`failed`. Everything after `--` is
the command. The item must exist but need not be claimed. Prints
`run-id  item-id  status  output-path`.

```text
$ rumb run RUMB-0001 --actor operator -- cargo test
RUN-0001	RUMB-0001	passed	.rumb/runs/RUN-0001.log
```

### `rumb review <id> --actor <actor>`

Move an item to `in_review`. Appends an `item.review` event.

### `rumb done <id> --actor <actor>`

Mark an item `done`. Appends an `item.done` event. Completing an item can make
its dependents ready.

## Event log

### `rumb log [<id>]`

Print events in sequence order. With an `<id>`, filter to events whose
`object_id` matches. Each line is
`timestamp  action  object_type  object_id  payload`. Note that edge events use
`from->to` as their object ID, so they do not appear under an item's filtered
log.

## MCP

### `rumb mcp serve`

Launch the MCP stdio server. This is a thin launcher that locates and execs the
`rumb-mcp` binary (see [resolution order](mcp.md#how-rumb-mcp-serve-finds-the-server)).
It is the command registered in `.mcp.json`.

### `rumb mcp install`

```text
rumb mcp install [--name rumb] [--command <path>] [--target .mcp.json] [--force]
```

Write (or update) an MCP server entry so a client can launch rumb. By default it
adds a `rumb` server to `.mcp.json` at the project root that runs
`<command> mcp serve`. The command path is chosen automatically: `bin/rumb` when
invoked through the repo shim, an installed external shim's path otherwise, or
`rumb` on `PATH` as a fallback. Use `--force` to replace an existing entry,
`--name` to install under a different server name, `--command` to override the
command, and `--target` to write to a different file. See
[MCP server](mcp.md#registering-the-server) for details.
