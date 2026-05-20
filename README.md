# Rumb MVP Plan: Self-Hosting Agent Work Coordinator

## Summary
Build `rumb` as a small Rust + DuckDB local coordinator for one repo, one operator, and many agents. P0 ships a CLI and MCP server over the same core library, with local state in `.rumb/state.duckdb`.

The MVP is complete when `rumb` can initialize its own repo, seed its post-P0 work graph through `rumb item create`, let an agent claim a ready item, create a git branch/worktree under `.rumb/worktrees`, run verification, and record the full event trail. From that point onward, rumb development uses rumb.

Out of scope for P0: federation, approvals, auth, PR integration, TUI, daemon runners, tracked config, importing markdown/TOML plans.

## Local Build and MCP Install

Rumb ships two binaries:

```text
rumb      # CLI
rumb-mcp  # standalone MCP stdio server
```

For local development, use the repo shims. They compile the needed debug binary on first use:

```text
bin/rumb doctor
bin/rumb mcp serve
bin/rumb-mcp
```

To build both binaries directly:

```text
scripts/build.sh
scripts/build.sh --release
```

To install release shims into a prefix:

```text
scripts/install.sh
scripts/install.sh /opt/homebrew
```

To register rumb as a project MCP server, run this from the repository:

```text
bin/rumb mcp install
```

That writes `.mcp.json` with a `rumb` MCP server entry that runs `rumb mcp serve`. When invoked through `bin/rumb`, the command path is recorded as `bin/rumb`; when invoked through an installed external shim, that shim path is recorded; otherwise it falls back to `rumb` on `PATH`. Use `--force` to replace an existing `rumb` entry, `--name <name>` to install under another MCP server name, or `--command <path>` to override the command path.

## Execution Milestones

The shortest path is to build only enough first-generation rumb to load and inspect work items, then use that to seed the rest of P0. Do not push to origin until Milestone 2 is passing locally and the first post-P0 work graph has been loaded through rumb itself.

### Milestone 1: Seedable CLI

Goal: initialize local state and manually load the P0/post-P0 work graph.

Required commands:

```text
rumb init --name rumb
rumb doctor
rumb item create --kind <kind> --title <title> --parent <id> [--status draft|ready] [--source <ref>]
rumb edge add <from> <to> --kind <depends_on|blocks|relates_to|supersedes|implements>
rumb ready
rumb log [<id>]
```

Delegation steps:

1. Core/state context, high effort: scaffold the Rust crate, add DuckDB, implement schema initialization, migrations, transactions, ID allocation, item/edge/event APIs, depth calculation, and dependency-aware ready queries.
2. CLI context, medium effort: add the `rumb` binary and wire `init`, `doctor`, `item create`, `edge add`, `ready`, and `log` to the shared core.
3. Integration test context, medium effort: test in temporary git repos that `init` creates `.rumb/state.duckdb`, `.rumb/` is ignored, item creation works, dependencies affect readiness, and events appear in `log`.

Acceptance:

```text
cargo build
./target/debug/rumb init --name rumb
./target/debug/rumb item create --kind feature --title "Implement claim flow" --status ready --parent RUMB-0000
./target/debug/rumb ready
./target/debug/rumb log
```

After this milestone, seed all remaining P0 work through `rumb item create` and `rumb edge add`.

### Milestone 2: Self-Hosting Minimum

Goal: claim real work in local branches/worktrees and record verification.

Required commands:

```text
rumb claim <id> --actor <actor> [--ttl 4h] [--confirm-foundation]
rumb renew <claim-id> --actor <actor> [--ttl 4h]
rumb release <claim-id> --actor <actor>
rumb review <id> --actor <actor>
rumb done <id> --actor <actor>
rumb run <id> --actor <actor> -- <command...>
```

Delegation steps:

1. Claim/worktree context, high effort: implement transactional claim exclusivity, TTL expiry, branch creation, worktree creation under `.rumb/worktrees`, proposal creation, and claim events.
2. Run/review lifecycle context, medium effort: implement `run`, captured stdout/stderr logs under `.rumb/runs`, `review`, `done`, lifecycle validation, and mutation events.
3. Concurrency/event test context, high effort: test second active claim failure, expired claim readiness, renewal, release, run pass/fail recording, and event coverage for every mutation.

Acceptance:

```text
./target/debug/rumb claim <ready-id> --actor operator
./target/debug/rumb run <ready-id> --actor operator -- cargo test
./target/debug/rumb review <ready-id> --actor operator
./target/debug/rumb done <ready-id> --actor operator
```

After this milestone, rumb development should happen from rumb-created branches/worktrees. This is the first point where pushing to origin can be reconsidered.

### Milestone 3: MCP Parity

Goal: agents use MCP tools for the same operations the operator can perform through the CLI.

Delegation steps:

1. MCP server context, high effort: generate the `rumb-mcp` starting point with `rmcp create`, expose JSON tools mirroring the CLI verbs, and keep all behavior in the shared core library.
2. MCP smoke test context, medium effort: start `rumb-mcp`, create an item, query ready work, claim an item, record a run, and inspect logs/events.

Acceptance:

```text
rumb-mcp can create, ready, claim, run, and log through structured JSON tool calls.
```

## Key Interfaces
Use one Rust crate with shared core logic plus two binaries: `rumb` and `rumb-mcp`. Generate the MCP starting point with `rmcp create`, but keep all behavior in the shared core.

CLI commands:

```text
rumb init --name rumb
rumb doctor

rumb item create --kind <kind> --title <title> --parent <id> [--status draft|ready] [--source <ref>]
rumb item status <id> <draft|ready|blocked|claimed|in_review|done|superseded|abandoned> --actor <actor>
rumb edge add <from> <to> --kind <depends_on|blocks|relates_to|supersedes|implements>

rumb ready
rumb claim <id> --actor <actor> [--ttl 4h] [--confirm-foundation]
rumb renew <claim-id> --actor <actor> [--ttl 4h]
rumb release <claim-id> --actor <actor>
rumb review <id> --actor <actor>
rumb done <id> --actor <actor>

rumb run <id> --actor <actor> -- <command...>
rumb log [<id>]
```

MCP tools mirror the CLI verbs with JSON arguments and structured JSON results. Agents should use MCP when available; the CLI remains valid for operators and bootstrapping.

## Core Model
Store all mutable coordination state in `.rumb/state.duckdb`. `rumb init` must ensure `.rumb/` is ignored, preferably through `.git/info/exclude`; `rumb doctor` must flag any `.rumb` path that git treats as trackable.

Entities:

```text
item
  id: RUMB-0000 some sequential style sequence, prefixed or not
  parent_id: primary tree parent
  kind: required non-empty string
  title, status, source_ref, timestamps

edge
  from_item, to_item, kind

claim
  item_id, actor_id, lease_until, status, branch, worktree_path

proposal
  item_id, branch, base_ref, head_ref, status

run
  item_id, proposal_id, command, status, output_path, timestamps

event
  actor_id, action, object_type, object_id, payload_json, timestamp
```

`RUMB-0000` is the project root and is not claimable. Depth is computed from the primary parent chain. Cross-links live in `edge`; they do not affect depth.

Use a common work item lifecycle for P0: `draft`, `ready`, `blocked`, `claimed`, `in_review`, `done`, `superseded`, `abandoned`. Kinds are flexible; document these initial conventions: `principle`, `adr`, `spec`, `feature`, `task`, `bug`, `test`, `chore`.

Claim rules:

- `rumb ready` returns items with status `ready`, satisfied `depends_on` edges, and no active unexpired claim.
- `rumb claim` is transactional: lease item, create branch `rumb/<id>-<slug>`, create worktree `.rumb/worktrees/<id>-<slug>`, create proposal, append event.
- Depth `0` cannot be claimed. Depth `1` requires `--confirm-foundation`. Depth `2+` is claimable normally.
- Expired claims no longer block readiness; renewal appends an event.

Runs:

- `rumb run <id> -- <cmd>` executes locally, captures stdout/stderr to `.rumb/runs/<run-id>.log`, records status, and appends events.
- No daemon in P0.

## Test Plan

Unit tests:

- schema initialization and migrations
- ID allocation
- depth calculation from parent tree
- dependency readiness queries
- lifecycle transitions
- claim exclusivity and TTL expiry
- event creation for every mutation

Integration tests in temporary git repos:

- `rumb init` creates DB and ignores `.rumb/`
- `rumb doctor` catches unignored `.rumb/`
- `rumb claim` creates branch and `.rumb/worktrees/...`
- second claim on same active item fails
- expired claim makes item ready again
- `rumb run` records passed/failed commands and log paths

MCP smoke tests:

- start `rumb-mcp`
- create item
- query ready work
- claim item
- record run
- inspect log/events

Dogfood acceptance:

1. Build P0 manually.
2. Run `rumb init --name rumb`.
3. Seed the next rumb work graph using only `rumb item create` and `rumb edge add`.
4. Claim the first post-P0 item through rumb.
5. Use the generated branch/worktree for implementation.
6. Record verification with `rumb run`.
7. Mark review/done through rumb after operator review.

## Assumptions

- `.rumb/` is local runtime state and never repo canon.
- There is no tracked `rumb.toml` in P0.
- Actor IDs are explicit strings such as `operator`, `codex-a`, or `rebotica-qwen`; auth comes later.
- DuckDB is the single state store; writes use transactions and short retry-on-busy behavior.
- GitHub/GitLab PRs are external for P0; rumb records branch/worktree/proposal state only.
