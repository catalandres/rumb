use std::collections::BTreeMap;

use duckdb::params;

use super::model::*;
use super::store::*;
use super::RumbProject;

/// An item is spiraling once it has this many trailing consecutive failed runs
/// with no status movement in between.
const SPIRAL_MIN_FAILED: usize = 3;
/// An inbox child is stale when its newest event is older than this.
const STALE_SECS: u64 = 7 * 24 * 60 * 60;
/// Momentum counts items moved to `done` within this window.
const MOMENTUM_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;

impl RumbProject {
    /// A deterministic, computed read of where the project is going: items
    /// spiraling on failed runs, recurring title threads, stale inbox captures,
    /// and recent done-momentum by kind. No LLM, no heuristics beyond fixed
    /// thresholds.
    pub fn digest(&self) -> Result<Digest, RumbError> {
        with_storage_retry(|| {
            let conn = self.open_database()?;
            let now = timestamp();
            let items = load_items(&conn)?;
            let reserved = reserved_node_ids(&conn)?;

            Ok(Digest {
                spirals: spirals(&conn, &items, &reserved)?,
                threads: threads(&items, &reserved),
                stale_inbox: stale_inbox(&conn, &items, now)?,
                momentum: momentum(&conn, now)?,
            })
        })
    }
}

fn spirals(
    conn: &duckdb::Connection,
    items: &[Item],
    reserved: &std::collections::HashSet<String>,
) -> Result<Vec<Spiral>, RumbError> {
    let mut spirals = Vec::new();
    for item in items.iter().filter(|item| !reserved.contains(&item.id)) {
        let since = last_status_move(conn, &item.id)?;
        let runs = load_runs_for_item(conn, &item.id)?;
        let trailing = runs
            .iter()
            .filter(|run| since.is_none_or(|ts| run.started_at > ts))
            .rev()
            .take_while(|run| run.status == RunStatus::Failed)
            .count();
        if trailing >= SPIRAL_MIN_FAILED {
            spirals.push(Spiral {
                item: item.clone(),
                failed_runs: trailing,
            });
        }
    }
    Ok(spirals)
}

fn threads(items: &[Item], reserved: &std::collections::HashSet<String>) -> Vec<Thread> {
    let mut by_title: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in items.iter().filter(|item| !reserved.contains(&item.id)) {
        by_title
            .entry(normalize_title(&item.title))
            .or_default()
            .push(item.id.clone());
    }
    by_title
        .into_iter()
        .filter(|(_, ids)| ids.len() >= 2)
        .map(|(title, item_ids)| Thread { title, item_ids })
        .collect()
}

fn stale_inbox(
    conn: &duckdb::Connection,
    items: &[Item],
    now: u64,
) -> Result<Vec<Item>, RumbError> {
    let Some(inbox) = inbox_id(conn)? else {
        return Ok(Vec::new());
    };
    let mut stale = Vec::new();
    for child in items
        .iter()
        .filter(|item| item.parent_id.as_deref() == Some(inbox.as_str()))
    {
        if last_event(conn, &child.id)?.is_none_or(|ts| now.saturating_sub(ts) > STALE_SECS) {
            stale.push(child.clone());
        }
    }
    Ok(stale)
}

fn momentum(conn: &duckdb::Connection, now: u64) -> Result<Vec<Momentum>, RumbError> {
    let cutoff = now.saturating_sub(MOMENTUM_WINDOW_SECS);
    let mut stmt = conn.prepare(
        "SELECT object_id, payload_json FROM events \
         WHERE action = 'item.done' AND timestamp >= ? ORDER BY seq",
    )?;
    let rows = stmt.query_map(params![cutoff as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for row in rows {
        let (object_id, payload) = row?;
        // Prefer the kind recorded in the done event (kind-at-time); fall back to
        // the item's current kind for events written before that was recorded.
        let kind = serde_json::from_str::<serde_json::Value>(&payload)
            .ok()
            .and_then(|value| {
                value
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            });
        let kind = match kind {
            Some(kind) => kind,
            None => match load_item(conn, &object_id)? {
                Some(item) => item.kind,
                None => continue,
            },
        };
        *counts.entry(kind).or_default() += 1;
    }
    Ok(counts
        .into_iter()
        .map(|(kind, count)| Momentum { kind, count })
        .collect())
}

fn last_status_move(conn: &duckdb::Connection, item_id: &str) -> Result<Option<u64>, RumbError> {
    max_event_timestamp(
        conn,
        "SELECT MAX(timestamp) FROM events \
         WHERE object_id = ? AND action IN ('item.status', 'item.review', 'item.done')",
        item_id,
    )
}

fn last_event(conn: &duckdb::Connection, item_id: &str) -> Result<Option<u64>, RumbError> {
    max_event_timestamp(
        conn,
        "SELECT MAX(timestamp) FROM events WHERE object_id = ?",
        item_id,
    )
}

fn max_event_timestamp(
    conn: &duckdb::Connection,
    sql: &str,
    item_id: &str,
) -> Result<Option<u64>, RumbError> {
    let ts: Option<i64> = conn.query_row(sql, params![item_id], |row| row.get(0))?;
    Ok(ts.map(|ts| ts as u64))
}

fn normalize_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
