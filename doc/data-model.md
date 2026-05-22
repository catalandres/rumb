# Data model

All mutable coordination state lives under `.rumb/` in a single DuckDB database.
The directory is local runtime state, never repo canon — `init` adds it to
`.git/info/exclude` and `doctor` flags it if git would track it.

## On-disk layout

```text
.rumb/
├── state.duckdb                      # all items, edges, claims, proposals, runs, events
├── worktrees/
│   └── <id>-<slug>/                  # git worktree created per claim
└── runs/
    └── <run-id>.log                  # captured stdout/stderr per run
```

A run log is a small text file:

```text
command	cargo test
status	passed
exit_code	0

[stdout]
…captured stdout…

[stderr]
…captured stderr…
```

When a command cannot be spawned at all, `status` is `failed`, `exit_code` is
`unknown`, and the spawn error is written to the `[stderr]` section.

## Identifiers

| Entity | Pattern | Notes |
| --- | --- | --- |
| Item | `RUMB-0000`, `RUMB-0001`, … | `RUMB-0000` is the root; four-digit zero-padded. |
| Claim | `CLAIM-0001`, … | Sequential per repo. |
| Proposal | `PROP-0001`, … | Sequential per repo. |
| Run | `RUN-0001`, … | Sequential per repo; stays sequential even after spawn failures. |

Item references accept `7`, `0007`, `rumb-7`, or `RUMB-0007`. Branches are
`rumb/<id>-<slug>` and worktrees `.rumb/worktrees/<id>-<slug>`, where the slug is
a lowercased, alphanumeric-and-dash form of the title, capped at 48 characters
(falling back to `item` if empty).

All timestamps are Unix seconds.

## Schema

The database is created and upgraded via a `migrations` table. The current schema
version is **3**, applied as three migrations:

| Version | Name | Creates |
| --- | --- | --- |
| 1 | `milestone_1_state` | `items`, `edges`, `events` |
| 2 | `claim_worktree_state` | `claims`, `proposals` |
| 3 | `run_lifecycle_state` | `runs` |

Migrations are idempotent: `ensure_schema` runs on every connection and applies
only the versions not already recorded.

### Tables

```sql
migrations (
  version    INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  applied_at BIGINT NOT NULL
)

items (
  id         TEXT PRIMARY KEY,        -- RUMB-NNNN
  parent_id  TEXT,                    -- primary tree parent; NULL only for the root
  kind       TEXT NOT NULL,
  title      TEXT NOT NULL,
  status     TEXT NOT NULL,           -- draft|ready|blocked|in_review|done|superseded|abandoned
  tier       TEXT,                    -- routine|standard|hard (default 'standard'; never null in practice)
  source_ref TEXT,
  body       TEXT,                    -- full text of a captured note; null for most items
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL
)

meta (
  key   TEXT PRIMARY KEY,             -- e.g. 'inbox_id'
  value TEXT NOT NULL
)

edges (
  from_item  TEXT NOT NULL,
  to_item    TEXT NOT NULL,
  kind       TEXT NOT NULL,           -- depends_on|blocks|relates_to|supersedes|implements
  created_at BIGINT NOT NULL,
  PRIMARY KEY (from_item, to_item, kind)
)

claims (
  id            TEXT PRIMARY KEY,     -- CLAIM-NNNN
  item_id       TEXT NOT NULL,
  actor_id      TEXT NOT NULL,
  lease_until   BIGINT NOT NULL,      -- Unix seconds; expiry evaluated at read time
  status        TEXT NOT NULL,        -- pending|active|released|failed
  branch        TEXT NOT NULL,        -- rumb/<id>-<slug>
  worktree_path TEXT NOT NULL,        -- .rumb/worktrees/<id>-<slug>
  created_at    BIGINT NOT NULL,
  updated_at    BIGINT NOT NULL
)

proposals (
  id         TEXT PRIMARY KEY,        -- PROP-NNNN
  item_id    TEXT NOT NULL,
  branch     TEXT NOT NULL,
  base_ref   TEXT NOT NULL,           -- branch current when the claim was reserved
  head_ref   TEXT,
  status     TEXT NOT NULL,           -- pending|open|released|failed
  created_at BIGINT NOT NULL,
  updated_at BIGINT NOT NULL
)

runs (
  id          TEXT PRIMARY KEY,       -- RUN-NNNN
  item_id     TEXT NOT NULL,
  proposal_id TEXT,                   -- the item's latest proposal at run time, if any
  command     TEXT NOT NULL,          -- space-joined argv
  status      TEXT NOT NULL,          -- running|passed|failed
  output_path TEXT NOT NULL,          -- .rumb/runs/<run-id>.log
  started_at  BIGINT NOT NULL,
  finished_at BIGINT                  -- NULL while running
)

events (
  seq          BIGINT PRIMARY KEY,    -- monotonic, assigned at append time
  timestamp    BIGINT NOT NULL,
  action       TEXT NOT NULL,
  object_type  TEXT NOT NULL,         -- project|item|edge
  object_id    TEXT NOT NULL,         -- item ID for most actions; from->to for edges
  payload_json TEXT NOT NULL          -- action-specific JSON
)
```

## Event actions

Events are append-only and ordered by `seq`. The actions rumb emits:

| Action | When | `object_id` |
| --- | --- | --- |
| `init` | repository initialized | `RUMB-0000` |
| `item.create` | item created | item ID |
| `item.status` | status set via `item status` | item ID |
| `edge.add` | edge added | `from->to` |
| `claim.reserve` | claim reserved (phase 1) | item ID |
| `claim.create` | claim activated after worktree creation | item ID |
| `claim.failed` | worktree creation failed, claim rolled back | item ID |
| `claim.renew` | lease extended | item ID |
| `claim.release` | claim released | item ID |
| `run.record` | run finished and recorded | item ID |
| `item.review` | item moved to review | item ID |
| `item.done` | item marked done | item ID |
| `item.reparent` | item moved under a new parent (undoable) | item ID |
| `item.edit` | title/source edited (undoable) | item ID |
| `item.recast` | kind changed (undoable) | item ID |
| `item.merge` | item merged into another, superseded (undoable) | item ID (the source) |
| `edge.unlink` | edge removed (undoable) | `from->to` |
| `item.capture` | note captured into the inbox (undoable) | item ID |

## Concurrency and durability

Writes run inside DuckDB transactions. Busy/locked/conflict errors are retried
with exponential backoff (a few attempts, starting at 25 ms). Claim exclusivity
is enforced inside the reserve transaction, so two actors racing for the same
item produce exactly one active claim and one failure.
