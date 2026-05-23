use std::collections::HashSet;

use serde_json::json;

use super::graph::{compute_ready, would_create_cycle};
use super::model::*;
use super::store::*;
use super::RumbProject;

impl RumbProject {
    /// Move an item under a new parent. Rejects cycles, blocks while the item has
    /// an active claim, refuses reserved nodes, and (like `claim`) requires
    /// `confirm` when the move lands the item at depth 1 (directly under root).
    pub fn reparent(&self, input: Reparent) -> Result<Item, RumbError> {
        self.mutate(|m| {
            let mut item = load_item(m.conn(), &input.item_id)?
                .ok_or_else(|| RumbError::MissingItem(input.item_id.clone()))?;
            if is_reserved_node(m.conn(), &item.id)? {
                return Err(RumbError::ReservedNode(item.id.clone()));
            }
            let now = timestamp();
            if active_claim_for_item(m.conn(), &item.id, now)?.is_some() {
                return Err(RumbError::GroomingBlockedByClaim(item.id.clone()));
            }
            if !item_exists(m.conn(), &input.new_parent_id)? {
                return Err(RumbError::MissingItem(input.new_parent_id.clone()));
            }
            let items = load_items(m.conn())?;
            if would_create_cycle(&item.id, &input.new_parent_id, &items) {
                return Err(RumbError::InvalidParentChain(input.new_parent_id.clone()));
            }
            if input.new_parent_id == ROOT_ID && !input.confirm {
                return Err(RumbError::FoundationRequiresConfirm);
            }

            item.parent_id = Some(input.new_parent_id.clone());
            item.updated_at = now;
            m.update_item(&item)?;
            m.event(
                "item.reparent",
                "item",
                &item.id,
                with_note(
                    json!({
                        "actor": &input.actor,
                        "parent_id": &input.new_parent_id,
                    }),
                    &input.note,
                ),
                now,
            );
            m.mark_undoable();
            Ok(item)
        })
    }

    /// Set previously-immutable fields (title, source_ref, and/or tier). Claim-safe
    /// and allowed on reserved nodes (renaming the project root is not destructive).
    pub fn edit(&self, input: EditItem) -> Result<Item, RumbError> {
        if input.title.is_none() && input.source_ref.is_none() && input.tier.is_none() {
            return Err(RumbError::NoGroomingChanges);
        }
        if input
            .title
            .as_deref()
            .is_some_and(|title| title.trim().is_empty())
        {
            return Err(RumbError::EmptyTitle);
        }

        self.mutate(|m| {
            let mut item = load_item(m.conn(), &input.item_id)?
                .ok_or_else(|| RumbError::MissingItem(input.item_id.clone()))?;
            let now = timestamp();
            if let Some(title) = &input.title {
                item.title = title.clone();
            }
            if let Some(source_ref) = &input.source_ref {
                item.source_ref = Some(source_ref.clone());
            }
            if let Some(tier) = input.tier {
                item.tier = tier;
            }
            item.updated_at = now;
            m.update_item(&item)?;
            m.event(
                "item.edit",
                "item",
                &item.id,
                with_note(
                    json!({
                        "actor": &input.actor,
                        "title": input.title.as_deref(),
                        "source_ref": input.source_ref.as_deref(),
                        "tier": input.tier.map(|tier| tier.to_string()),
                    }),
                    &input.note,
                ),
                now,
            );
            m.mark_undoable();
            Ok(item)
        })
    }

    /// Change an item's kind (types-as-role: a task matures into a spec). Kind
    /// stays free-form; only non-empty is enforced. Claim-safe; reserved nodes
    /// keep their structural kind.
    pub fn recast(&self, input: Recast) -> Result<Item, RumbError> {
        if input.kind.trim().is_empty() {
            return Err(RumbError::EmptyKind);
        }

        self.mutate(|m| {
            let mut item = load_item(m.conn(), &input.item_id)?
                .ok_or_else(|| RumbError::MissingItem(input.item_id.clone()))?;
            if is_reserved_node(m.conn(), &item.id)? {
                return Err(RumbError::ReservedNode(item.id.clone()));
            }
            let now = timestamp();
            let previous_kind = item.kind.clone();
            item.kind = input.kind.clone();
            item.updated_at = now;
            m.update_item(&item)?;
            m.event(
                "item.recast",
                "item",
                &item.id,
                with_note(
                    json!({
                        "actor": &input.actor,
                        "kind": &input.kind,
                        "previous_kind": previous_kind,
                    }),
                    &input.note,
                ),
                now,
            );
            m.mark_undoable();
            Ok(item)
        })
    }

    /// Remove a graph edge. Reports any items that became ready as a direct
    /// consequence (removing a `depends_on`/`blocks` edge can unblock work) so a
    /// freshly-ready item is surfaced rather than silently dropped. Claim-safe.
    pub fn unlink(&self, input: Unlink) -> Result<UnlinkOutcome, RumbError> {
        self.mutate(|m| {
            let edge =
                load_edge(m.conn(), &input.from, &input.to, input.kind)?.ok_or_else(|| {
                    RumbError::MissingEdge(format!("{}->{} ({})", input.from, input.to, input.kind))
                })?;

            let now = timestamp();
            let items = load_items(m.conn())?;
            let claimed = active_claim_item_ids(m.conn(), now)?;
            let reserved = reserved_node_ids(m.conn())?;
            let ready_before: HashSet<String> =
                compute_ready(&items, &load_edges(m.conn())?, &claimed, &reserved)
                    .into_iter()
                    .map(|item| item.id)
                    .collect();

            m.delete_edge(&edge)?;

            // Removing an edge can only relax readiness, so any item ready now that
            // was not ready before is newly unblocked. Items are unchanged by an
            // edge delete, so the same `items` snapshot is reused.
            let newly_ready = compute_ready(&items, &load_edges(m.conn())?, &claimed, &reserved)
                .into_iter()
                .filter(|item| !ready_before.contains(&item.id))
                .collect();

            m.event(
                "edge.unlink",
                "edge",
                &format!("{}->{}", input.from, input.to),
                with_note(
                    json!({
                        "actor": &input.actor,
                        "from": &input.from,
                        "to": &input.to,
                        "kind": input.kind.to_string(),
                    }),
                    &input.note,
                ),
                now,
            );
            m.mark_undoable();
            Ok(UnlinkOutcome { edge, newly_ready })
        })
    }

    /// Merge `from` into `into`: reparent `from`'s children under `into`, rewire
    /// `from`'s edges to `into` (dropping self-loops and duplicates), record a
    /// `supersedes` edge `into -> from`, and mark `from` superseded. `from` is
    /// never deleted, so its claims/proposals/runs stay valid history. Blocks
    /// while `from` has an active claim; refuses reserved nodes and cycles.
    pub fn merge(&self, input: Merge) -> Result<MergeOutcome, RumbError> {
        if input.from_id == input.into_id {
            return Err(RumbError::CannotMergeIntoSelf(input.from_id));
        }

        self.mutate(|m| {
            let mut from = load_item(m.conn(), &input.from_id)?
                .ok_or_else(|| RumbError::MissingItem(input.from_id.clone()))?;
            let into = load_item(m.conn(), &input.into_id)?
                .ok_or_else(|| RumbError::MissingItem(input.into_id.clone()))?;
            if is_reserved_node(m.conn(), &from.id)? {
                return Err(RumbError::ReservedNode(from.id.clone()));
            }
            let now = timestamp();
            if active_claim_for_item(m.conn(), &from.id, now)?.is_some() {
                return Err(RumbError::GroomingBlockedByClaim(from.id.clone()));
            }
            let items = load_items(m.conn())?;
            // `into` must not be `from` or a descendant of it, or reparenting
            // children (and the supersedes edge) would form a cycle.
            if would_create_cycle(&from.id, &into.id, &items) {
                return Err(RumbError::InvalidParentChain(into.id.clone()));
            }

            // 1. Move from's children under into.
            let mut moved_children = Vec::new();
            for child in items
                .iter()
                .filter(|item| item.parent_id.as_deref() == Some(from.id.as_str()))
            {
                let mut child = child.clone();
                child.parent_id = Some(into.id.clone());
                child.updated_at = now;
                m.update_item(&child)?;
                moved_children.push(child.id);
            }

            // 2. Rewire from's edges to into, dropping self-loops and duplicates.
            let edges = load_edges(m.conn())?;
            let existing: HashSet<(String, String, String)> = edges
                .iter()
                .map(|edge| (edge.from.clone(), edge.to.clone(), edge.kind.to_string()))
                .collect();
            let mut planned: HashSet<(String, String, String)> = HashSet::new();
            for edge in edges
                .iter()
                .filter(|edge| edge.from == from.id || edge.to == from.id)
            {
                m.delete_edge(edge)?;
                let new_from = replace_endpoint(&edge.from, &from.id, &into.id);
                let new_to = replace_endpoint(&edge.to, &from.id, &into.id);
                let key = (new_from.clone(), new_to.clone(), edge.kind.to_string());
                if new_from != new_to && !existing.contains(&key) && planned.insert(key) {
                    m.insert_edge(&Edge {
                        from: new_from,
                        to: new_to,
                        kind: edge.kind,
                        created_at: now,
                    })?;
                }
            }

            // 3. Supersede from (status only; the row otherwise stays as history).
            m.update_item_status(&from.id, Status::Superseded, now)?;
            from.status = Status::Superseded;
            from.updated_at = now;

            // 4. Record into -> from supersedes. Step 2 deleted any prior from-incident
            // edge (including an old into->from), and never plans an into->from edge,
            // so this insert cannot collide.
            let supersedes_edge = Edge {
                from: into.id.clone(),
                to: from.id.clone(),
                kind: EdgeKind::Supersedes,
                created_at: now,
            };
            m.insert_edge(&supersedes_edge)?;

            m.event(
                "item.merge",
                "item",
                &from.id,
                with_note(
                    json!({
                        "actor": &input.actor,
                        "from": &from.id,
                        "into": &into.id,
                        "moved_children": &moved_children,
                    }),
                    &input.note,
                ),
                now,
            );
            m.mark_undoable();
            Ok(MergeOutcome {
                from,
                into,
                moved_children,
                supersedes_edge,
            })
        })
    }
}

/// Merge an optional rejected-alternative note into a grooming verb's intent
/// payload, so history records *why* (and what was rejected), not just *what*.
fn with_note(value: serde_json::Value, note: &GroomNote) -> String {
    let mut value = value;
    if let serde_json::Value::Object(map) = &mut value {
        if let Some(rejected) = &note.rejected {
            map.insert("rejected".to_owned(), json!(rejected));
        }
        if let Some(why) = &note.why {
            map.insert("why".to_owned(), json!(why));
        }
    }
    value.to_string()
}

/// Rewrite an edge endpoint during a merge: `from` becomes `into`, everything
/// else is unchanged.
fn replace_endpoint(endpoint: &str, from: &str, into: &str) -> String {
    if endpoint == from {
        into.to_owned()
    } else {
        endpoint.to_owned()
    }
}
