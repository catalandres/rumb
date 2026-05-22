# Concepts

Rumb models work as a graph of **items** with a primary parent tree plus
cross-links (**edges**). Agents and operators (**actors**) take **claims** on
ready items, which produce git **proposals** (branch + worktree) and verification
**runs**. Every state change appends an **event**. All of it lives in one DuckDB
file; see [Data model](data-model.md) for the physical schema.

## Items and the tree

Each item has an ID like `RUMB-0042`, a primary `parent_id`, a free-form `kind`,
a `title`, a `status`, an optional `source_ref`, and timestamps.

- `RUMB-0000` is the project root, created by `init`. It has no parent and is
  never claimable.
- IDs are allocated sequentially with four-digit zero padding (`RUMB-0001`,
  `RUMB-0002`, …).
- Item references are forgiving on input: `7`, `0007`, `rumb-7`, and `RUMB-0007`
  all resolve to `RUMB-0007`.

### Depth

Depth is the number of hops from an item up to the root along the primary parent
chain:

- **Depth 0** — the root (`RUMB-0000`). Not claimable.
- **Depth 1** — "foundation" items directly under the root. Claimable only with
  `--confirm-foundation`, because they are broad and easy to claim by accident.
- **Depth 2+** — claimable normally.

Cross-links (edges) do not affect depth; only the parent chain does.

### Kinds

`kind` is any non-empty string. The conventional set is `principle`, `adr`,
`spec`, `feature`, `task`, `bug`, `test`, and `chore`, but rumb does not enforce
this list. Two kinds are used structurally: `project` (the root) and `inbox` (the
capture inbox). Captured notes get kind `note`.

### Tier

Every item has a `tier` — a work-weight signal that is a property of the work
itself, not a model name: `routine`, `standard` (the default), or `hard`. Set it
at `item create` or via `edit`. Tier is displayed in `ready` and `view`; it does
not gate readiness or claiming today (tier-aware dispatch is a later cycle).

### Body

`body` is optional free text holding the full content of a captured note (see
[Capture](#capture)). Most items leave it empty; the one-line `title` is the
summary.

## Status lifecycle

Items move through a common lifecycle:

```text
draft → ready → in_review → done
          │
          └→ blocked, superseded, abandoned
```

The full set of statuses is `draft`, `ready`, `blocked`, `in_review`, `done`,
`superseded`, and `abandoned`.

Claiming is **not** a status. A claim is a separate, time-bounded lease (see
[Claims and leases](#claims-and-leases)); an item being actively worked stays
`ready` and is simply hidden from `rumb ready` while the lease holds. This keeps
"what kind of thing is this and where is it in its life" (status) independent of
"is someone working it right now" (an active claim).

The lifecycle is mostly **advisory**. Only two operations enforce status rules:
readiness (which item shows up in `rumb ready`) and claiming. The dedicated
verbs `review`, `done`, and `item status` set the status directly without
validating the transition, so the lifecycle is a convention you follow, not a
state machine the tool polices.

Claims never change an item's status: reserving, releasing, or failing a claim
leaves the status untouched. An item leaves `rumb ready` while a claim is active
and reappears when the claim is released or its lease expires — purely a
read-time effect, with no status write.

## Edges

Edges are typed cross-links between items: `from_item → to_item` with a `kind` of
`depends_on`, `blocks`, `relates_to`, `supersedes`, or `implements`. An edge is
unique by `(from, to, kind)`.

Two kinds affect readiness:

- **`depends_on`** — `A depends_on B` means **A waits for B**. A is not ready
  until B is `done`.
- **`blocks`** — `A blocks B` means **B waits for A**. B is not ready until A is
  `done`.

`relates_to`, `supersedes`, and `implements` are informational and do not gate
readiness.

## Readiness

`rumb ready` returns items that are claimable right now. An item is ready when
**all** of these hold:

1. It is not the root.
2. It has no active, unexpired claim.
3. Its status is `ready`.
4. Every dependency is satisfied: each `depends_on` target is `done`, and each
   `blocks` predecessor is `done`.

Because an active claim is the only thing rule 2 checks, an item whose claim
lease has expired automatically becomes ready again — its status was `ready` all
along.

## Grooming

The graph is reshapeable. Five grooming verbs change structure and metadata after
an item is created; each is recorded as an undoable changeset.

- **`reparent <id> --under <parent>`** — move an item to a new parent. Rejects
  cycles, requires `--confirm` when the move lands the item at depth 1 (the same
  guard as claiming a foundation item), and is blocked while the item has an
  active claim.
- **`edit <id> [--title …] [--source …]`** — set the title and/or source
  reference. At least one field is required.
- **`recast <id> --kind <kind>`** — change an item's kind (a task that matures
  into a spec). Kind stays free-form; only non-empty is enforced.
- **`unlink <from> <to> --kind <kind>`** — remove a graph edge. Removing a
  `depends_on`/`blocks` edge can unblock work, so `unlink` reports any items that
  became ready as a result.
- **`merge <from> --into <to>`** — fold one item into another: `from`'s children
  reparent under `to`, `from`'s edges rewire to `to` (de-duplicated), a
  `supersedes` edge `to → from` is recorded, and `from` is set to `superseded`.
  `from` is never deleted, so its claims, proposals, and runs stay valid history.
  Blocked while `from` has an active claim.

The project root and the inbox are **reserved nodes**: they cannot be reparented,
recast, or merged away, and they never appear in `ready`. Editing the root
(renaming the project) is allowed.

## Inbox and capture

Every project has an **inbox**: a normal item with kind `inbox`, seeded as a
direct child of the root. It is a regular numeric `RUMB-NNNN` item (so it resolves
through the usual reference forms), found internally by an `inbox_id` entry in the
`meta` table rather than a hardcoded id. The inbox is created at `init`, and an
existing repo that predates the inbox gets one via a schema migration.

`rumb capture "<text>"` drops a thought into the inbox with no "what kind of thing
is this?" freeze: it creates a `note` with status `draft` and tier `standard`. The
full text is stored in `body`; the `title` is a clean one-line summary (whitespace
collapsed, truncated). Because captures are `draft`, they never show up in `ready`
until you groom them — reparent them out of the inbox, `recast` them into a real
kind, or `edit` them. That is the capture-then-groom loop.

## Actors

An actor is an explicit string identity such as `operator`, `codex-a`, or
`rebotica-qwen`. Actors are passed on every mutating command (`--actor`) and
recorded in events. There is no authentication; actor identity is by convention.

## Claims and leases

A claim is an exclusive, time-bounded lease on an item by an actor. Claiming is
transactional and proceeds in two phases:

1. **Reserve** (one transaction): validate the item (exists, not root, depth and
   dependency rules, no active claim, `ready` status), allocate a claim
   (`CLAIM-0001`, …) and a proposal (`PROP-0001`, …), compute the branch
   `rumb/<id>-<slug>` and worktree path `.rumb/worktrees/<id>-<slug>`, and append
   a `claim.reserve` event. The item's status is left unchanged. The claim starts
   `pending`.
2. **Create the worktree**: run `git worktree add -b <branch> <worktree>`.
   - On success the claim becomes `active`, the proposal becomes `open`, and a
     `claim.create` event is appended.
   - On failure the claim becomes `failed`, the proposal becomes `failed`, and a
     `claim.failed` event records the reason.

Claim statuses are `pending`, `active`, `released`, and `failed`.

Lease management:

- **Default TTL** is 4 hours. TTLs are written like `90s`, `30m`, `4h`, `2d`; a
  bare number is seconds. Zero or unparseable values are rejected.
- **Renew** extends the lease. Only the owning actor can renew, and only while
  the claim is still active and unexpired.
- **Release** ends a claim early. The owning actor sets it to `released` and the
  matching proposal becomes `released`. The item's status is untouched; with no
  active claim left it simply shows up in `rumb ready` again.
- **Expiry** is implicit: an expired claim no longer blocks readiness or new
  claims. There is no daemon that reaps expired claims; expiry is evaluated at
  read time against the current clock.

Only one active, unexpired claim can exist per item; a second concurrent claim
fails with the active claim's ID.

## Proposals

A proposal records the git intent behind a claim: `item_id`, `branch`,
`base_ref` (the branch that was current when the claim was reserved), an optional
`head_ref`, and a `status`. Proposal status tracks the claim: `pending` →
`open` on activation, `released` on release, `failed` on a failed claim. Rumb
records branch/worktree/proposal state only; it does not open PRs.

## Runs

A run executes a command locally and records the result. `rumb run <id> --
<cmd...>` does not require the item to be claimed — it only requires the item to
exist. A run:

- Allocates a run ID (`RUN-0001`, …) and associates it with the item's latest
  proposal, if any.
- Executes the command from the repository root, capturing stdout and stderr.
- Writes a log to `.rumb/runs/<run-id>.log` containing the command, status, exit
  code, and captured output.
- Records status `passed` (exit 0) or `failed` (non-zero, or the command could
  not be spawned) and appends a `run.record` event.

There is no daemon; runs are synchronous and local.

## Events

Every mutation appends an immutable event with a monotonic sequence number, a
timestamp, an `action`, an `object_type`, an `object_id`, and a JSON payload.
The actions rumb emits are:

```text
init          item.create   item.status   edge.add
claim.reserve claim.create  claim.failed  claim.renew  claim.release
run.record    item.review   item.done
item.reparent item.edit     item.recast   item.merge   edge.unlink
item.capture
```

Grooming events (`item.reparent`, `item.edit`, `item.recast`, `item.merge`, and
`edge.unlink`) are recorded as **undoable** changesets; lifecycle events above
them are not. `edge.unlink` uses `from->to` as its `object_id` (like `edge.add`),
so it does not appear in an item's filtered log.

`rumb log` prints events in sequence order. `rumb log <id>` filters by
`object_id`. Most item-related events (claims, runs, reviews) use the item ID as
`object_id`, so they appear under the item. Edge events use `from->to` as their
`object_id`, so they do not appear in an item's filtered log.
