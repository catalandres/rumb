use std::collections::HashSet;

use serde_json::json;

use super::model::*;
use super::store::*;
use super::RumbProject;

impl RumbProject {
    /// Reverse the most recent undoable changeset (a grooming or capture move).
    /// Refuses when a later changeset depends on the same items, their
    /// descendants, or the touched edge endpoints — undo is not a force.
    pub fn undo(&self) -> Result<UndoOutcome, RumbError> {
        self.mutate(|m| {
            let changeset = latest_undoable_changeset(m.conn())?.ok_or(RumbError::NothingToUndo)?;
            let deltas = load_changeset_deltas(m.conn(), changeset.seq)?;

            // Collect what this changeset touched: every item id / edge endpoint in
            // its deltas, its `object_id`, and the descendants of any touched item.
            let mut touched = HashSet::new();
            for delta in &deltas {
                collect_pk_ids(&delta.pk_json, &mut touched)?;
            }
            touched.insert(changeset.object_id.clone());
            add_descendants(&mut touched, &load_items(m.conn())?);

            let later = referenced_ids_after(m.conn(), changeset.seq)?;
            if let Some(id) = touched.iter().find(|id| later.contains(*id)) {
                return Err(RumbError::UndoBlocked(format!(
                    "a later change depends on {id}"
                )));
            }

            // Apply each delta's inverse, newest first.
            for delta in deltas.iter().rev() {
                match delta.table_name.as_str() {
                    "items" => invert_item(m, delta)?,
                    "edges" => invert_edge(m, delta)?,
                    other => {
                        return Err(RumbError::InvalidState(format!(
                            "unknown delta table: {other}"
                        )))
                    }
                }
            }

            m.event(
                "undo",
                "changeset",
                &changeset.object_id,
                json!({
                    "undone_seq": changeset.seq,
                    "undone_verb": &changeset.verb,
                })
                .to_string(),
                timestamp(),
            );
            // Consume the original so a repeat `undo` targets the previous move.
            set_changeset_not_undoable(m.conn(), changeset.seq)?;

            Ok(UndoOutcome {
                seq: changeset.seq,
                verb: changeset.verb,
                object_id: changeset.object_id,
            })
        })
    }
}

fn invert_item(m: &mut super::history::Mutation, delta: &DeltaRecord) -> Result<(), RumbError> {
    match (&delta.before_json, &delta.after_json) {
        // was an insert -> delete it
        (None, Some(after)) => {
            let item = item_from_row_json(after)?;
            if is_reserved_node(m.conn(), &item.id)? {
                return Err(RumbError::UndoBlocked(format!(
                    "would delete reserved node {}",
                    item.id
                )));
            }
            m.delete_item(&item)
        }
        // was a delete -> re-insert it
        (Some(before), None) => m.insert_item(&item_from_row_json(before)?),
        // was an update -> restore the before image
        (Some(before), Some(_)) => m.update_item(&item_from_row_json(before)?),
        (None, None) => Err(RumbError::InvalidState(
            "item delta with no before or after image".to_owned(),
        )),
    }
}

fn invert_edge(m: &mut super::history::Mutation, delta: &DeltaRecord) -> Result<(), RumbError> {
    match (&delta.before_json, &delta.after_json) {
        (None, Some(after)) => m.delete_edge(&edge_from_row_json(after)?),
        (Some(before), None) => m.insert_edge(&edge_from_row_json(before)?),
        (Some(_), Some(_)) | (None, None) => Err(RumbError::InvalidState(
            "unexpected edge delta shape".to_owned(),
        )),
    }
}

fn collect_pk_ids(pk_json: &str, ids: &mut HashSet<String>) -> Result<(), RumbError> {
    let pk: serde_json::Value =
        serde_json::from_str(pk_json).map_err(|err| RumbError::InvalidState(err.to_string()))?;
    for key in ["id", "from", "to"] {
        if let Some(value) = pk.get(key).and_then(serde_json::Value::as_str) {
            ids.insert(value.to_owned());
        }
    }
    Ok(())
}

/// Expand `touched` with the descendants (by parent chain) of any item already in it.
fn add_descendants(touched: &mut HashSet<String>, items: &[Item]) {
    loop {
        let mut added = false;
        for item in items {
            if let Some(parent) = &item.parent_id {
                if touched.contains(parent) && touched.insert(item.id.clone()) {
                    added = true;
                }
            }
        }
        if !added {
            break;
        }
    }
}
