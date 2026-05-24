use std::collections::BTreeMap;

use super::model::*;
use super::store::*;
use super::RumbProject;

type EdgeKey = (String, String, String);

impl RumbProject {
    /// Reconstruct the item graph as it stood at changeset `seq` — read-only, no
    /// writes. Starts from the genesis snapshot and replays deltas forward to
    /// `seq`, then layers in reserved infrastructure (the inbox is seeded outside
    /// the changeset timeline, so it is injected rather than replayed).
    pub fn at(&self, seq: i64) -> Result<GraphAt, RumbError> {
        with_storage_retry(|| {
            let conn = self.open_database()?;
            let (genesis_seq, payload) = genesis_snapshot_row(&conn)?;
            if seq < genesis_seq || seq > max_changeset_seq(&conn)? {
                return Err(RumbError::SeqOutOfRange(seq));
            }

            let snapshot: serde_json::Value = serde_json::from_str(&payload)
                .map_err(|err| RumbError::InvalidState(err.to_string()))?;
            let mut items: BTreeMap<String, Item> = BTreeMap::new();
            let mut edges: BTreeMap<EdgeKey, Edge> = BTreeMap::new();
            for value in snapshot
                .get("items")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                let item = item_from_row_json(&value.to_string())?;
                items.insert(item.id.clone(), item);
            }
            for value in snapshot
                .get("edges")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                let edge = edge_from_row_json(&value.to_string())?;
                edges.insert(edge_key(&edge), edge);
            }

            for delta in load_deltas_through(&conn, genesis_seq, seq)? {
                apply_delta(&delta, &mut items, &mut edges)?;
            }

            // The inbox is raw-inserted (no delta), so replay never produces it.
            // Inject the current inbox node as always-present infrastructure.
            if let Some(id) = inbox_id(&conn)? {
                if let std::collections::btree_map::Entry::Vacant(entry) = items.entry(id) {
                    if let Some(inbox) = load_item(&conn, entry.key())? {
                        entry.insert(inbox);
                    }
                }
            }

            Ok(GraphAt {
                seq,
                items: items.into_values().collect(),
                edges: edges.into_values().collect(),
            })
        })
    }
}

fn apply_delta(
    delta: &DeltaRecord,
    items: &mut BTreeMap<String, Item>,
    edges: &mut BTreeMap<EdgeKey, Edge>,
) -> Result<(), RumbError> {
    match delta.table_name.as_str() {
        "items" => match (&delta.before_json, &delta.after_json) {
            (_, Some(after)) => {
                let item = item_from_row_json(after)?;
                items.insert(item.id.clone(), item);
            }
            (Some(before), None) => {
                items.remove(&item_from_row_json(before)?.id);
            }
            (None, None) => return Err(empty_delta()),
        },
        "edges" => match (&delta.before_json, &delta.after_json) {
            (_, Some(after)) => {
                let edge = edge_from_row_json(after)?;
                edges.insert(edge_key(&edge), edge);
            }
            (Some(before), None) => {
                edges.remove(&edge_key(&edge_from_row_json(before)?));
            }
            (None, None) => return Err(empty_delta()),
        },
        other => {
            return Err(RumbError::InvalidState(format!(
                "unknown delta table: {other}"
            )))
        }
    }
    Ok(())
}

fn edge_key(edge: &Edge) -> EdgeKey {
    (edge.from.clone(), edge.to.clone(), edge.kind.to_string())
}

fn empty_delta() -> RumbError {
    RumbError::InvalidState("delta with no before or after image".to_owned())
}
