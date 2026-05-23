use duckdb::{params, Connection, Transaction};
use serde_json::json;

use super::model::{Edge, Item, RumbError, Status};
use super::store::{
    delete_edge_row, delete_item_row, edge_row_value, insert_edge, insert_item, item_row_value,
    load_item, update_item_row, update_item_status_row, with_write_retry,
};
use super::RumbProject;

/// One captured row-level change within a changeset (before/after row images).
/// `before_json` is `None` for an insert; `after_json` is `None` for a delete.
struct DeltaRow {
    table_name: &'static str,
    pk_json: String,
    before_json: Option<String>,
    after_json: Option<String>,
}

/// Composite primary key of an edge as JSON, shared by insert/delete deltas so
/// the two never drift.
fn edge_pk_json(edge: &Edge) -> String {
    json!({
        "from": edge.from,
        "to": edge.to,
        "kind": edge.kind.to_string(),
    })
    .to_string()
}

/// The semantic header for one changeset. `verb`/`intent_json` reproduce the
/// pre-changeset event log's `action`/`payload_json` so the `events` view is
/// byte-identical. `actor` stays inside `intent_json` for now (the dedicated
/// column is populated NULL until a later promotion).
struct ChangesetHeader {
    ts: u64,
    verb: String,
    object_type: String,
    object_id: String,
    intent_json: String,
    /// Grooming changesets set this so a later `undo` can reverse them; lifecycle
    /// changesets (claims/runs/status) stay `false`.
    undoable: bool,
}

/// Transaction-scoped recorder: owns the write transaction, accumulates the
/// changeset header plus row-level deltas, and writes them atomically with the
/// data on commit. Every state-mutating operation routes through this so the
/// `changesets` table is the single authoritative timeline.
pub(crate) struct Mutation<'c> {
    tx: Transaction<'c>,
    header: Option<ChangesetHeader>,
    deltas: Vec<DeltaRow>,
}

impl<'c> Mutation<'c> {
    fn new(tx: Transaction<'c>) -> Self {
        Self {
            tx,
            header: None,
            deltas: Vec::new(),
        }
    }

    /// Borrow the underlying connection for reads and lifecycle-table writes
    /// (claims/proposals/runs) that are intentionally not delta-captured.
    pub(crate) fn conn(&self) -> &Connection {
        &self.tx
    }

    /// Declare this transaction's changeset header. Called exactly once per
    /// mutation. `intent_json` must match the legacy event payload byte-for-byte.
    pub(crate) fn event(
        &mut self,
        verb: &str,
        object_type: &str,
        object_id: &str,
        intent_json: String,
        ts: u64,
    ) {
        self.header = Some(ChangesetHeader {
            ts,
            verb: verb.to_owned(),
            object_type: object_type.to_owned(),
            object_id: object_id.to_owned(),
            intent_json,
            undoable: false,
        });
    }

    /// Mark the current changeset as undoable. Call after `event()` for grooming
    /// verbs (reparent/edit/recast/unlink/merge) so PR4's `undo` can reverse them.
    pub(crate) fn mark_undoable(&mut self) {
        if let Some(header) = self.header.as_mut() {
            header.undoable = true;
        }
    }

    pub(crate) fn insert_item(&mut self, item: &Item) -> Result<(), RumbError> {
        insert_item(&self.tx, item)?;
        self.deltas.push(DeltaRow {
            table_name: "items",
            pk_json: json!({ "id": item.id }).to_string(),
            before_json: None,
            after_json: Some(item_row_value(item).to_string()),
        });
        Ok(())
    }

    pub(crate) fn update_item_status(
        &mut self,
        item_id: &str,
        status: Status,
        updated_at: u64,
    ) -> Result<(), RumbError> {
        let before = load_item(&self.tx, item_id)?;
        update_item_status_row(&self.tx, item_id, status, updated_at)?;
        let after = load_item(&self.tx, item_id)?;
        self.deltas.push(DeltaRow {
            table_name: "items",
            pk_json: json!({ "id": item_id }).to_string(),
            before_json: before.map(|item| item_row_value(&item).to_string()),
            after_json: after.map(|item| item_row_value(&item).to_string()),
        });
        Ok(())
    }

    /// Update every mutable column of an item (parent/kind/title/status/source),
    /// capturing the full before/after row image. Used by the grooming verbs.
    pub(crate) fn update_item(&mut self, item: &Item) -> Result<(), RumbError> {
        let before = load_item(&self.tx, &item.id)?;
        update_item_row(&self.tx, item)?;
        let after = load_item(&self.tx, &item.id)?;
        self.deltas.push(DeltaRow {
            table_name: "items",
            pk_json: json!({ "id": item.id }).to_string(),
            before_json: before.map(|item| item_row_value(&item).to_string()),
            after_json: after.map(|item| item_row_value(&item).to_string()),
        });
        Ok(())
    }

    /// Delete an item, capturing its row image as the `before` of the delta
    /// (`after` is `None`). Used by `undo` to reverse a create/capture.
    pub(crate) fn delete_item(&mut self, item: &Item) -> Result<(), RumbError> {
        let deleted = delete_item_row(&self.tx, &item.id)?;
        if deleted == 0 {
            return Err(RumbError::MissingItem(item.id.clone()));
        }
        self.deltas.push(DeltaRow {
            table_name: "items",
            pk_json: json!({ "id": item.id }).to_string(),
            before_json: Some(item_row_value(item).to_string()),
            after_json: None,
        });
        Ok(())
    }

    pub(crate) fn insert_edge(&mut self, edge: &Edge) -> Result<(), RumbError> {
        insert_edge(&self.tx, edge)?;
        self.deltas.push(DeltaRow {
            table_name: "edges",
            pk_json: edge_pk_json(edge),
            before_json: None,
            after_json: Some(edge_row_value(edge).to_string()),
        });
        Ok(())
    }

    /// Delete an edge, capturing its row image as the `before` of the delta
    /// (`after` is `None`). Errors if the edge does not exist.
    pub(crate) fn delete_edge(&mut self, edge: &Edge) -> Result<(), RumbError> {
        let deleted = delete_edge_row(&self.tx, &edge.from, &edge.to, edge.kind)?;
        if deleted == 0 {
            return Err(RumbError::MissingEdge(format!(
                "{}->{} ({})",
                edge.from, edge.to, edge.kind
            )));
        }
        self.deltas.push(DeltaRow {
            table_name: "edges",
            pk_json: edge_pk_json(edge),
            before_json: Some(edge_row_value(edge).to_string()),
            after_json: None,
        });
        Ok(())
    }

    fn commit(self) -> Result<(), RumbError> {
        let header = match self.header {
            Some(header) => header,
            None => {
                if !self.deltas.is_empty() {
                    return Err(RumbError::InvalidState(
                        "deltas recorded without a changeset".to_owned(),
                    ));
                }
                self.tx.commit()?;
                return Ok(());
            }
        };
        let seq: i64 = self.tx.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM changesets",
            [],
            |row| row.get(0),
        )?;
        self.tx.execute(
            r"
            INSERT INTO changesets
                (seq, ts, actor, verb, object_type, object_id, intent_json, undoable, kind)
            VALUES (?, ?, NULL, ?, ?, ?, ?, ?, 'event')
            ",
            params![
                seq,
                header.ts as i64,
                &header.verb,
                &header.object_type,
                &header.object_id,
                &header.intent_json,
                header.undoable,
            ],
        )?;
        for (idx, delta) in self.deltas.iter().enumerate() {
            self.tx.execute(
                r"
                INSERT INTO deltas
                    (changeset_seq, delta_idx, table_name, pk_json, before_json, after_json)
                VALUES (?, ?, ?, ?, ?, ?)
                ",
                params![
                    seq,
                    idx as i32,
                    delta.table_name,
                    &delta.pk_json,
                    &delta.before_json,
                    &delta.after_json,
                ],
            )?;
        }
        self.tx.commit()?;
        Ok(())
    }
}

impl RumbProject {
    /// Run a state mutation through the recorder. The body performs all writes
    /// via the `Mutation` handle (item/edge writes are delta-captured; reads and
    /// lifecycle-table writes go through its `conn()` accessor), and calls
    /// `m.event(..)` exactly once to declare the changeset. On success the
    /// changeset header and deltas are written atomically with the data.
    pub(crate) fn mutate<T>(
        &self,
        mut body: impl FnMut(&mut Mutation) -> Result<T, RumbError>,
    ) -> Result<T, RumbError> {
        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            let mut mutation = Mutation::new(tx);
            let result = body(&mut mutation)?;
            mutation.commit()?;
            Ok(result)
        })
    }
}
