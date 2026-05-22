use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use duckdb::{params, Connection};

use super::model::*;

#[derive(Debug)]
pub(crate) struct DbItem {
    id: String,
    parent_id: Option<String>,
    kind: String,
    title: String,
    status: String,
    source_ref: Option<String>,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug)]
pub(crate) struct DbEdge {
    from: String,
    to: String,
    kind: String,
    created_at: i64,
}

#[derive(Debug)]
pub(crate) struct DbClaim {
    id: String,
    item_id: String,
    actor_id: String,
    lease_until: i64,
    status: String,
    branch: String,
    worktree_path: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug)]
pub(crate) struct DbProposal {
    id: String,
    item_id: String,
    branch: String,
    base_ref: String,
    head_ref: Option<String>,
    status: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug)]
pub(crate) struct DbRun {
    id: String,
    item_id: String,
    proposal_id: Option<String>,
    command: String,
    status: String,
    output_path: String,
    started_at: i64,
    finished_at: Option<i64>,
}
pub(crate) fn ensure_schema(conn: &mut Connection) -> Result<(), RumbError> {
    let tx = conn.transaction()?;
    tx.execute_batch(
        r"
        CREATE TABLE IF NOT EXISTS migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at BIGINT NOT NULL
        );
        ",
    )?;

    let applied = applied_migrations(&tx)?;
    if !applied.contains(&1) {
        tx.execute_batch(
            r"
            CREATE TABLE IF NOT EXISTS items (
                id TEXT PRIMARY KEY,
                parent_id TEXT,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                status TEXT NOT NULL,
                source_ref TEXT,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS edges (
                from_item TEXT NOT NULL,
                to_item TEXT NOT NULL,
                kind TEXT NOT NULL,
                created_at BIGINT NOT NULL,
                PRIMARY KEY (from_item, to_item, kind)
            );

            CREATE TABLE IF NOT EXISTS events (
                seq BIGINT PRIMARY KEY,
                timestamp BIGINT NOT NULL,
                action TEXT NOT NULL,
                object_type TEXT NOT NULL,
                object_id TEXT NOT NULL,
                payload_json TEXT NOT NULL
            );
            ",
        )?;
        tx.execute(
            "INSERT INTO migrations (version, name, applied_at) VALUES (?, ?, ?)",
            params![1, "milestone_1_state", timestamp() as i64],
        )?;
    }

    if !applied.contains(&2) {
        tx.execute_batch(
            r"
            CREATE TABLE IF NOT EXISTS claims (
                id TEXT PRIMARY KEY,
                item_id TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                lease_until BIGINT NOT NULL,
                status TEXT NOT NULL,
                branch TEXT NOT NULL,
                worktree_path TEXT NOT NULL,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS proposals (
                id TEXT PRIMARY KEY,
                item_id TEXT NOT NULL,
                branch TEXT NOT NULL,
                base_ref TEXT NOT NULL,
                head_ref TEXT,
                status TEXT NOT NULL,
                created_at BIGINT NOT NULL,
                updated_at BIGINT NOT NULL
            );
            ",
        )?;
        tx.execute(
            "INSERT INTO migrations (version, name, applied_at) VALUES (?, ?, ?)",
            params![2, "claim_worktree_state", timestamp() as i64],
        )?;
    }

    if !applied.contains(&3) {
        tx.execute_batch(
            r"
            CREATE TABLE IF NOT EXISTS runs (
                id TEXT PRIMARY KEY,
                item_id TEXT NOT NULL,
                proposal_id TEXT,
                command TEXT NOT NULL,
                status TEXT NOT NULL,
                output_path TEXT NOT NULL,
                started_at BIGINT NOT NULL,
                finished_at BIGINT
            );
            ",
        )?;
        tx.execute(
            "INSERT INTO migrations (version, name, applied_at) VALUES (?, ?, ?)",
            params![3, "run_lifecycle_state", timestamp() as i64],
        )?;
    }
    tx.commit()?;

    Ok(())
}

pub(crate) fn applied_migrations(conn: &Connection) -> Result<HashSet<i32>, RumbError> {
    let mut versions = HashSet::new();
    let mut stmt = conn.prepare("SELECT version FROM migrations")?;
    let rows = stmt.query_map([], |row| row.get::<_, i32>(0))?;
    for row in rows {
        versions.insert(row?);
    }
    Ok(versions)
}

pub(crate) fn item_exists(conn: &Connection, id: &str) -> Result<bool, RumbError> {
    let count = conn.query_row(
        "SELECT COUNT(*) FROM items WHERE id = ?",
        params![id],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(count > 0)
}

pub(crate) fn insert_item(conn: &Connection, item: &Item) -> Result<(), RumbError> {
    let status = item.status.to_string();
    conn.execute(
        r"
        INSERT INTO items (
            id, parent_id, kind, title, status, source_ref, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        ",
        params![
            &item.id,
            item.parent_id.as_deref(),
            &item.kind,
            &item.title,
            &status,
            item.source_ref.as_deref(),
            item.created_at as i64,
            item.updated_at as i64,
        ],
    )?;
    Ok(())
}

pub(crate) fn insert_edge(conn: &Connection, edge: &Edge) -> Result<(), RumbError> {
    let kind = edge.kind.to_string();
    conn.execute(
        r"
        INSERT INTO edges (from_item, to_item, kind, created_at)
        VALUES (?, ?, ?, ?)
        ",
        params![&edge.from, &edge.to, &kind, edge.created_at as i64],
    )?;
    Ok(())
}

pub(crate) fn load_item(conn: &Connection, id: &str) -> Result<Option<Item>, RumbError> {
    let mut stmt = conn.prepare(
        r"
        SELECT id, parent_id, kind, title, status, source_ref, created_at, updated_at
        FROM items
        WHERE id = ?
        ",
    )?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(DbItem {
            id: row.get(0)?,
            parent_id: row.get(1)?,
            kind: row.get(2)?,
            title: row.get(3)?,
            status: row.get(4)?,
            source_ref: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    })?;
    rows.next().transpose()?.map(item_from_db).transpose()
}

pub(crate) fn item_status(conn: &Connection, id: &str) -> Result<Option<Status>, RumbError> {
    Ok(load_item(conn, id)?.map(|item| item.status))
}

pub(crate) fn update_item_status_row(
    conn: &Connection,
    item_id: &str,
    status: Status,
    updated_at: u64,
) -> Result<(), RumbError> {
    let changed = conn.execute(
        "UPDATE items SET status = ?, updated_at = ? WHERE id = ?",
        params![status.to_string(), updated_at as i64, item_id],
    )?;
    if changed == 0 {
        return Err(RumbError::MissingItem(item_id.to_owned()));
    }
    Ok(())
}

pub(crate) fn insert_claim(conn: &Connection, claim: &Claim) -> Result<(), RumbError> {
    conn.execute(
        r"
        INSERT INTO claims (
            id, item_id, actor_id, lease_until, status, branch, worktree_path, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ",
        params![
            &claim.id,
            &claim.item_id,
            &claim.actor_id,
            claim.lease_until as i64,
            claim.status.to_string(),
            &claim.branch,
            &claim.worktree_path,
            claim.created_at as i64,
            claim.updated_at as i64,
        ],
    )?;
    Ok(())
}

pub(crate) fn insert_proposal(
    conn: &Connection,
    proposal_id: &str,
    claim: &Claim,
    status: &str,
    now: u64,
    base_ref: &str,
) -> Result<(), RumbError> {
    conn.execute(
        r"
        INSERT INTO proposals (
            id, item_id, branch, base_ref, head_ref, status, created_at, updated_at
        ) VALUES (?, ?, ?, ?, NULL, ?, ?, ?)
        ",
        params![
            proposal_id,
            &claim.item_id,
            &claim.branch,
            base_ref,
            status,
            now as i64,
            now as i64,
        ],
    )?;
    Ok(())
}

pub(crate) fn load_claim(conn: &Connection, id: &str) -> Result<Option<Claim>, RumbError> {
    let mut stmt = conn.prepare(
        r"
        SELECT id, item_id, actor_id, lease_until, status, branch, worktree_path, created_at, updated_at
        FROM claims
        WHERE id = ?
        ",
    )?;
    let mut rows = stmt.query_map(params![id], map_claim_row)?;
    rows.next().transpose()?.map(claim_from_db).transpose()
}

pub(crate) fn load_claims_for_item(
    conn: &Connection,
    item_id: &str,
) -> Result<Vec<Claim>, RumbError> {
    let mut claims = Vec::new();
    let mut stmt = conn.prepare(
        r"
        SELECT id, item_id, actor_id, lease_until, status, branch, worktree_path, created_at, updated_at
        FROM claims
        WHERE item_id = ?
        ORDER BY created_at, id
        ",
    )?;
    let rows = stmt.query_map(params![item_id], map_claim_row)?;
    for row in rows {
        claims.push(claim_from_db(row?)?);
    }
    Ok(claims)
}

pub(crate) fn update_claim_status(conn: &Connection, claim: &Claim) -> Result<(), RumbError> {
    conn.execute(
        "UPDATE claims SET status = ?, updated_at = ? WHERE id = ?",
        params![claim.status.to_string(), claim.updated_at as i64, &claim.id],
    )?;
    Ok(())
}

pub(crate) fn update_claim_lease(conn: &Connection, claim: &Claim) -> Result<(), RumbError> {
    conn.execute(
        "UPDATE claims SET lease_until = ?, updated_at = ? WHERE id = ?",
        params![claim.lease_until as i64, claim.updated_at as i64, &claim.id],
    )?;
    Ok(())
}

pub(crate) fn update_proposal_status(
    conn: &Connection,
    proposal_id: &str,
    status: &str,
    updated_at: u64,
) -> Result<(), RumbError> {
    conn.execute(
        "UPDATE proposals SET status = ?, updated_at = ? WHERE id = ?",
        params![status, updated_at as i64, proposal_id],
    )?;
    Ok(())
}

pub(crate) fn update_proposal_status_for_claim(
    conn: &Connection,
    claim: &Claim,
    status: &str,
    updated_at: u64,
) -> Result<(), RumbError> {
    conn.execute(
        "UPDATE proposals SET status = ?, updated_at = ? WHERE item_id = ? AND branch = ?",
        params![status, updated_at as i64, &claim.item_id, &claim.branch],
    )?;
    Ok(())
}

pub(crate) fn latest_proposal_id_for_item(
    conn: &Connection,
    item_id: &str,
) -> Result<Option<String>, RumbError> {
    let mut stmt = conn.prepare(
        r"
        SELECT id
        FROM proposals
        WHERE item_id = ?
        ORDER BY updated_at DESC, created_at DESC, id DESC
        LIMIT 1
        ",
    )?;
    let mut rows = stmt.query_map(params![item_id], |row| row.get::<_, String>(0))?;
    rows.next().transpose().map_err(RumbError::Storage)
}

pub(crate) fn load_proposals_for_item(
    conn: &Connection,
    item_id: &str,
) -> Result<Vec<Proposal>, RumbError> {
    let mut proposals = Vec::new();
    let mut stmt = conn.prepare(
        r"
        SELECT id, item_id, branch, base_ref, head_ref, status, created_at, updated_at
        FROM proposals
        WHERE item_id = ?
        ORDER BY created_at, id
        ",
    )?;
    let rows = stmt.query_map(params![item_id], |row| {
        Ok(DbProposal {
            id: row.get(0)?,
            item_id: row.get(1)?,
            branch: row.get(2)?,
            base_ref: row.get(3)?,
            head_ref: row.get(4)?,
            status: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    })?;
    for row in rows {
        proposals.push(proposal_from_db(row?)?);
    }
    Ok(proposals)
}

pub(crate) fn load_runs_for_item(
    conn: &Connection,
    item_id: &str,
) -> Result<Vec<RunRecord>, RumbError> {
    let mut runs = Vec::new();
    let mut stmt = conn.prepare(
        r"
        SELECT id, item_id, proposal_id, command, status, output_path, started_at, finished_at
        FROM runs
        WHERE item_id = ?
        ORDER BY started_at, id
        ",
    )?;
    let rows = stmt.query_map(params![item_id], |row| {
        Ok(DbRun {
            id: row.get(0)?,
            item_id: row.get(1)?,
            proposal_id: row.get(2)?,
            command: row.get(3)?,
            status: row.get(4)?,
            output_path: row.get(5)?,
            started_at: row.get(6)?,
            finished_at: row.get(7)?,
        })
    })?;
    for row in rows {
        runs.push(run_from_db(row?)?);
    }
    Ok(runs)
}

pub(crate) fn load_events_for_item(
    conn: &Connection,
    item_id: &str,
) -> Result<Vec<Event>, RumbError> {
    let mut events = Vec::new();
    let mut stmt = conn.prepare(
        r"
        SELECT timestamp, action, object_type, object_id, payload_json
        FROM events
        WHERE object_id = ?
        ORDER BY seq
        ",
    )?;
    let rows = stmt.query_map(params![item_id], map_event_row)?;
    for row in rows {
        events.push(row?);
    }
    Ok(events)
}

pub(crate) fn active_claim_for_item(
    conn: &Connection,
    item_id: &str,
    now: u64,
) -> Result<Option<Claim>, RumbError> {
    let mut stmt = conn.prepare(
        r"
        SELECT id, item_id, actor_id, lease_until, status, branch, worktree_path, created_at, updated_at
        FROM claims
        WHERE item_id = ?
          AND status IN ('pending', 'active')
          AND lease_until > ?
        ORDER BY created_at
        LIMIT 1
        ",
    )?;
    let mut rows = stmt.query_map(params![item_id, now as i64], map_claim_row)?;
    rows.next().transpose()?.map(claim_from_db).transpose()
}

pub(crate) fn has_other_active_claim(
    conn: &Connection,
    claim_id: &str,
    item_id: &str,
    now: u64,
) -> Result<bool, RumbError> {
    let count = conn.query_row(
        r"
        SELECT COUNT(*)
        FROM claims
        WHERE item_id = ?
          AND id <> ?
          AND status IN ('pending', 'active')
          AND lease_until > ?
        ",
        params![item_id, claim_id, now as i64],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(count > 0)
}

pub(crate) fn active_claim_item_ids(
    conn: &Connection,
    now: u64,
) -> Result<HashSet<String>, RumbError> {
    let mut ids = HashSet::new();
    let mut stmt = conn.prepare(
        r"
        SELECT DISTINCT item_id
        FROM claims
        WHERE status IN ('pending', 'active')
          AND lease_until > ?
        ",
    )?;
    let rows = stmt.query_map(params![now as i64], |row| row.get::<_, String>(0))?;
    for row in rows {
        ids.insert(row?);
    }
    Ok(ids)
}

pub(crate) fn next_prefixed_id(
    conn: &Connection,
    table: &str,
    prefix: &str,
    width: usize,
) -> Result<String, RumbError> {
    if !matches!(table, "claims" | "proposals" | "runs") {
        return Err(RumbError::InvalidState(format!(
            "invalid id allocation table: {table}"
        )));
    }

    let mut max_id = 0;
    let sql = format!("SELECT id FROM {table} WHERE id LIKE ?");
    let like = format!("{prefix}%");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![like], |row| row.get::<_, String>(0))?;
    for row in rows {
        if let Some(value) = row?
            .strip_prefix(prefix)
            .and_then(|suffix| suffix.parse::<u32>().ok())
        {
            max_id = max_id.max(value);
        }
    }
    let next = max_id + 1;
    Ok(format!("{prefix}{next:0width$}"))
}

pub(crate) struct EventInput<'a> {
    pub(crate) timestamp: u64,
    pub(crate) action: &'a str,
    pub(crate) object_type: &'a str,
    pub(crate) object_id: &'a str,
    pub(crate) payload: String,
}

pub(crate) fn append_event(conn: &Connection, event: EventInput<'_>) -> Result<(), RumbError> {
    let seq = conn.query_row("SELECT COALESCE(MAX(seq), 0) + 1 FROM events", [], |row| {
        row.get::<_, i64>(0)
    })?;
    conn.execute(
        r"
        INSERT INTO events (seq, timestamp, action, object_type, object_id, payload_json)
        VALUES (?, ?, ?, ?, ?, ?)
        ",
        params![
            seq,
            event.timestamp as i64,
            event.action,
            event.object_type,
            event.object_id,
            event.payload,
        ],
    )?;
    Ok(())
}

pub(crate) fn next_item_id(conn: &Connection) -> Result<String, RumbError> {
    let mut max_id = 0;
    let mut stmt = conn.prepare("SELECT id FROM items WHERE id LIKE 'RUMB-%'")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    for row in rows {
        if let Some(value) = row?
            .strip_prefix("RUMB-")
            .and_then(|suffix| suffix.parse::<u32>().ok())
        {
            max_id = max_id.max(value);
        }
    }
    let next = max_id + 1;
    Ok(format!("RUMB-{next:04}"))
}

pub(crate) fn load_items(conn: &Connection) -> Result<Vec<Item>, RumbError> {
    let mut items = Vec::new();
    let mut stmt = conn.prepare(
        r"
        SELECT id, parent_id, kind, title, status, source_ref, created_at, updated_at
        FROM items
        ORDER BY id
        ",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(DbItem {
            id: row.get(0)?,
            parent_id: row.get(1)?,
            kind: row.get(2)?,
            title: row.get(3)?,
            status: row.get(4)?,
            source_ref: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    })?;

    for row in rows {
        items.push(item_from_db(row?)?);
    }
    Ok(items)
}

pub(crate) fn load_edges(conn: &Connection) -> Result<Vec<Edge>, RumbError> {
    let mut edges = Vec::new();
    let mut stmt = conn.prepare(
        r"
        SELECT from_item, to_item, kind, created_at
        FROM edges
        ORDER BY created_at, from_item, to_item, kind
        ",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(DbEdge {
            from: row.get(0)?,
            to: row.get(1)?,
            kind: row.get(2)?,
            created_at: row.get(3)?,
        })
    })?;

    for row in rows {
        edges.push(edge_from_db(row?)?);
    }
    Ok(edges)
}

pub(crate) fn item_from_db(item: DbItem) -> Result<Item, RumbError> {
    Ok(Item {
        id: item.id,
        parent_id: item.parent_id,
        kind: item.kind,
        title: item.title,
        status: item.status.parse()?,
        source_ref: item.source_ref,
        created_at: stored_timestamp(item.created_at)?,
        updated_at: stored_timestamp(item.updated_at)?,
    })
}

pub(crate) fn edge_from_db(edge: DbEdge) -> Result<Edge, RumbError> {
    Ok(Edge {
        from: edge.from,
        to: edge.to,
        kind: edge.kind.parse()?,
        created_at: stored_timestamp(edge.created_at)?,
    })
}

pub(crate) fn map_claim_row(row: &duckdb::Row<'_>) -> duckdb::Result<DbClaim> {
    Ok(DbClaim {
        id: row.get(0)?,
        item_id: row.get(1)?,
        actor_id: row.get(2)?,
        lease_until: row.get(3)?,
        status: row.get(4)?,
        branch: row.get(5)?,
        worktree_path: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

pub(crate) fn claim_from_db(claim: DbClaim) -> Result<Claim, RumbError> {
    Ok(Claim {
        id: claim.id,
        item_id: claim.item_id,
        actor_id: claim.actor_id,
        lease_until: stored_timestamp(claim.lease_until)?,
        status: claim.status.parse()?,
        branch: claim.branch,
        worktree_path: claim.worktree_path,
        created_at: stored_timestamp(claim.created_at)?,
        updated_at: stored_timestamp(claim.updated_at)?,
    })
}

pub(crate) fn proposal_from_db(proposal: DbProposal) -> Result<Proposal, RumbError> {
    Ok(Proposal {
        id: proposal.id,
        item_id: proposal.item_id,
        branch: proposal.branch,
        base_ref: proposal.base_ref,
        head_ref: proposal.head_ref,
        status: proposal.status,
        created_at: stored_timestamp(proposal.created_at)?,
        updated_at: stored_timestamp(proposal.updated_at)?,
    })
}

pub(crate) fn run_from_db(run: DbRun) -> Result<RunRecord, RumbError> {
    Ok(RunRecord {
        id: run.id,
        item_id: run.item_id,
        proposal_id: run.proposal_id,
        command: run.command,
        status: run.status.parse()?,
        output_path: run.output_path,
        started_at: stored_timestamp(run.started_at)?,
        finished_at: run
            .finished_at
            .map(stored_timestamp)
            .transpose()?
            .unwrap_or(0),
    })
}

pub(crate) fn map_event_row(row: &duckdb::Row<'_>) -> duckdb::Result<Event> {
    let timestamp: i64 = row.get(0)?;
    Ok(Event {
        timestamp: timestamp as u64,
        action: row.get(1)?,
        object_type: row.get(2)?,
        object_id: row.get(3)?,
        payload: row.get(4)?,
    })
}

pub(crate) fn stored_timestamp(value: i64) -> Result<u64, RumbError> {
    value
        .try_into()
        .map_err(|_| RumbError::InvalidState(format!("negative timestamp: {value}")))
}

pub(crate) fn normalize_item_id(reference: &str) -> Result<String, RumbError> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return Err(RumbError::InvalidItemRef(reference.to_owned()));
    }

    let number = trimmed
        .strip_prefix("RUMB-")
        .or_else(|| trimmed.strip_prefix("rumb-"))
        .unwrap_or(trimmed);
    if number.is_empty() || !number.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(RumbError::InvalidItemRef(reference.to_owned()));
    }
    let value = number
        .parse::<u32>()
        .map_err(|_| RumbError::InvalidItemRef(reference.to_owned()))?;
    Ok(format!("RUMB-{value:04}"))
}

pub(crate) fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }
        if slug.len() >= 48 {
            break;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "item".to_owned()
    } else {
        slug
    }
}

pub(crate) fn write_run_log(
    path: &Path,
    command: &str,
    status: RunStatus,
    exit_code: Option<i32>,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<(), RumbError> {
    let mut file = fs::File::create(path)?;
    writeln!(file, "command\t{command}")?;
    writeln!(file, "status\t{status}")?;
    match exit_code {
        Some(code) => writeln!(file, "exit_code\t{code}")?,
        None => writeln!(file, "exit_code\tunknown")?,
    }
    writeln!(file, "\n[stdout]")?;
    file.write_all(stdout)?;
    if !stdout.ends_with(b"\n") {
        writeln!(file)?;
    }
    writeln!(file, "\n[stderr]")?;
    file.write_all(stderr)?;
    if !stderr.ends_with(b"\n") {
        writeln!(file)?;
    }
    Ok(())
}

pub fn parse_ttl(value: &str) -> Result<u64, RumbError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(RumbError::InvalidTtl(value.to_owned()));
    }

    let (number, multiplier) = match trimmed.chars().last() {
        Some('s') => (&trimmed[..trimmed.len() - 1], 1),
        Some('m') => (&trimmed[..trimmed.len() - 1], 60),
        Some('h') => (&trimmed[..trimmed.len() - 1], 60 * 60),
        Some('d') => (&trimmed[..trimmed.len() - 1], 24 * 60 * 60),
        Some(ch) if ch.is_ascii_digit() => (trimmed, 1),
        _ => return Err(RumbError::InvalidTtl(value.to_owned())),
    };

    let amount = number
        .parse::<u64>()
        .map_err(|_| RumbError::InvalidTtl(value.to_owned()))?;
    if amount == 0 {
        return Err(RumbError::InvalidTtl(value.to_owned()));
    }
    amount
        .checked_mul(multiplier)
        .ok_or_else(|| RumbError::InvalidTtl(value.to_owned()))
}

pub fn default_ttl_seconds() -> u64 {
    DEFAULT_TTL_SECONDS
}

pub(crate) fn with_storage_retry<T>(
    mut operation: impl FnMut() -> Result<T, RumbError>,
) -> Result<T, RumbError> {
    let mut delay = Duration::from_millis(25);
    for attempt in 0..STORAGE_RETRY_ATTEMPTS {
        match operation() {
            Err(err) if attempt + 1 < STORAGE_RETRY_ATTEMPTS && is_busy_error(&err) => {
                thread::sleep(delay);
                delay = delay.saturating_mul(2);
            }
            result => return result,
        }
    }
    operation()
}

pub(crate) fn with_write_retry<T>(
    operation: impl FnMut() -> Result<T, RumbError>,
) -> Result<T, RumbError> {
    with_storage_retry(operation)
}

pub(crate) fn is_busy_error(err: &RumbError) -> bool {
    match err {
        RumbError::Storage(storage) => {
            let message = storage.to_string().to_ascii_lowercase();
            message.contains("busy")
                || message.contains("locked")
                || message.contains("conflict")
                || message.contains("transaction")
        }
        _ => false,
    }
}

pub(crate) fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
