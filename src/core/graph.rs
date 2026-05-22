use std::collections::{HashMap, HashSet};

use super::model::*;

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
