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
this list.

## Status lifecycle

Items move through a common lifecycle:

```text
draft → ready → claimed → in_review → done
                   │
                   └→ blocked, superseded, abandoned
```

The full set of statuses is `draft`, `ready`, `blocked`, `claimed`, `in_review`,
`done`, `superseded`, and `abandoned`.

The lifecycle is mostly **advisory**. Only two operations enforce status rules:
readiness (which item shows up in `rumb ready`) and claiming. The dedicated
verbs `review`, `done`, and `item status` set the status directly without
validating the transition, so the lifecycle is a convention you follow, not a
state machine the tool polices.

Some status changes happen automatically:

- A successful `claim` moves the item to `claimed`.
- A failed claim (e.g. git worktree creation fails) restores it to `ready`.
- Releasing the last active claim restores a `claimed` item to `ready`.

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
3. Its status is `ready` or `claimed`.
4. Every dependency is satisfied: each `depends_on` target is `done`, and each
   `blocks` predecessor is `done`.

Because `claimed` items with no *active* claim still qualify, an item whose claim
lease has expired automatically becomes ready again.

## Actors

An actor is an explicit string identity such as `operator`, `codex-a`, or
`rebotica-qwen`. Actors are passed on every mutating command (`--actor`) and
recorded in events. There is no authentication; actor identity is by convention.

## Claims and leases

A claim is an exclusive, time-bounded lease on an item by an actor. Claiming is
transactional and proceeds in two phases:

1. **Reserve** (one transaction): validate the item (exists, not root, depth and
   dependency rules, no active claim, claimable status), allocate a claim
   (`CLAIM-0001`, …) and a proposal (`PROP-0001`, …), compute the branch
   `rumb/<id>-<slug>` and worktree path `.rumb/worktrees/<id>-<slug>`, set the
   item to `claimed`, and append a `claim.reserve` event. The claim starts
   `pending`.
2. **Create the worktree**: run `git worktree add -b <branch> <worktree>`.
   - On success the claim becomes `active`, the proposal becomes `open`, and a
     `claim.create` event is appended.
   - On failure the claim becomes `failed`, the proposal becomes `failed`, the
     item is restored to `ready`, and a `claim.failed` event records the reason.

Claim statuses are `pending`, `active`, `released`, and `failed`.

Lease management:

- **Default TTL** is 4 hours. TTLs are written like `90s`, `30m`, `4h`, `2d`; a
  bare number is seconds. Zero or unparseable values are rejected.
- **Renew** extends the lease. Only the owning actor can renew, and only while
  the claim is still active and unexpired.
- **Release** ends a claim early. The owning actor sets it to `released`, the
  matching proposal becomes `released`, and if no other active claim remains the
  item returns to `ready`.
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
```

`rumb log` prints events in sequence order. `rumb log <id>` filters by
`object_id`. Most item-related events (claims, runs, reviews) use the item ID as
`object_id`, so they appear under the item. Edge events use `from->to` as their
`object_id`, so they do not appear in an item's filtered log.
