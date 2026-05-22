use std::collections::{HashMap, HashSet};

use super::model::*;

/// The items that are claimable right now: not the root, not actively claimed,
/// `ready` status, and every dependency satisfied. Shared by `ready_items` and
/// `unlink` so the "newly ready" calculation can never drift from real readiness.
pub(crate) fn compute_ready(
    items: &[Item],
    edges: &[Edge],
    claimed_ids: &HashSet<String>,
    reserved_ids: &HashSet<String>,
) -> Vec<Item> {
    items
        .iter()
        .filter(|item| !reserved_ids.contains(&item.id))
        .filter(|item| !claimed_ids.contains(&item.id))
        .filter(|item| item.status == Status::Ready)
        .filter(|item| dependencies_satisfied(item, edges, items))
        .cloned()
        .collect()
}

/// Would setting `item_id`'s parent to `new_parent_id` create a cycle? True when
/// `new_parent_id` is `item_id` itself or one of its descendants — i.e. walking
/// the parent chain up from `new_parent_id` reaches `item_id`.
pub(crate) fn would_create_cycle(item_id: &str, new_parent_id: &str, items: &[Item]) -> bool {
    let by_id: HashMap<&str, &Item> = items.iter().map(|item| (item.id.as_str(), item)).collect();
    let mut seen = HashSet::new();
    let mut current = new_parent_id.to_owned();
    loop {
        if current == item_id {
            return true;
        }
        if !seen.insert(current.clone()) {
            return false;
        }
        match by_id
            .get(current.as_str())
            .and_then(|item| item.parent_id.clone())
        {
            Some(parent) => current = parent,
            None => return false,
        }
    }
}

pub(crate) fn dependencies_satisfied(item: &Item, edges: &[Edge], items: &[Item]) -> bool {
    edges.iter().all(|edge| match edge.kind {
        EdgeKind::DependsOn if edge.from == item.id => items
            .iter()
            .find(|dependency| dependency.id == edge.to)
            .is_some_and(|dependency| dependency.status == Status::Done),
        EdgeKind::Blocks if edge.to == item.id => items
            .iter()
            .find(|blocker| blocker.id == edge.from)
            .is_some_and(|blocker| blocker.status == Status::Done),
        _ => true,
    })
}

pub(crate) fn item_depth(item_id: &str, items: &[Item]) -> Result<usize, RumbError> {
    let by_id: HashMap<&str, &Item> = items.iter().map(|item| (item.id.as_str(), item)).collect();
    let mut seen = HashSet::new();
    let mut depth = 0;
    let mut current = item_id;

    loop {
        if !seen.insert(current.to_owned()) {
            return Err(RumbError::InvalidParentChain(current.to_owned()));
        }
        let item = by_id
            .get(current)
            .ok_or_else(|| RumbError::MissingItem(current.to_owned()))?;
        match item.parent_id.as_deref() {
            Some(parent_id) => {
                depth += 1;
                current = parent_id;
            }
            None => return Ok(depth),
        }
    }
}
