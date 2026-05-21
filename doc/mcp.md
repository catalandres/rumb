# MCP server

`rumb-mcp` is an [MCP](https://modelcontextprotocol.io) server that exposes
rumb's operations as structured JSON tools over stdio. It is built on the same
shared core as the CLI, so behavior is identical — agents should prefer MCP when
available, while the CLI remains the operator and bootstrapping interface.

The server discovers the project by walking up from its current working directory
to find `.rumb/` (except the `init` tool, which initializes the current
directory). Run the client with its working directory set to somewhere inside the
target repository.

## Tools

Each tool mirrors a CLI verb. Arguments are JSON; results are returned as
structured JSON content.

| Tool | Arguments | Description |
| --- | --- | --- |
| `init` | `name` | Initialize rumb state in the current repository. |
| `doctor` | — | Run the setup checks; returns each check plus `ok`. |
| `item_create` | `kind`, `title`, `parent`, `status?`, `source?` | Create an item. `status` defaults to `draft`. |
| `item_status` | `id`, `status`, `actor` | Set an item's status. |
| `edge_add` | `from`, `to`, `kind` | Add a typed edge. |
| `ready` | — | List currently ready items. |
| `claim` | `id`, `actor`, `ttl?`, `confirm_foundation?` | Claim an item and create its worktree. |
| `renew` | `claim_id`, `actor`, `ttl?` | Renew an active claim's lease. |
| `release` | `claim_id`, `actor` | Release a claim. |
| `run` | `id`, `actor`, `command` (array) | Run a command and record the result. |
| `review` | `id`, `actor` | Move an item into review. |
| `done` | `id`, `actor` | Mark an item done. |
| `log` | `id?` | List events, optionally scoped to one item. |

Notes:

- `ttl` accepts the same format as the CLI (`90s`, `30m`, `4h`, `2d`, or bare
  seconds) and defaults to 4 hours when omitted.
- `command` is an array of strings, e.g. `["cargo", "test"]`.
- The CLI-only views `list` and `view item` are **not** exposed as MCP tools. Use
  `ready` and `log` for structured reads.

See [Concepts](concepts.md) for the meaning of these operations and
[CLI reference](cli.md) for the equivalent commands.

## Result shapes

Tools return structured JSON. The common shapes are:

- **item** — `{ id, kind, status, title, parent_id, source_ref }`
- **edge** — `{ from, to, kind, created_at }`
- **claim** — `{ id, item_id, actor_id, status, branch, worktree_path, lease_until }`
- **run** — `{ id, item_id, status, output_path }`
- **event** — `{ timestamp, action, object_type, object_id, payload }`, where
  `payload` is parsed JSON (or `{ "raw": "…" }` if it was not valid JSON)
- `ready` returns `{ items: [item, …] }`; `log` returns `{ events: [event, …] }`

Errors are returned as MCP errors carrying the underlying rumb message.

## Registering the server

`rumb mcp install` writes an MCP server entry into a config file (default
`.mcp.json` at the project root):

```sh
rumb mcp install
```

This produces an entry like:

```json
{
  "mcpServers": {
    "rumb": {
      "command": "bin/rumb",
      "args": ["mcp", "serve"]
    }
  }
}
```

The `command` is chosen automatically:

- `bin/rumb` (relative to the repo root) when you run `install` through the repo
  shim — this is what the checked-in `.mcp.json` uses for local development.
- An installed external shim's path, if rumb was invoked through one.
- `rumb` on `PATH` as a fallback.

Existing entries and other servers in the file are preserved. Installing over an
existing `rumb` entry requires `--force`.

Options:

- `--name <name>` — install under a different server name (default `rumb`).
- `--command <path>` — override the recorded command path.
- `--target <file>` — write to a different config file (default `.mcp.json`).
- `--force` — replace an existing entry with the same name.

## How `rumb mcp serve` finds the server

`rumb mcp serve` is a launcher: it locates the `rumb-mcp` binary and execs it.
Resolution order:

1. `$RUMB_MCP_SHIM`, if set and non-empty.
2. Under `$RUMB_HOME` (if set): `target/release/rumb-mcp`, then
   `target/debug/rumb-mcp`, then `bin/rumb-mcp`.
3. A `rumb-mcp` sibling next to the running `rumb` executable.
4. `rumb-mcp` on `PATH`.

The dev and release shims set `RUMB_HOME` and `RUMB_MCP_SHIM` for you; see
[Architecture](architecture.md#environment-variables).

## Smoke testing manually

You can drive the server over stdio with raw JSON-RPC. Start it in a repo, send
`initialize`, then call tools via `tools/call`. The integration test in
[`tests/mcp_smoke.rs`](../tests/mcp_smoke.rs) is a complete worked example of the
initialize → `init` → `item_create` → `ready`/`claim`/`run`/`release` → `log`
flow.
