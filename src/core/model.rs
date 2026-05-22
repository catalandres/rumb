use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::str::FromStr;

use clap::ValueEnum;
use thiserror::Error;

pub(crate) const STATE_DIR: &str = ".rumb";
pub(crate) const STATE_FILE: &str = "state.duckdb";
pub(crate) const ROOT_ID: &str = "RUMB-0000";
pub(crate) const META_INBOX_ID: &str = "inbox_id";
pub(crate) const CURRENT_SCHEMA_VERSION: i32 = 6;
pub(crate) const DEFAULT_TTL_SECONDS: u64 = 4 * 60 * 60;
pub(crate) const STORAGE_RETRY_ATTEMPTS: usize = 5;

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
    #[error("invalid tier: {0}")]
    InvalidTier(String),
    #[error("invalid ttl: {0}")]
    InvalidTtl(String),
    #[error("invalid item reference: {0}")]
    InvalidItemRef(String),
    #[error("item does not exist: {0}")]
    MissingItem(String),
    #[error("edge does not exist: {0}")]
    MissingEdge(String),
    #[error("inbox node is not initialized")]
    MissingInbox,
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
    #[error("reserved node cannot be groomed: {0}")]
    ReservedNode(String),
    #[error("item has an active claim and cannot be groomed: {0}")]
    GroomingBlockedByClaim(String),
    #[error("no changes provided")]
    NoGroomingChanges,
    #[error("cannot merge an item into itself: {0}")]
    CannotMergeIntoSelf(String),
    #[error("git command failed: {0}")]
    GitFailed(String),
    #[error("mcp install error: {0}")]
    McpInstall(String),
    #[error("doctor checks failed")]
    DoctorFailed,
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
    pub tier: Tier,
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
pub struct Reparent {
    pub item_id: String,
    pub new_parent_id: String,
    pub actor: String,
    pub confirm: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EditItem {
    pub item_id: String,
    pub title: Option<String>,
    pub source_ref: Option<String>,
    pub tier: Option<Tier>,
    pub actor: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capture {
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Recast {
    pub item_id: String,
    pub kind: String,
    pub actor: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Unlink {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub actor: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Merge {
    pub from_id: String,
    pub into_id: String,
    pub actor: String,
}

/// Result of `unlink`: the removed edge plus any items that became ready as a
/// direct consequence of removing it (so callers can surface, not silently drop,
/// a freshly-unblocked item).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnlinkOutcome {
    pub edge: Edge,
    pub newly_ready: Vec<Item>,
}

/// Result of `merge`: the now-superseded source item, the destination it merged
/// into, the children that were reparented, and the `supersedes` edge recorded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeOutcome {
    pub from: Item,
    pub into: Item,
    pub moved_children: Vec<String>,
    pub supersedes_edge: Edge,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Item {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub status: Status,
    pub tier: Tier,
    pub source_ref: Option<String>,
    pub body: Option<String>,
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

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum Status {
    #[default]
    Draft,
    Ready,
    Blocked,
    InReview,
    Done,
    Superseded,
    Abandoned,
}

impl Display for Status {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Blocked => "blocked",
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
            "in_review" => Ok(Self::InReview),
            "done" => Ok(Self::Done),
            "superseded" => Ok(Self::Superseded),
            "abandoned" => Ok(Self::Abandoned),
            _ => Err(RumbError::InvalidStatus(value.to_owned())),
        }
    }
}

/// The work-weight of an item: a property of the work itself, not a model name.
/// Displayed (not filtered) in `ready`/`view`; tier-based dispatch is deferred.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum Tier {
    Routine,
    #[default]
    Standard,
    Hard,
}

impl Display for Tier {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Routine => "routine",
            Self::Standard => "standard",
            Self::Hard => "hard",
        })
    }
}

impl FromStr for Tier {
    type Err = RumbError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "routine" => Ok(Self::Routine),
            "standard" => Ok(Self::Standard),
            "hard" => Ok(Self::Hard),
            _ => Err(RumbError::InvalidTier(value.to_owned())),
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
