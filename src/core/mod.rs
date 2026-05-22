use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use duckdb::{params, Connection};
use serde_json::json;

mod claims;
mod graph;
mod model;
mod store;
use graph::{dependencies_satisfied, item_depth};
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

        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            if !item_exists(&tx, ROOT_ID)? {
                let now = timestamp();
                let root = Item {
                    id: ROOT_ID.to_owned(),
                    parent_id: None,
                    kind: "project".to_owned(),
                    title: options.name.clone(),
                    status: Status::Ready,
                    source_ref: None,
                    created_at: now,
                    updated_at: now,
                };
                insert_item(&tx, &root)?;
                append_event(
                    &tx,
                    EventInput {
                        timestamp: now,
                        action: "init",
                        object_type: "project",
                        object_id: ROOT_ID,
                        payload: json!({ "name": &options.name }).to_string(),
                    },
                )?;
            }
            tx.commit()?;
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

        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            if !item_exists(&tx, &input.parent_id)? {
                return Err(RumbError::MissingItem(input.parent_id.clone()));
            }

            let now = timestamp();
            let item = Item {
                id: next_item_id(&tx)?,
                parent_id: Some(input.parent_id.clone()),
                kind: input.kind.clone(),
                title: input.title.clone(),
                status: input.status,
                source_ref: input.source_ref.clone(),
                created_at: now,
                updated_at: now,
            };
            insert_item(&tx, &item)?;
            append_event(
                &tx,
                EventInput {
                    timestamp: now,
                    action: "item.create",
                    object_type: "item",
                    object_id: &item.id,
                    payload: json!({
                        "kind": &item.kind,
                        "status": item.status.to_string(),
                        "parent_id": item.parent_id.as_deref(),
                        "source_ref": item.source_ref.as_deref(),
                    })
                    .to_string(),
                },
            )?;
            tx.commit()?;

            Ok(item)
        })
    }

    pub fn add_edge(&self, input: AddEdge) -> Result<Edge, RumbError> {
        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            if !item_exists(&tx, &input.from)? {
                return Err(RumbError::MissingItem(input.from.clone()));
            }
            if !item_exists(&tx, &input.to)? {
                return Err(RumbError::MissingItem(input.to.clone()));
            }

            let now = timestamp();
            let edge = Edge {
                from: input.from.clone(),
                to: input.to.clone(),
                kind: input.kind,
                created_at: now,
            };
            insert_edge(&tx, &edge)?;
            append_event(
                &tx,
                EventInput {
                    timestamp: now,
                    action: "edge.add",
                    object_type: "edge",
                    object_id: &format!("{}->{}", edge.from, edge.to),
                    payload: json!({
                        "from": &edge.from,
                        "to": &edge.to,
                        "kind": edge.kind.to_string(),
                    })
                    .to_string(),
                },
            )?;
            tx.commit()?;

            Ok(edge)
        })
    }

    pub fn ready_items(&self) -> Result<Vec<Item>, RumbError> {
        with_storage_retry(|| {
            let conn = self.open_database()?;
            let items = load_items(&conn)?;
            let edges = load_edges(&conn)?;
            let claimed_item_ids = active_claim_item_ids(&conn, timestamp())?;
            let ready = items
                .iter()
                .filter(|item| item.id != ROOT_ID)
                .filter(|item| !claimed_item_ids.contains(&item.id))
                .filter(|item| matches!(item.status, Status::Ready | Status::Claimed))
                .filter(|item| dependencies_satisfied(item, &edges, &items))
                .cloned()
                .collect();
            Ok(ready)
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
        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            let mut item = load_item(&tx, &input.item_id)?
                .ok_or_else(|| RumbError::MissingItem(input.item_id.clone()))?;
            let now = timestamp();
            item.status = input.status;
            item.updated_at = now;
            update_item_status_row(&tx, &item.id, item.status, now)?;
            append_event(
                &tx,
                EventInput {
                    timestamp: now,
                    action: "item.status",
                    object_type: "item",
                    object_id: &item.id,
                    payload: json!({
                        "actor": &input.actor,
                        "status": item.status.to_string(),
                    })
                    .to_string(),
                },
            )?;
            tx.commit()?;
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
        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            let mut item = load_item(&tx, item_id)?
                .ok_or_else(|| RumbError::MissingItem(item_id.to_owned()))?;
            let now = timestamp();
            item.status = status;
            item.updated_at = now;
            update_item_status_row(&tx, &item.id, item.status, now)?;
            append_event(
                &tx,
                EventInput {
                    timestamp: now,
                    action,
                    object_type: "item",
                    object_id: &item.id,
                    payload: json!({
                        "actor": actor,
                        "status": item.status.to_string(),
                    })
                    .to_string(),
                },
            )?;
            tx.commit()?;
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
