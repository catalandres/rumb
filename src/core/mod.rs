use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use duckdb::{params, Connection};
use serde_json::json;

mod capture;
mod claims;
mod digest;
mod graph;
mod grooming;
mod history;
mod model;
mod replay;
mod store;
mod undo;
use graph::{compute_ready, item_depth};
pub use model::*;
pub use store::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RumbProject {
    root: PathBuf,
}

impl RumbProject {
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn discover(start: impl AsRef<Path>) -> Result<Self, RumbError> {
        let mut path = start.as_ref().to_path_buf();
        loop {
            if path.join(STATE_DIR).is_dir() {
                return Ok(Self::open(path));
            }
            if !path.pop() {
                return Err(RumbError::NotInitialized(start.as_ref().to_path_buf()));
            }
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root.join(STATE_DIR)
    }

    pub fn state_file(&self) -> PathBuf {
        self.state_dir().join(STATE_FILE)
    }

    pub fn init(&self, options: &InitOptions) -> Result<(), RumbError> {
        fs::create_dir_all(self.state_dir())?;

        self.mutate(|m| {
            let now = timestamp();
            if !item_exists(m.conn(), ROOT_ID)? {
                let root = Item {
                    id: ROOT_ID.to_owned(),
                    parent_id: None,
                    kind: "project".to_owned(),
                    title: options.name.clone(),
                    status: Status::Ready,
                    tier: Tier::Standard,
                    source_ref: None,
                    body: None,
                    created_at: now,
                    updated_at: now,
                };
                m.insert_item(&root)?;
                m.event(
                    "init",
                    "project",
                    ROOT_ID,
                    json!({ "name": &options.name }).to_string(),
                    now,
                );
            }
            // Seed the inbox now that root exists (fresh repos; existing repos already
            // got it from migration 6). Raw insert — infrastructure, not timeline history.
            ensure_inbox(m.conn(), now)?;
            Ok(())
        })?;

        self.ensure_git_exclude()?;
        Ok(())
    }

    pub fn doctor(&self) -> Result<DoctorReport, RumbError> {
        Ok(DoctorReport {
            state_dir_exists: self.state_dir().is_dir(),
            state_file_exists: self.database_ready(),
            rumb_ignored_by_git: self.rumb_ignored_by_git()?,
        })
    }

    pub fn create_item(&self, input: CreateItem) -> Result<Item, RumbError> {
        if input.kind.trim().is_empty() {
            return Err(RumbError::EmptyKind);
        }
        if input.title.trim().is_empty() {
            return Err(RumbError::EmptyTitle);
        }

        self.mutate(|m| {
            if !item_exists(m.conn(), &input.parent_id)? {
                return Err(RumbError::MissingItem(input.parent_id.clone()));
            }

            let now = timestamp();
            let item = Item {
                id: next_item_id(m.conn())?,
                parent_id: Some(input.parent_id.clone()),
                kind: input.kind.clone(),
                title: input.title.clone(),
                status: input.status,
                tier: input.tier,
                source_ref: input.source_ref.clone(),
                body: None,
                created_at: now,
                updated_at: now,
            };
            m.insert_item(&item)?;
            m.event(
                "item.create",
                "item",
                &item.id,
                json!({
                    "kind": &item.kind,
                    "status": item.status.to_string(),
                    "tier": item.tier.to_string(),
                    "parent_id": item.parent_id.as_deref(),
                    "source_ref": item.source_ref.as_deref(),
                })
                .to_string(),
                now,
            );

            Ok(item)
        })
    }

    pub fn add_edge(&self, input: AddEdge) -> Result<Edge, RumbError> {
        self.mutate(|m| {
            if !item_exists(m.conn(), &input.from)? {
                return Err(RumbError::MissingItem(input.from.clone()));
            }
            if !item_exists(m.conn(), &input.to)? {
                return Err(RumbError::MissingItem(input.to.clone()));
            }

            let now = timestamp();
            let edge = Edge {
                from: input.from.clone(),
                to: input.to.clone(),
                kind: input.kind,
                created_at: now,
            };
            m.insert_edge(&edge)?;
            m.event(
                "edge.add",
                "edge",
                &format!("{}->{}", edge.from, edge.to),
                json!({
                    "from": &edge.from,
                    "to": &edge.to,
                    "kind": edge.kind.to_string(),
                })
                .to_string(),
                now,
            );

            Ok(edge)
        })
    }

    pub fn ready_items(&self) -> Result<Vec<Item>, RumbError> {
        with_storage_retry(|| {
            let conn = self.open_database()?;
            let items = load_items(&conn)?;
            let edges = load_edges(&conn)?;
            let claimed_item_ids = active_claim_item_ids(&conn, timestamp())?;
            let reserved = reserved_node_ids(&conn)?;
            Ok(compute_ready(&items, &edges, &claimed_item_ids, &reserved))
        })
    }

    pub fn list_items(&self) -> Result<Vec<Item>, RumbError> {
        with_storage_retry(|| {
            let conn = self.open_database()?;
            load_items(&conn)
        })
    }

    pub fn item_details(&self, reference: &str) -> Result<ItemDetails, RumbError> {
        let item_id = normalize_item_id(reference)?;
        with_storage_retry(|| {
            let conn = self.open_database()?;
            let items = load_items(&conn)?;
            let item = items
                .iter()
                .find(|item| item.id == item_id)
                .cloned()
                .ok_or_else(|| RumbError::MissingItem(item_id.clone()))?;
            let depth = item_depth(&item.id, &items)?;
            let children = items
                .iter()
                .filter(|child| child.parent_id.as_deref() == Some(item.id.as_str()))
                .cloned()
                .collect();
            let edges = load_edges(&conn)?;
            let incoming_edges = edges
                .iter()
                .filter(|edge| edge.to == item.id)
                .cloned()
                .collect();
            let outgoing_edges = edges
                .iter()
                .filter(|edge| edge.from == item.id)
                .cloned()
                .collect();
            let claims = load_claims_for_item(&conn, &item.id)?;
            let proposals = load_proposals_for_item(&conn, &item.id)?;
            let runs = load_runs_for_item(&conn, &item.id)?;
            let events = load_events_for_item(&conn, &item.id)?;

            Ok(ItemDetails {
                item,
                depth,
                children,
                incoming_edges,
                outgoing_edges,
                claims,
                proposals,
                runs,
                events,
            })
        })
    }

    pub fn update_item_status(&self, input: UpdateItemStatus) -> Result<Item, RumbError> {
        self.mutate(|m| {
            let mut item = load_item(m.conn(), &input.item_id)?
                .ok_or_else(|| RumbError::MissingItem(input.item_id.clone()))?;
            let now = timestamp();
            item.status = input.status;
            item.updated_at = now;
            m.update_item_status(&item.id, item.status, now)?;
            m.event(
                "item.status",
                "item",
                &item.id,
                json!({
                    "actor": &input.actor,
                    "status": item.status.to_string(),
                    "kind": &item.kind,
                })
                .to_string(),
                now,
            );
            Ok(item)
        })
    }

    pub fn review_item(&self, input: ReviewItem) -> Result<Item, RumbError> {
        self.transition_item(
            &input.item_id,
            Status::InReview,
            &input.actor,
            "item.review",
        )
    }

    pub fn done_item(&self, input: DoneItem) -> Result<Item, RumbError> {
        self.transition_item(&input.item_id, Status::Done, &input.actor, "item.done")
    }

    fn transition_item(
        &self,
        item_id: &str,
        status: Status,
        actor: &str,
        action: &'static str,
    ) -> Result<Item, RumbError> {
        self.mutate(|m| {
            let mut item = load_item(m.conn(), item_id)?
                .ok_or_else(|| RumbError::MissingItem(item_id.to_owned()))?;
            let now = timestamp();
            item.status = status;
            item.updated_at = now;
            m.update_item_status(&item.id, item.status, now)?;
            m.event(
                action,
                "item",
                &item.id,
                json!({
                    "actor": actor,
                    "status": item.status.to_string(),
                    "kind": &item.kind,
                })
                .to_string(),
                now,
            );
            Ok(item)
        })
    }

    pub fn events(&self, id: Option<&str>) -> Result<Vec<Event>, RumbError> {
        with_storage_retry(|| {
            let conn = self.open_database()?;
            let mut events = Vec::new();
            let sql = match id {
                Some(_) => {
                    "SELECT timestamp, action, object_type, object_id, payload_json FROM events WHERE object_id = ? ORDER BY seq"
                }
                None => {
                    "SELECT timestamp, action, object_type, object_id, payload_json FROM events ORDER BY seq"
                }
            };
            let mut stmt = conn.prepare(sql)?;
            let rows = match id {
                Some(id) => stmt.query_map(params![id], map_event_row)?,
                None => stmt.query_map([], map_event_row)?,
            };

            for row in rows {
                events.push(row?);
            }
            Ok(events)
        })
    }

    fn open_database(&self) -> Result<Connection, RumbError> {
        let mut conn = Connection::open(self.state_file())?;
        ensure_schema(&mut conn)?;
        Ok(conn)
    }

    fn database_ready(&self) -> bool {
        if !self.state_file().is_file() {
            return false;
        }
        with_storage_retry(|| {
            Connection::open(self.state_file())
                .and_then(|conn| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM migrations WHERE version = ?",
                        params![CURRENT_SCHEMA_VERSION],
                        |row| row.get::<_, i64>(0),
                    )
                })
                .map_err(RumbError::Storage)
        })
        .is_ok_and(|count| count == 1)
    }

    fn ensure_git_exclude(&self) -> Result<(), RumbError> {
        let exclude = self.root.join(".git/info/exclude");
        if let Some(parent) = exclude.parent() {
            if !parent.exists() {
                return Ok(());
            }
        }
        let existing = fs::read_to_string(&exclude).unwrap_or_default();
        if existing.lines().any(|line| line.trim() == ".rumb/") {
            return Ok(());
        }

        let mut file = OpenOptions::new().append(true).create(true).open(exclude)?;
        if !existing.ends_with('\n') && !existing.is_empty() {
            writeln!(file)?;
        }
        writeln!(file, ".rumb/")?;
        Ok(())
    }

    fn rumb_ignored_by_git(&self) -> Result<bool, RumbError> {
        if !self.root.join(".git").exists() {
            return Ok(false);
        }

        let output = Command::new("git")
            .arg("check-ignore")
            .arg("--quiet")
            .arg(".rumb/")
            .current_dir(&self.root)
            .status();

        match output {
            Ok(status) if status.success() => Ok(true),
            Ok(_) | Err(_) => {
                let exclude = self.root.join(".git/info/exclude");
                let ignored = fs::read_to_string(exclude)
                    .unwrap_or_default()
                    .lines()
                    .any(|line| line.trim() == ".rumb/");
                Ok(ignored)
            }
        }
    }
}

#[cfg(test)]
mod tests;
