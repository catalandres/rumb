use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::ValueEnum;
use duckdb::{params, Connection};
use serde_json::json;
use thiserror::Error;

const STATE_DIR: &str = ".rumb";
const STATE_FILE: &str = "state.duckdb";
const ROOT_ID: &str = "RUMB-0000";
const CURRENT_SCHEMA_VERSION: i32 = 3;
const DEFAULT_TTL_SECONDS: u64 = 4 * 60 * 60;
const STORAGE_RETRY_ATTEMPTS: usize = 5;

#[derive(Debug, Error)]
pub enum RumbError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage error: {0}")]
    Storage(#[from] duckdb::Error),
    #[error("could not find .rumb from {0}")]
    NotInitialized(PathBuf),
    #[error("invalid state: {0}")]
    InvalidState(String),
    #[error("invalid status: {0}")]
    InvalidStatus(String),
    #[error("invalid edge kind: {0}")]
    InvalidEdgeKind(String),
    #[error("invalid ttl: {0}")]
    InvalidTtl(String),
    #[error("invalid item reference: {0}")]
    InvalidItemRef(String),
    #[error("item does not exist: {0}")]
    MissingItem(String),
    #[error("claim does not exist: {0}")]
    MissingClaim(String),
    #[error("item kind must not be empty")]
    EmptyKind,
    #[error("item title must not be empty")]
    EmptyTitle,
    #[error("run command must not be empty")]
    EmptyCommand,
    #[error("root item cannot be claimed")]
    RootCannotClaim,
    #[error("depth 1 item requires --confirm-foundation")]
    FoundationRequiresConfirm,
    #[error("item is not ready: {0}")]
    ItemNotReady(String),
    #[error("item has unsatisfied dependencies: {0}")]
    UnsatisfiedDependencies(String),
    #[error("item already has an active claim: {0}")]
    ClaimAlreadyActive(String),
    #[error("claim is not active: {0}")]
    ClaimNotActive(String),
    #[error("claim actor mismatch: expected {expected}, got {actual}")]
    ClaimActorMismatch { expected: String, actual: String },
    #[error("invalid parent chain at item: {0}")]
    InvalidParentChain(String),
    #[error("git command failed: {0}")]
    GitFailed(String),
    #[error("mcp install error: {0}")]
    McpInstall(String),
    #[error("doctor checks failed")]
    DoctorFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RumbProject {
    root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitOptions {
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateItem {
    pub kind: String,
    pub title: String,
    pub parent_id: String,
    pub status: Status,
    pub source_ref: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddEdge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimItem {
    pub item_id: String,
    pub actor: String,
    pub ttl_seconds: u64,
    pub confirm_foundation: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenewClaim {
    pub claim_id: String,
    pub actor: String,
    pub ttl_seconds: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseClaim {
    pub claim_id: String,
    pub actor: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateItemStatus {
    pub item_id: String,
    pub status: Status,
    pub actor: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunCommand {
    pub item_id: String,
    pub actor: String,
    pub command: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewItem {
    pub item_id: String,
    pub actor: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoneItem {
    pub item_id: String,
    pub actor: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub status: Status,
    pub source_ref: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub created_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Event {
    pub timestamp: u64,
    pub action: String,
    pub object_type: String,
    pub object_id: String,
    pub payload: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Claim {
    pub id: String,
    pub item_id: String,
    pub actor_id: String,
    pub lease_until: u64,
    pub status: ClaimStatus,
    pub branch: String,
    pub worktree_path: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunRecord {
    pub id: String,
    pub item_id: String,
    pub proposal_id: Option<String>,
    pub command: String,
    pub status: RunStatus,
    pub output_path: String,
    pub started_at: u64,
    pub finished_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    pub id: String,
    pub item_id: String,
    pub branch: String,
    pub base_ref: String,
    pub head_ref: Option<String>,
    pub status: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemDetails {
    pub item: Item,
    pub depth: usize,
    pub children: Vec<Item>,
    pub incoming_edges: Vec<Edge>,
    pub outgoing_edges: Vec<Edge>,
    pub claims: Vec<Claim>,
    pub proposals: Vec<Proposal>,
    pub runs: Vec<RunRecord>,
    pub events: Vec<Event>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Running,
    Passed,
    Failed,
}

impl Display for RunStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
        })
    }
}

impl FromStr for RunStatus {
    type Err = RumbError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "running" => Ok(Self::Running),
            "passed" => Ok(Self::Passed),
            "failed" => Ok(Self::Failed),
            _ => Err(RumbError::InvalidState(format!(
                "invalid run status: {value}"
            ))),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ClaimStatus {
    Pending,
    Active,
    Released,
    Failed,
}

impl Display for ClaimStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Released => "released",
            Self::Failed => "failed",
        })
    }
}

impl FromStr for ClaimStatus {
    type Err = RumbError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "active" => Ok(Self::Active),
            "released" => Ok(Self::Released),
            "failed" => Ok(Self::Failed),
            _ => Err(RumbError::InvalidState(format!(
                "invalid claim status: {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoctorReport {
    pub state_dir_exists: bool,
    pub state_file_exists: bool,
    pub rumb_ignored_by_git: bool,
}

impl DoctorReport {
    pub fn ok(&self) -> bool {
        self.state_dir_exists && self.state_file_exists && self.rumb_ignored_by_git
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum Status {
    Draft,
    Ready,
    Blocked,
    Claimed,
    InReview,
    Done,
    Superseded,
    Abandoned,
}

impl Default for Status {
    fn default() -> Self {
        Self::Draft
    }
}

impl Display for Status {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Blocked => "blocked",
            Self::Claimed => "claimed",
            Self::InReview => "in_review",
            Self::Done => "done",
            Self::Superseded => "superseded",
            Self::Abandoned => "abandoned",
        })
    }
}

impl FromStr for Status {
    type Err = RumbError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "draft" => Ok(Self::Draft),
            "ready" => Ok(Self::Ready),
            "blocked" => Ok(Self::Blocked),
            "claimed" => Ok(Self::Claimed),
            "in_review" => Ok(Self::InReview),
            "done" => Ok(Self::Done),
            "superseded" => Ok(Self::Superseded),
            "abandoned" => Ok(Self::Abandoned),
            _ => Err(RumbError::InvalidStatus(value.to_owned())),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum EdgeKind {
    DependsOn,
    Blocks,
    RelatesTo,
    Supersedes,
    Implements,
}

impl Display for EdgeKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DependsOn => "depends_on",
            Self::Blocks => "blocks",
            Self::RelatesTo => "relates_to",
            Self::Supersedes => "supersedes",
            Self::Implements => "implements",
        })
    }
}

impl FromStr for EdgeKind {
    type Err = RumbError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "depends_on" => Ok(Self::DependsOn),
            "blocks" => Ok(Self::Blocks),
            "relates_to" => Ok(Self::RelatesTo),
            "supersedes" => Ok(Self::Supersedes),
            "implements" => Ok(Self::Implements),
            _ => Err(RumbError::InvalidEdgeKind(value.to_owned())),
        }
    }
}

#[derive(Debug)]
struct DbItem {
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
struct DbEdge {
    from: String,
    to: String,
    kind: String,
    created_at: i64,
}

#[derive(Debug)]
struct DbClaim {
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
struct DbProposal {
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
struct DbRun {
    id: String,
    item_id: String,
    proposal_id: Option<String>,
    command: String,
    status: String,
    output_path: String,
    started_at: i64,
    finished_at: Option<i64>,
}

#[derive(Debug)]
struct ClaimReservation {
    claim: Claim,
    proposal_id: String,
}

#[derive(Debug)]
struct RunReservation {
    id: String,
    item_id: String,
    proposal_id: Option<String>,
    command: String,
    output_path: String,
    started_at: u64,
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

    pub fn claim_item(&self, input: ClaimItem) -> Result<Claim, RumbError> {
        let reservation = self.reserve_claim(&input)?;
        let git_result = self.create_claim_worktree(&reservation.claim);

        match git_result {
            Ok(()) => self.activate_claim(&reservation),
            Err(err) => {
                let _ = self.fail_claim(&reservation, &err.to_string());
                Err(err)
            }
        }
    }

    pub fn renew_claim(&self, input: RenewClaim) -> Result<Claim, RumbError> {
        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            let mut claim = load_claim(&tx, &input.claim_id)?
                .ok_or_else(|| RumbError::MissingClaim(input.claim_id.clone()))?;
            if claim.actor_id != input.actor {
                return Err(RumbError::ClaimActorMismatch {
                    expected: claim.actor_id,
                    actual: input.actor.clone(),
                });
            }
            let now = timestamp();
            if claim.status != ClaimStatus::Active || claim.lease_until <= now {
                return Err(RumbError::ClaimNotActive(input.claim_id.clone()));
            }

            claim.lease_until = now + input.ttl_seconds;
            claim.updated_at = now;
            update_claim_lease(&tx, &claim)?;
            append_event(
                &tx,
                EventInput {
                    timestamp: now,
                    action: "claim.renew",
                    object_type: "item",
                    object_id: &claim.item_id,
                    payload: json!({
                        "claim_id": &claim.id,
                        "actor": &input.actor,
                        "lease_until": claim.lease_until,
                    })
                    .to_string(),
                },
            )?;
            tx.commit()?;
            Ok(claim)
        })
    }

    pub fn release_claim(&self, input: ReleaseClaim) -> Result<Claim, RumbError> {
        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            let mut claim = load_claim(&tx, &input.claim_id)?
                .ok_or_else(|| RumbError::MissingClaim(input.claim_id.clone()))?;
            if claim.actor_id != input.actor {
                return Err(RumbError::ClaimActorMismatch {
                    expected: claim.actor_id,
                    actual: input.actor.clone(),
                });
            }
            if !matches!(claim.status, ClaimStatus::Active | ClaimStatus::Pending) {
                return Err(RumbError::ClaimNotActive(input.claim_id.clone()));
            }

            let now = timestamp();
            claim.status = ClaimStatus::Released;
            claim.updated_at = now;
            update_claim_status(&tx, &claim)?;
            update_proposal_status_for_claim(&tx, &claim, "released", now)?;

            if item_status(&tx, &claim.item_id)? == Some(Status::Claimed)
                && !has_other_active_claim(&tx, &claim.id, &claim.item_id, now)?
            {
                update_item_status_row(&tx, &claim.item_id, Status::Ready, now)?;
            }

            append_event(
                &tx,
                EventInput {
                    timestamp: now,
                    action: "claim.release",
                    object_type: "item",
                    object_id: &claim.item_id,
                    payload: json!({
                        "claim_id": &claim.id,
                        "actor": &input.actor,
                    })
                    .to_string(),
                },
            )?;
            tx.commit()?;
            Ok(claim)
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

    pub fn run_command(&self, input: RunCommand) -> Result<RunRecord, RumbError> {
        if input.command.is_empty() {
            return Err(RumbError::EmptyCommand);
        }

        let reservation = self.reserve_run(&input)?;
        let output_path = self.root.join(&reservation.output_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let output = Command::new(&input.command[0])
            .args(&input.command[1..])
            .current_dir(&self.root)
            .output();
        let finished_at = timestamp();
        let (status, exit_code, stdout, stderr) = match output {
            Ok(output) => (
                if output.status.success() {
                    RunStatus::Passed
                } else {
                    RunStatus::Failed
                },
                output.status.code(),
                output.stdout,
                output.stderr,
            ),
            Err(err) => (
                RunStatus::Failed,
                None,
                Vec::new(),
                format!("failed to execute command: {err}\n").into_bytes(),
            ),
        };

        write_run_log(
            &output_path,
            &reservation.command,
            status,
            exit_code,
            &stdout,
            &stderr,
        )?;
        self.finish_run(&reservation, status, finished_at, &input.actor)
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

    fn reserve_run(&self, input: &RunCommand) -> Result<RunReservation, RumbError> {
        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            if !item_exists(&tx, &input.item_id)? {
                return Err(RumbError::MissingItem(input.item_id.clone()));
            }

            let started_at = timestamp();
            let run_id = next_prefixed_id(&tx, "runs", "RUN-", 4)?;
            let proposal_id = latest_proposal_id_for_item(&tx, &input.item_id)?;
            let command = input.command.join(" ");
            let output_path = format!(".rumb/runs/{run_id}.log");
            tx.execute(
                r"
                INSERT INTO runs (
                    id, item_id, proposal_id, command, status, output_path, started_at, finished_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, NULL)
                ",
                params![
                    &run_id,
                    &input.item_id,
                    proposal_id.as_deref(),
                    &command,
                    RunStatus::Running.to_string(),
                    &output_path,
                    started_at as i64,
                ],
            )?;
            tx.commit()?;
            Ok(RunReservation {
                id: run_id,
                item_id: input.item_id.clone(),
                proposal_id,
                command,
                output_path,
                started_at,
            })
        })
    }

    fn finish_run(
        &self,
        reservation: &RunReservation,
        status: RunStatus,
        finished_at: u64,
        actor: &str,
    ) -> Result<RunRecord, RumbError> {
        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            tx.execute(
                "UPDATE runs SET status = ?, finished_at = ? WHERE id = ?",
                params![status.to_string(), finished_at as i64, &reservation.id],
            )?;
            append_event(
                &tx,
                EventInput {
                    timestamp: finished_at,
                    action: "run.record",
                    object_type: "item",
                    object_id: &reservation.item_id,
                    payload: json!({
                        "actor": actor,
                        "run_id": &reservation.id,
                        "status": status.to_string(),
                        "output_path": &reservation.output_path,
                        "command": &reservation.command,
                    })
                    .to_string(),
                },
            )?;
            tx.commit()?;
            Ok(RunRecord {
                id: reservation.id.clone(),
                item_id: reservation.item_id.clone(),
                proposal_id: reservation.proposal_id.clone(),
                command: reservation.command.clone(),
                status,
                output_path: reservation.output_path.clone(),
                started_at: reservation.started_at,
                finished_at,
            })
        })
    }

    fn reserve_claim(&self, input: &ClaimItem) -> Result<ClaimReservation, RumbError> {
        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            let item = load_item(&tx, &input.item_id)?
                .ok_or_else(|| RumbError::MissingItem(input.item_id.clone()))?;
            if item.id == ROOT_ID {
                return Err(RumbError::RootCannotClaim);
            }

            let items = load_items(&tx)?;
            let edges = load_edges(&tx)?;
            let depth = item_depth(&item.id, &items)?;
            if depth == 1 && !input.confirm_foundation {
                return Err(RumbError::FoundationRequiresConfirm);
            }
            if !dependencies_satisfied(&item, &edges, &items) {
                return Err(RumbError::UnsatisfiedDependencies(item.id));
            }

            let now = timestamp();
            if let Some(active_claim) = active_claim_for_item(&tx, &item.id, now)? {
                return Err(RumbError::ClaimAlreadyActive(active_claim.id));
            }
            if !matches!(item.status, Status::Ready | Status::Claimed) {
                return Err(RumbError::ItemNotReady(item.id));
            }

            let claim_id = next_prefixed_id(&tx, "claims", "CLAIM-", 4)?;
            let proposal_id = next_prefixed_id(&tx, "proposals", "PROP-", 4)?;
            let branch = format!("rumb/{}-{}", item.id, slugify(&item.title));
            let worktree_path = format!(".rumb/worktrees/{}-{}", item.id, slugify(&item.title));
            let lease_until = now + input.ttl_seconds;
            let claim = Claim {
                id: claim_id,
                item_id: item.id.clone(),
                actor_id: input.actor.clone(),
                lease_until,
                status: ClaimStatus::Pending,
                branch,
                worktree_path,
                created_at: now,
                updated_at: now,
            };
            insert_claim(&tx, &claim)?;
            insert_proposal(
                &tx,
                &proposal_id,
                &claim,
                "pending",
                now,
                &self.current_ref()?,
            )?;
            update_item_status_row(&tx, &item.id, Status::Claimed, now)?;
            append_event(
                &tx,
                EventInput {
                    timestamp: now,
                    action: "claim.reserve",
                    object_type: "item",
                    object_id: &item.id,
                    payload: json!({
                        "claim_id": &claim.id,
                        "actor": &input.actor,
                        "branch": &claim.branch,
                        "worktree_path": &claim.worktree_path,
                        "lease_until": claim.lease_until,
                    })
                    .to_string(),
                },
            )?;
            tx.commit()?;
            Ok(ClaimReservation { claim, proposal_id })
        })
    }

    fn activate_claim(&self, reservation: &ClaimReservation) -> Result<Claim, RumbError> {
        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            let mut claim = load_claim(&tx, &reservation.claim.id)?
                .ok_or_else(|| RumbError::MissingClaim(reservation.claim.id.clone()))?;
            let now = timestamp();
            claim.status = ClaimStatus::Active;
            claim.updated_at = now;
            update_claim_status(&tx, &claim)?;
            update_proposal_status(&tx, &reservation.proposal_id, "open", now)?;
            append_event(
                &tx,
                EventInput {
                    timestamp: now,
                    action: "claim.create",
                    object_type: "item",
                    object_id: &claim.item_id,
                    payload: json!({
                        "claim_id": &claim.id,
                        "actor": &claim.actor_id,
                        "branch": &claim.branch,
                        "worktree_path": &claim.worktree_path,
                        "lease_until": claim.lease_until,
                    })
                    .to_string(),
                },
            )?;
            tx.commit()?;
            Ok(claim)
        })
    }

    fn fail_claim(&self, reservation: &ClaimReservation, reason: &str) -> Result<(), RumbError> {
        with_write_retry(|| {
            let mut conn = self.open_database()?;
            let tx = conn.transaction()?;
            let mut claim = load_claim(&tx, &reservation.claim.id)?
                .ok_or_else(|| RumbError::MissingClaim(reservation.claim.id.clone()))?;
            let now = timestamp();
            claim.status = ClaimStatus::Failed;
            claim.updated_at = now;
            update_claim_status(&tx, &claim)?;
            update_proposal_status(&tx, &reservation.proposal_id, "failed", now)?;
            if item_status(&tx, &claim.item_id)? == Some(Status::Claimed) {
                update_item_status_row(&tx, &claim.item_id, Status::Ready, now)?;
            }
            append_event(
                &tx,
                EventInput {
                    timestamp: now,
                    action: "claim.failed",
                    object_type: "item",
                    object_id: &claim.item_id,
                    payload: json!({
                        "claim_id": &claim.id,
                        "actor": &claim.actor_id,
                        "reason": reason,
                    })
                    .to_string(),
                },
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    fn create_claim_worktree(&self, claim: &Claim) -> Result<(), RumbError> {
        let worktree = self.root.join(&claim.worktree_path);
        if let Some(parent) = worktree.parent() {
            fs::create_dir_all(parent)?;
        }

        let status = Command::new("git")
            .arg("worktree")
            .arg("add")
            .arg("-b")
            .arg(&claim.branch)
            .arg(&worktree)
            .current_dir(&self.root)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(RumbError::GitFailed(format!(
                "git worktree add -b {} {} exited with {status}",
                claim.branch,
                worktree.display()
            )))
        }
    }

    fn current_ref(&self) -> Result<String, RumbError> {
        let output = Command::new("git")
            .arg("rev-parse")
            .arg("--abbrev-ref")
            .arg("HEAD")
            .current_dir(&self.root)
            .output()?;
        if !output.status.success() {
            return Ok("HEAD".to_owned());
        }
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if branch.is_empty() || branch == "HEAD" {
            Ok("HEAD".to_owned())
        } else {
            Ok(branch)
        }
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

fn ensure_schema(conn: &mut Connection) -> Result<(), RumbError> {
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

fn applied_migrations(conn: &Connection) -> Result<HashSet<i32>, RumbError> {
    let mut versions = HashSet::new();
    let mut stmt = conn.prepare("SELECT version FROM migrations")?;
    let rows = stmt.query_map([], |row| row.get::<_, i32>(0))?;
    for row in rows {
        versions.insert(row?);
    }
    Ok(versions)
}

fn item_exists(conn: &Connection, id: &str) -> Result<bool, RumbError> {
    let count = conn.query_row(
        "SELECT COUNT(*) FROM items WHERE id = ?",
        params![id],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(count > 0)
}

fn insert_item(conn: &Connection, item: &Item) -> Result<(), RumbError> {
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

fn insert_edge(conn: &Connection, edge: &Edge) -> Result<(), RumbError> {
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

fn load_item(conn: &Connection, id: &str) -> Result<Option<Item>, RumbError> {
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

fn item_status(conn: &Connection, id: &str) -> Result<Option<Status>, RumbError> {
    Ok(load_item(conn, id)?.map(|item| item.status))
}

fn update_item_status_row(
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

fn insert_claim(conn: &Connection, claim: &Claim) -> Result<(), RumbError> {
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

fn insert_proposal(
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

fn load_claim(conn: &Connection, id: &str) -> Result<Option<Claim>, RumbError> {
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

fn load_claims_for_item(conn: &Connection, item_id: &str) -> Result<Vec<Claim>, RumbError> {
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

fn update_claim_status(conn: &Connection, claim: &Claim) -> Result<(), RumbError> {
    conn.execute(
        "UPDATE claims SET status = ?, updated_at = ? WHERE id = ?",
        params![claim.status.to_string(), claim.updated_at as i64, &claim.id],
    )?;
    Ok(())
}

fn update_claim_lease(conn: &Connection, claim: &Claim) -> Result<(), RumbError> {
    conn.execute(
        "UPDATE claims SET lease_until = ?, updated_at = ? WHERE id = ?",
        params![claim.lease_until as i64, claim.updated_at as i64, &claim.id],
    )?;
    Ok(())
}

fn update_proposal_status(
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

fn update_proposal_status_for_claim(
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

fn latest_proposal_id_for_item(
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

fn load_proposals_for_item(conn: &Connection, item_id: &str) -> Result<Vec<Proposal>, RumbError> {
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

fn load_runs_for_item(conn: &Connection, item_id: &str) -> Result<Vec<RunRecord>, RumbError> {
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

fn load_events_for_item(conn: &Connection, item_id: &str) -> Result<Vec<Event>, RumbError> {
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

fn active_claim_for_item(
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

fn has_other_active_claim(
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

fn active_claim_item_ids(conn: &Connection, now: u64) -> Result<HashSet<String>, RumbError> {
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

fn next_prefixed_id(
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

struct EventInput<'a> {
    timestamp: u64,
    action: &'a str,
    object_type: &'a str,
    object_id: &'a str,
    payload: String,
}

fn append_event(conn: &Connection, event: EventInput<'_>) -> Result<(), RumbError> {
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

fn next_item_id(conn: &Connection) -> Result<String, RumbError> {
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

fn load_items(conn: &Connection) -> Result<Vec<Item>, RumbError> {
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

fn load_edges(conn: &Connection) -> Result<Vec<Edge>, RumbError> {
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

fn item_from_db(item: DbItem) -> Result<Item, RumbError> {
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

fn edge_from_db(edge: DbEdge) -> Result<Edge, RumbError> {
    Ok(Edge {
        from: edge.from,
        to: edge.to,
        kind: edge.kind.parse()?,
        created_at: stored_timestamp(edge.created_at)?,
    })
}

fn map_claim_row(row: &duckdb::Row<'_>) -> duckdb::Result<DbClaim> {
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

fn claim_from_db(claim: DbClaim) -> Result<Claim, RumbError> {
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

fn proposal_from_db(proposal: DbProposal) -> Result<Proposal, RumbError> {
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

fn run_from_db(run: DbRun) -> Result<RunRecord, RumbError> {
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

fn map_event_row(row: &duckdb::Row<'_>) -> duckdb::Result<Event> {
    let timestamp: i64 = row.get(0)?;
    Ok(Event {
        timestamp: timestamp as u64,
        action: row.get(1)?,
        object_type: row.get(2)?,
        object_id: row.get(3)?,
        payload: row.get(4)?,
    })
}

fn stored_timestamp(value: i64) -> Result<u64, RumbError> {
    value
        .try_into()
        .map_err(|_| RumbError::InvalidState(format!("negative timestamp: {value}")))
}

fn dependencies_satisfied(item: &Item, edges: &[Edge], items: &[Item]) -> bool {
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

fn item_depth(item_id: &str, items: &[Item]) -> Result<usize, RumbError> {
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

fn normalize_item_id(reference: &str) -> Result<String, RumbError> {
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

fn slugify(value: &str) -> String {
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

fn write_run_log(
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

fn with_storage_retry<T>(
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

fn with_write_retry<T>(operation: impl FnMut() -> Result<T, RumbError>) -> Result<T, RumbError> {
    with_storage_retry(operation)
}

fn is_busy_error(err: &RumbError) -> bool {
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

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_git_repo(path: &Path) {
        std::process::Command::new("git")
            .arg("init")
            .current_dir(path)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .arg("-c")
            .arg("user.email=rumb@example.invalid")
            .arg("-c")
            .arg("user.name=Rumb Test")
            .arg("-c")
            .arg("commit.gpgsign=false")
            .arg("commit")
            .arg("--allow-empty")
            .arg("-m")
            .arg("init")
            .current_dir(path)
            .status()
            .unwrap();
    }

    fn init_project(path: &Path) -> RumbProject {
        let project = RumbProject::open(path);
        project
            .init(&InitOptions {
                name: "rumb".to_owned(),
            })
            .unwrap();
        project
    }

    fn create_ready_item(project: &RumbProject, parent_id: &str, title: &str) -> Item {
        project
            .create_item(CreateItem {
                kind: "feature".to_owned(),
                title: title.to_owned(),
                parent_id: parent_id.to_owned(),
                status: Status::Ready,
                source_ref: None,
            })
            .unwrap()
    }

    #[test]
    fn init_creates_duckdb_schema_migration_root_and_git_exclude() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());

        let project = init_project(dir.path());

        assert!(project.state_dir().is_dir());
        assert!(project.state_file().is_file());
        assert!(fs::read_to_string(dir.path().join(".git/info/exclude"))
            .unwrap()
            .lines()
            .any(|line| line.trim() == ".rumb/"));
        assert!(project.doctor().unwrap().ok());

        let conn = Connection::open(project.state_file()).unwrap();
        let migration_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM migrations WHERE version IN (1, 2)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migration_count, 2);

        let claim_table_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM claims", [], |row| row.get(0))
            .unwrap();
        let proposal_table_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM proposals", [], |row| row.get(0))
            .unwrap();
        let run_table_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(claim_table_count, 0);
        assert_eq!(proposal_table_count, 0);
        assert_eq!(run_table_count, 0);

        let root: (String, String, String, String) = conn
            .query_row(
                "SELECT id, kind, title, status FROM items WHERE id = 'RUMB-0000'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            root,
            (
                "RUMB-0000".to_owned(),
                "project".to_owned(),
                "rumb".to_owned(),
                "ready".to_owned()
            )
        );
    }

    #[test]
    fn id_allocation_is_sequential() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());

        let first = project
            .create_item(CreateItem {
                kind: "feature".to_owned(),
                title: "First".to_owned(),
                parent_id: ROOT_ID.to_owned(),
                status: Status::Ready,
                source_ref: None,
            })
            .unwrap();
        let second = project
            .create_item(CreateItem {
                kind: "feature".to_owned(),
                title: "Second".to_owned(),
                parent_id: ROOT_ID.to_owned(),
                status: Status::Draft,
                source_ref: None,
            })
            .unwrap();

        assert_eq!(first.id, "RUMB-0001");
        assert_eq!(second.id, "RUMB-0002");
    }

    #[test]
    fn dependency_readiness_honors_depends_on_and_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());

        let done_dependency = project
            .create_item(CreateItem {
                kind: "task".to_owned(),
                title: "Done dependency".to_owned(),
                parent_id: ROOT_ID.to_owned(),
                status: Status::Done,
                source_ref: None,
            })
            .unwrap();
        let satisfied = project
            .create_item(CreateItem {
                kind: "feature".to_owned(),
                title: "Satisfied".to_owned(),
                parent_id: ROOT_ID.to_owned(),
                status: Status::Ready,
                source_ref: None,
            })
            .unwrap();
        let unsatisfied = project
            .create_item(CreateItem {
                kind: "feature".to_owned(),
                title: "Unsatisfied".to_owned(),
                parent_id: ROOT_ID.to_owned(),
                status: Status::Ready,
                source_ref: None,
            })
            .unwrap();
        let blocker = project
            .create_item(CreateItem {
                kind: "feature".to_owned(),
                title: "Blocker".to_owned(),
                parent_id: ROOT_ID.to_owned(),
                status: Status::Ready,
                source_ref: None,
            })
            .unwrap();
        let blocked = project
            .create_item(CreateItem {
                kind: "feature".to_owned(),
                title: "Blocked".to_owned(),
                parent_id: ROOT_ID.to_owned(),
                status: Status::Ready,
                source_ref: None,
            })
            .unwrap();

        project
            .add_edge(AddEdge {
                from: satisfied.id.clone(),
                to: done_dependency.id.clone(),
                kind: EdgeKind::DependsOn,
            })
            .unwrap();
        project
            .add_edge(AddEdge {
                from: unsatisfied.id.clone(),
                to: satisfied.id.clone(),
                kind: EdgeKind::DependsOn,
            })
            .unwrap();
        project
            .add_edge(AddEdge {
                from: blocker.id.clone(),
                to: blocked.id.clone(),
                kind: EdgeKind::Blocks,
            })
            .unwrap();

        let ready_ids = project
            .ready_items()
            .unwrap()
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();
        assert_eq!(ready_ids, vec![satisfied.id, blocker.id]);
    }

    #[test]
    fn root_cannot_be_claimed_and_foundation_claims_require_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let project = init_project(dir.path());
        let foundation = create_ready_item(&project, ROOT_ID, "Foundation Work");

        assert!(matches!(
            project
                .claim_item(ClaimItem {
                    item_id: ROOT_ID.to_owned(),
                    actor: "operator".to_owned(),
                    ttl_seconds: DEFAULT_TTL_SECONDS,
                    confirm_foundation: true,
                })
                .unwrap_err(),
            RumbError::RootCannotClaim
        ));
        assert!(matches!(
            project
                .claim_item(ClaimItem {
                    item_id: foundation.id.clone(),
                    actor: "operator".to_owned(),
                    ttl_seconds: DEFAULT_TTL_SECONDS,
                    confirm_foundation: false,
                })
                .unwrap_err(),
            RumbError::FoundationRequiresConfirm
        ));
    }

    #[test]
    fn depth_two_claim_does_not_require_foundation_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let project = init_project(dir.path());
        let foundation = create_ready_item(&project, ROOT_ID, "Foundation Work");
        let child = create_ready_item(&project, &foundation.id, "Child Work");

        let claim = project
            .claim_item(ClaimItem {
                item_id: child.id.clone(),
                actor: "operator".to_owned(),
                ttl_seconds: DEFAULT_TTL_SECONDS,
                confirm_foundation: false,
            })
            .unwrap();

        assert_eq!(claim.id, "CLAIM-0001");
        assert_eq!(claim.status, ClaimStatus::Active);
        assert_eq!(claim.branch, "rumb/RUMB-0002-child-work");
        assert!(dir.path().join(&claim.worktree_path).is_dir());
    }

    #[test]
    fn claim_creates_claim_proposal_branch_and_worktree_rows() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let project = init_project(dir.path());
        let foundation = create_ready_item(&project, ROOT_ID, "Foundation Work");
        let child = create_ready_item(&project, &foundation.id, "Child Work");

        let claim = project
            .claim_item(ClaimItem {
                item_id: child.id.clone(),
                actor: "operator".to_owned(),
                ttl_seconds: DEFAULT_TTL_SECONDS,
                confirm_foundation: false,
            })
            .unwrap();

        let branch_status = std::process::Command::new("git")
            .arg("rev-parse")
            .arg("--verify")
            .arg(&claim.branch)
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(branch_status.success());
        assert!(dir.path().join(&claim.worktree_path).is_dir());

        let conn = Connection::open(project.state_file()).unwrap();
        let claim_row: (String, String, String, String) = conn
            .query_row(
                "SELECT id, item_id, status, branch FROM claims WHERE id = ?",
                params![&claim.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            claim_row,
            (
                claim.id.clone(),
                child.id.clone(),
                "active".to_owned(),
                claim.branch.clone()
            )
        );

        let proposal_row: (String, String, String) = conn
            .query_row(
                "SELECT item_id, branch, status FROM proposals WHERE item_id = ? AND branch = ?",
                params![&child.id, &claim.branch],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            proposal_row,
            (child.id.clone(), claim.branch, "open".to_owned())
        );
    }

    #[test]
    fn active_unexpired_claim_blocks_readiness_and_second_claim() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let project = init_project(dir.path());
        let foundation = create_ready_item(&project, ROOT_ID, "Foundation Work");
        let child = create_ready_item(&project, &foundation.id, "Child Work");

        let claim = project
            .claim_item(ClaimItem {
                item_id: child.id.clone(),
                actor: "operator".to_owned(),
                ttl_seconds: DEFAULT_TTL_SECONDS,
                confirm_foundation: false,
            })
            .unwrap();

        assert!(project
            .ready_items()
            .unwrap()
            .into_iter()
            .all(|item| item.id != child.id));

        assert!(matches!(
            project
                .claim_item(ClaimItem {
                    item_id: child.id.clone(),
                    actor: "other".to_owned(),
                    ttl_seconds: DEFAULT_TTL_SECONDS,
                    confirm_foundation: false,
                })
                .unwrap_err(),
            RumbError::ClaimAlreadyActive(active_id) if active_id == claim.id
        ));
    }

    #[test]
    fn expired_claim_no_longer_blocks_readiness() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let project = init_project(dir.path());
        let foundation = create_ready_item(&project, ROOT_ID, "Foundation Work");
        let child = create_ready_item(&project, &foundation.id, "Child Work");

        let claim = project
            .claim_item(ClaimItem {
                item_id: child.id.clone(),
                actor: "operator".to_owned(),
                ttl_seconds: DEFAULT_TTL_SECONDS,
                confirm_foundation: false,
            })
            .unwrap();

        let conn = Connection::open(project.state_file()).unwrap();
        conn.execute(
            "UPDATE claims SET lease_until = ? WHERE id = ?",
            params![(timestamp() - 1) as i64, &claim.id],
        )
        .unwrap();
        drop(conn);

        assert!(project
            .ready_items()
            .unwrap()
            .into_iter()
            .any(|item| item.id == child.id));
    }

    #[test]
    fn renew_extends_lease_and_logs_event() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let project = init_project(dir.path());
        let foundation = create_ready_item(&project, ROOT_ID, "Foundation Work");
        let child = create_ready_item(&project, &foundation.id, "Child Work");

        let claim = project
            .claim_item(ClaimItem {
                item_id: child.id.clone(),
                actor: "operator".to_owned(),
                ttl_seconds: 60,
                confirm_foundation: false,
            })
            .unwrap();

        let renewed = project
            .renew_claim(RenewClaim {
                claim_id: claim.id.clone(),
                actor: "operator".to_owned(),
                ttl_seconds: DEFAULT_TTL_SECONDS * 2,
            })
            .unwrap();
        assert!(renewed.lease_until >= claim.lease_until);
        assert_eq!(
            project
                .events(Some(&child.id))
                .unwrap()
                .last()
                .unwrap()
                .action,
            "claim.renew"
        );
    }

    #[test]
    fn release_restores_ready_and_logs_event() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let project = init_project(dir.path());
        let foundation = create_ready_item(&project, ROOT_ID, "Foundation Work");
        let child = create_ready_item(&project, &foundation.id, "Child Work");

        let claim = project
            .claim_item(ClaimItem {
                item_id: child.id.clone(),
                actor: "operator".to_owned(),
                ttl_seconds: DEFAULT_TTL_SECONDS,
                confirm_foundation: false,
            })
            .unwrap();

        let released = project
            .release_claim(ReleaseClaim {
                claim_id: claim.id.clone(),
                actor: "operator".to_owned(),
            })
            .unwrap();
        assert_eq!(released.status, ClaimStatus::Released);
        assert!(project
            .ready_items()
            .unwrap()
            .into_iter()
            .any(|item| item.id == child.id));

        assert_eq!(
            project
                .events(Some(&child.id))
                .unwrap()
                .last()
                .unwrap()
                .action,
            "claim.release"
        );
    }

    #[test]
    fn successful_run_records_passed_status_log_path_and_captures_output() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());
        let item = create_ready_item(&project, ROOT_ID, "Run success");

        let run = project
            .run_command(RunCommand {
                item_id: item.id.clone(),
                actor: "operator".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "printf stdout-text; printf stderr-text >&2".to_owned(),
                ],
            })
            .unwrap();

        assert_eq!(run.id, "RUN-0001");
        assert_eq!(run.item_id, item.id);
        assert_eq!(run.status, RunStatus::Passed);
        assert_eq!(run.output_path, ".rumb/runs/RUN-0001.log");
        let log = fs::read_to_string(dir.path().join(&run.output_path)).unwrap();
        assert!(log.starts_with(
            "command\t/bin/sh -c printf stdout-text; printf stderr-text >&2\nstatus\tpassed\nexit_code\t0\n\n[stdout]\nstdout-text\n\n[stderr]\nstderr-text\n"
        ));
        assert!(log.contains("stdout-text"));
        assert!(log.contains("stderr-text"));
        assert!(log.contains("status\tpassed"));

        let events = project.events(Some(&run.item_id)).unwrap();
        assert_eq!(events.last().unwrap().action, "run.record");
        assert!(events
            .last()
            .unwrap()
            .payload
            .contains("\"status\":\"passed\""));
    }

    #[test]
    fn failing_run_records_failed_status_and_sequential_id() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());
        let item = create_ready_item(&project, ROOT_ID, "Run fail");

        let first = project
            .run_command(RunCommand {
                item_id: item.id.clone(),
                actor: "operator".to_owned(),
                command: vec!["/bin/sh".to_owned(), "-c".to_owned(), "true".to_owned()],
            })
            .unwrap();
        let second = project
            .run_command(RunCommand {
                item_id: item.id.clone(),
                actor: "operator".to_owned(),
                command: vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    "echo failing >&2; exit 7".to_owned(),
                ],
            })
            .unwrap();

        assert_eq!(first.id, "RUN-0001");
        assert_eq!(second.id, "RUN-0002");
        assert_eq!(second.status, RunStatus::Failed);
        let log = fs::read_to_string(dir.path().join(&second.output_path)).unwrap();
        assert!(log.contains("exit_code\t7"));
        assert!(log.contains("failing"));
        assert!(log.contains("status\tfailed"));
    }

    #[test]
    fn failed_spawn_records_failed_run_and_stderr_message() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());
        let item = create_ready_item(&project, ROOT_ID, "Spawn fail");

        let run = project
            .run_command(RunCommand {
                item_id: item.id.clone(),
                actor: "operator".to_owned(),
                command: vec!["/definitely/not/rumb-command".to_owned()],
            })
            .unwrap();

        assert_eq!(run.id, "RUN-0001");
        assert_eq!(run.status, RunStatus::Failed);
        let log = fs::read_to_string(dir.path().join(&run.output_path)).unwrap();
        assert!(log.contains("command\t/definitely/not/rumb-command"));
        assert!(log.contains("status\tfailed"));
        assert!(log.contains("exit_code\tunknown"));
        assert!(log.contains("[stderr]\nfailed to execute command:"));

        let conn = Connection::open(project.state_file()).unwrap();
        let stored_status: String = conn
            .query_row("SELECT status FROM runs WHERE id = 'RUN-0001'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(stored_status, "failed");
    }

    #[test]
    fn run_ids_stay_sequential_after_spawn_failures() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());
        let item = create_ready_item(&project, ROOT_ID, "Run sequence");

        let first = project
            .run_command(RunCommand {
                item_id: item.id.clone(),
                actor: "operator".to_owned(),
                command: vec!["/definitely/not/rumb-command".to_owned()],
            })
            .unwrap();
        let second = project
            .run_command(RunCommand {
                item_id: item.id.clone(),
                actor: "operator".to_owned(),
                command: vec!["/bin/sh".to_owned(), "-c".to_owned(), "true".to_owned()],
            })
            .unwrap();

        assert_eq!(first.id, "RUN-0001");
        assert_eq!(first.status, RunStatus::Failed);
        assert_eq!(second.id, "RUN-0002");
        assert_eq!(second.status, RunStatus::Passed);
    }

    #[test]
    fn review_and_done_transition_items_and_log_actor_events() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());
        let item = create_ready_item(&project, ROOT_ID, "Review done");

        let reviewed = project
            .review_item(ReviewItem {
                item_id: item.id.clone(),
                actor: "operator".to_owned(),
            })
            .unwrap();
        assert_eq!(reviewed.status, Status::InReview);

        let done = project
            .done_item(DoneItem {
                item_id: item.id.clone(),
                actor: "operator".to_owned(),
            })
            .unwrap();
        assert_eq!(done.status, Status::Done);

        let events = project.events(Some(&item.id)).unwrap();
        let actions = events
            .iter()
            .map(|event| event.action.as_str())
            .collect::<Vec<_>>();
        assert!(actions.contains(&"item.review"));
        assert!(actions.contains(&"item.done"));
        assert!(events.iter().any(|event| event.action == "item.review"
            && event.payload.contains("\"actor\":\"operator\"")));
        assert!(events
            .iter()
            .any(|event| event.action == "item.done"
                && event.payload.contains("\"actor\":\"operator\"")));
    }

    #[test]
    fn done_unlocks_dependent_ready_items() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());
        let dependency = create_ready_item(&project, ROOT_ID, "Dependency");
        let dependent = create_ready_item(&project, ROOT_ID, "Dependent");
        project
            .add_edge(AddEdge {
                from: dependent.id.clone(),
                to: dependency.id.clone(),
                kind: EdgeKind::DependsOn,
            })
            .unwrap();

        let ready_before = project
            .ready_items()
            .unwrap()
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();
        assert_eq!(ready_before, vec![dependency.id.clone()]);

        project
            .done_item(DoneItem {
                item_id: dependency.id.clone(),
                actor: "operator".to_owned(),
            })
            .unwrap();

        let ready_after = project
            .ready_items()
            .unwrap()
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>();
        assert_eq!(ready_after, vec![dependent.id]);
    }

    #[test]
    fn run_review_and_done_missing_items_error_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());

        assert!(matches!(
            project
                .run_command(RunCommand {
                    item_id: "RUMB-4040".to_owned(),
                    actor: "operator".to_owned(),
                    command: vec!["/bin/sh".to_owned(), "-c".to_owned(), "true".to_owned()],
                })
                .unwrap_err(),
            RumbError::MissingItem(id) if id == "RUMB-4040"
        ));
        assert!(matches!(
            project
                .review_item(ReviewItem {
                    item_id: "RUMB-4040".to_owned(),
                    actor: "operator".to_owned(),
                })
                .unwrap_err(),
            RumbError::MissingItem(id) if id == "RUMB-4040"
        ));
        assert!(matches!(
            project
                .done_item(DoneItem {
                    item_id: "RUMB-4040".to_owned(),
                    actor: "operator".to_owned(),
                })
                .unwrap_err(),
            RumbError::MissingItem(id) if id == "RUMB-4040"
        ));
    }

    #[test]
    fn failed_git_worktree_creation_restores_item_state_and_logs_failure() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let project = init_project(dir.path());
        let foundation = create_ready_item(&project, ROOT_ID, "Foundation Work");
        let child = create_ready_item(&project, &foundation.id, "Duplicate Branch");

        let branch = format!("rumb/{}-duplicate-branch", child.id);
        let branch_status = std::process::Command::new("git")
            .arg("branch")
            .arg(&branch)
            .current_dir(dir.path())
            .status()
            .unwrap();
        assert!(branch_status.success());

        assert!(matches!(
            project
                .claim_item(ClaimItem {
                    item_id: child.id.clone(),
                    actor: "operator".to_owned(),
                    ttl_seconds: DEFAULT_TTL_SECONDS,
                    confirm_foundation: false,
                })
                .unwrap_err(),
            RumbError::GitFailed(_)
        ));

        let conn = Connection::open(project.state_file()).unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM items WHERE id = ?",
                params![&child.id],
                |row| row.get(0),
            )
            .unwrap();
        let claim_status: String = conn
            .query_row(
                "SELECT status FROM claims WHERE item_id = ?",
                params![&child.id],
                |row| row.get(0),
            )
            .unwrap();
        let proposal_status: String = conn
            .query_row(
                "SELECT status FROM proposals WHERE item_id = ?",
                params![&child.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "ready");
        assert_eq!(claim_status, "failed");
        assert_eq!(proposal_status, "failed");
        drop(conn);

        let actions = project
            .events(Some(&child.id))
            .unwrap()
            .into_iter()
            .map(|event| event.action)
            .collect::<Vec<_>>();
        assert_eq!(
            actions,
            vec!["item.create", "claim.reserve", "claim.failed"]
        );
        assert!(project
            .ready_items()
            .unwrap()
            .into_iter()
            .any(|item| item.id == child.id));
    }

    #[test]
    fn item_status_updates_transactionally_and_logs_actor() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());
        let item = project
            .create_item(CreateItem {
                kind: "feature".to_owned(),
                title: "Status".to_owned(),
                parent_id: ROOT_ID.to_owned(),
                status: Status::Ready,
                source_ref: None,
            })
            .unwrap();

        let updated = project
            .update_item_status(UpdateItemStatus {
                item_id: item.id.clone(),
                status: Status::Done,
                actor: "operator".to_owned(),
            })
            .unwrap();

        assert_eq!(updated.status, Status::Done);
        let events = project.events(Some(&item.id)).unwrap();
        assert_eq!(events.last().unwrap().action, "item.status");
        assert!(events
            .last()
            .unwrap()
            .payload
            .contains("\"actor\":\"operator\""));
    }

    #[test]
    fn list_items_and_item_details_include_related_state() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let project = init_project(dir.path());
        let parent = create_ready_item(&project, ROOT_ID, "Parent");
        let child = create_ready_item(&project, &parent.id, "Child");
        let grandchild = create_ready_item(&project, &child.id, "Grandchild");
        project
            .add_edge(AddEdge {
                from: child.id.clone(),
                to: parent.id.clone(),
                kind: EdgeKind::RelatesTo,
            })
            .unwrap();
        project
            .add_edge(AddEdge {
                from: grandchild.id.clone(),
                to: child.id.clone(),
                kind: EdgeKind::RelatesTo,
            })
            .unwrap();
        let claim = project
            .claim_item(ClaimItem {
                item_id: child.id.clone(),
                actor: "operator".to_owned(),
                ttl_seconds: DEFAULT_TTL_SECONDS,
                confirm_foundation: false,
            })
            .unwrap();
        let run = project
            .run_command(RunCommand {
                item_id: child.id.clone(),
                actor: "operator".to_owned(),
                command: vec!["/bin/sh".to_owned(), "-c".to_owned(), "true".to_owned()],
            })
            .unwrap();

        let listed = project.list_items().unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "RUMB-0000",
                parent.id.as_str(),
                child.id.as_str(),
                grandchild.id.as_str()
            ]
        );

        for reference in ["2", "0002", child.id.as_str()] {
            let details = project.item_details(reference).unwrap();
            assert_eq!(details.item.id, child.id);
            assert_eq!(details.depth, 2);
            assert_eq!(details.children[0].id, grandchild.id);
            assert_eq!(details.incoming_edges[0].from, grandchild.id);
            assert_eq!(details.outgoing_edges[0].to, parent.id);
            assert_eq!(details.claims[0].id, claim.id);
            assert_eq!(details.proposals[0].item_id, child.id);
            assert_eq!(details.runs[0].id, run.id);
            assert!(details
                .events
                .iter()
                .any(|event| event.action == "claim.create"));
        }
    }

    #[test]
    fn item_details_accepts_rumb_0007_reference_forms_and_rejects_invalid_refs() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());
        for index in 1..=7 {
            create_ready_item(&project, ROOT_ID, &format!("Item {index}"));
        }

        for reference in ["7", "0007", "RUMB-0007"] {
            assert_eq!(
                project.item_details(reference).unwrap().item.id,
                "RUMB-0007"
            );
        }

        for reference in ["", "abc", "RUMB-abc", "7x"] {
            assert!(matches!(
                project.item_details(reference).unwrap_err(),
                RumbError::InvalidItemRef(_)
            ));
        }
    }

    #[test]
    fn events_are_created_for_mutations_and_can_be_filtered() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());
        let item = project
            .create_item(CreateItem {
                kind: "feature".to_owned(),
                title: "Claim flow".to_owned(),
                parent_id: ROOT_ID.to_owned(),
                status: Status::Ready,
                source_ref: Some("README".to_owned()),
            })
            .unwrap();
        project
            .add_edge(AddEdge {
                from: item.id.clone(),
                to: ROOT_ID.to_owned(),
                kind: EdgeKind::RelatesTo,
            })
            .unwrap();

        let actions = project
            .events(None)
            .unwrap()
            .into_iter()
            .map(|event| event.action)
            .collect::<Vec<_>>();
        assert_eq!(actions, vec!["init", "item.create", "edge.add"]);

        let item_events = project.events(Some(&item.id)).unwrap();
        assert_eq!(item_events.len(), 1);
        assert_eq!(item_events[0].action, "item.create");
        assert!(item_events[0].payload.contains("\"kind\":\"feature\""));
    }

    #[test]
    fn doctor_fails_when_rumb_is_not_ignored() {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let project = init_project(dir.path());

        fs::write(dir.path().join(".git/info/exclude"), "").unwrap();

        let report = project.doctor().unwrap();
        assert!(report.state_dir_exists);
        assert!(report.state_file_exists);
        assert!(!report.rumb_ignored_by_git);
        assert!(!report.ok());
    }

    #[test]
    fn doctor_fails_without_git_ignore_context() {
        let dir = tempfile::tempdir().unwrap();
        let project = init_project(dir.path());

        let report = project.doctor().unwrap();
        assert!(report.state_dir_exists);
        assert!(report.state_file_exists);
        assert!(!report.rumb_ignored_by_git);
        assert!(!report.ok());
    }

    #[test]
    fn existing_text_bootstrap_file_returns_storage_error() {
        let dir = tempfile::tempdir().unwrap();
        let project = RumbProject::open(dir.path());
        fs::create_dir_all(project.state_dir()).unwrap();
        fs::write(project.state_file(), "item\tRUMB-0000\n").unwrap();

        let err = project
            .init(&InitOptions {
                name: "rumb".to_owned(),
            })
            .unwrap_err();
        assert!(matches!(err, RumbError::Storage(_)));
    }
}
