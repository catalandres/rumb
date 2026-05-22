use std::path::PathBuf;
use std::str::FromStr;

use clap::Parser;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::CallToolResult,
    schemars, serve_server, tool, tool_handler, tool_router, transport, ErrorData, ServerHandler,
};
use serde::Deserialize;
use serde_json::{json, Value};

use rumb::{
    default_ttl_seconds, parse_ttl, AddEdge, Claim, ClaimItem, CreateItem, DoneItem, Edge,
    EdgeKind, EditItem, Event, InitOptions, Item, Merge, MergeOutcome, Recast, ReleaseClaim,
    RenewClaim, Reparent, ReviewItem, RumbError, RumbProject, RunCommand, RunRecord, Status,
    Unlink, UnlinkOutcome, UpdateItemStatus,
};

#[derive(Debug, Parser)]
#[command(name = "rumb-mcp", version, about = "MCP server for rumb")]
struct Cli {}

#[derive(Debug, Clone)]
struct RumbMcp {
    root: PathBuf,
    tool_router: ToolRouter<Self>,
}

impl RumbMcp {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            tool_router: Self::tool_router(),
        }
    }

    fn project(&self) -> Result<RumbProject, ErrorData> {
        RumbProject::discover(&self.root).map_err(to_mcp_error)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RumbMcp {}

#[tool_router(router = tool_router)]
impl RumbMcp {
    #[tool(description = "Initialize rumb state in the current repository")]
    async fn init(
        &self,
        Parameters(args): Parameters<InitArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = RumbProject::open(&self.root);
        project
            .init(&InitOptions {
                name: args.name.clone(),
            })
            .map_err(to_mcp_error)?;
        Ok(structured(json!({
            "initialized": true,
            "name": args.name,
            "root": project.root().display().to_string(),
            "state_path": project.state_file().display().to_string(),
        })))
    }

    #[tool(description = "Run rumb doctor checks")]
    async fn doctor(&self) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let report = project.doctor().map_err(to_mcp_error)?;
        Ok(structured(json!({
            "state_dir_exists": report.state_dir_exists,
            "state_file_exists": report.state_file_exists,
            "rumb_ignored_by_git": report.rumb_ignored_by_git,
            "ok": report.ok(),
        })))
    }

    #[tool(description = "Create a rumb item")]
    async fn item_create(
        &self,
        Parameters(args): Parameters<ItemCreateArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let status = parse_status(args.status.as_deref().unwrap_or("draft"))?;
        let project = self.project()?;
        let item = project
            .create_item(CreateItem {
                kind: args.kind,
                title: args.title,
                parent_id: args.parent,
                status,
                source_ref: args.source,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(item_json(&item)))
    }

    #[tool(description = "Update a rumb item status")]
    async fn item_status(
        &self,
        Parameters(args): Parameters<ItemStatusArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let status = parse_status(&args.status)?;
        let project = self.project()?;
        let item = project
            .update_item_status(UpdateItemStatus {
                item_id: args.id,
                status,
                actor: args.actor,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(item_json(&item)))
    }

    #[tool(description = "Add a rumb graph edge")]
    async fn edge_add(
        &self,
        Parameters(args): Parameters<EdgeAddArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let kind = parse_edge_kind(&args.kind)?;
        let project = self.project()?;
        let edge = project
            .add_edge(AddEdge {
                from: args.from,
                to: args.to,
                kind,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(edge_json(&edge)))
    }

    #[tool(description = "List currently ready rumb items")]
    async fn ready(&self) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let items = project.ready_items().map_err(to_mcp_error)?;
        Ok(structured(json!({
            "items": items.iter().map(item_json).collect::<Vec<_>>(),
        })))
    }

    #[tool(description = "Claim a ready rumb item and create its git worktree")]
    async fn claim(
        &self,
        Parameters(args): Parameters<ClaimArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let claim = project
            .claim_item(ClaimItem {
                item_id: args.id,
                actor: args.actor,
                ttl_seconds: ttl_seconds(args.ttl.as_deref())?,
                confirm_foundation: args.confirm_foundation.unwrap_or(false),
            })
            .map_err(to_mcp_error)?;
        Ok(structured(claim_json(&claim)))
    }

    #[tool(description = "Renew an active rumb claim lease")]
    async fn renew(
        &self,
        Parameters(args): Parameters<RenewArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let claim = project
            .renew_claim(RenewClaim {
                claim_id: args.claim_id,
                actor: args.actor,
                ttl_seconds: ttl_seconds(args.ttl.as_deref())?,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(claim_json(&claim)))
    }

    #[tool(description = "Release an active rumb claim")]
    async fn release(
        &self,
        Parameters(args): Parameters<ReleaseArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let claim = project
            .release_claim(ReleaseClaim {
                claim_id: args.claim_id,
                actor: args.actor,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(claim_json(&claim)))
    }

    #[tool(description = "Run a local command and record its result")]
    async fn run(
        &self,
        Parameters(args): Parameters<RunArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let run = project
            .run_command(RunCommand {
                item_id: args.id,
                actor: args.actor,
                command: args.command,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(run_json(&run)))
    }

    #[tool(description = "Move a rumb item into review")]
    async fn review(
        &self,
        Parameters(args): Parameters<ReviewArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let item = project
            .review_item(ReviewItem {
                item_id: args.id,
                actor: args.actor,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(item_json(&item)))
    }

    #[tool(description = "Mark a rumb item done")]
    async fn done(
        &self,
        Parameters(args): Parameters<DoneArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let item = project
            .done_item(DoneItem {
                item_id: args.id,
                actor: args.actor,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(item_json(&item)))
    }

    #[tool(description = "Move a rumb item under a new parent")]
    async fn reparent(
        &self,
        Parameters(args): Parameters<ReparentArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let item = project
            .reparent(Reparent {
                item_id: args.id,
                new_parent_id: args.under,
                actor: args.actor,
                confirm: args.confirm.unwrap_or(false),
            })
            .map_err(to_mcp_error)?;
        Ok(structured(item_json(&item)))
    }

    #[tool(description = "Edit a rumb item's title and/or source reference")]
    async fn edit(
        &self,
        Parameters(args): Parameters<EditArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let item = project
            .edit(EditItem {
                item_id: args.id,
                title: args.title,
                source_ref: args.source,
                actor: args.actor,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(item_json(&item)))
    }

    #[tool(description = "Change a rumb item's kind")]
    async fn recast(
        &self,
        Parameters(args): Parameters<RecastArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let item = project
            .recast(Recast {
                item_id: args.id,
                kind: args.kind,
                actor: args.actor,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(item_json(&item)))
    }

    #[tool(description = "Remove a rumb graph edge, surfacing any newly ready items")]
    async fn unlink(
        &self,
        Parameters(args): Parameters<UnlinkArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let kind = parse_edge_kind(&args.kind)?;
        let project = self.project()?;
        let outcome = project
            .unlink(Unlink {
                from: args.from,
                to: args.to,
                kind,
                actor: args.actor,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(unlink_json(&outcome)))
    }

    #[tool(description = "Merge one rumb item into another, superseding the source")]
    async fn merge(
        &self,
        Parameters(args): Parameters<MergeArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let outcome = project
            .merge(Merge {
                from_id: args.from,
                into_id: args.into,
                actor: args.actor,
            })
            .map_err(to_mcp_error)?;
        Ok(structured(merge_json(&outcome)))
    }

    #[tool(description = "List rumb events, optionally scoped to one item")]
    async fn log(
        &self,
        Parameters(args): Parameters<LogArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let project = self.project()?;
        let events = project.events(args.id.as_deref()).map_err(to_mcp_error)?;
        Ok(structured(json!({
            "events": events.iter().map(event_json).collect::<Vec<_>>(),
        })))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InitArgs {
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ItemCreateArgs {
    kind: String,
    title: String,
    parent: String,
    status: Option<String>,
    source: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ItemStatusArgs {
    id: String,
    status: String,
    actor: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EdgeAddArgs {
    from: String,
    to: String,
    kind: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ClaimArgs {
    id: String,
    actor: String,
    ttl: Option<String>,
    confirm_foundation: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RenewArgs {
    claim_id: String,
    actor: String,
    ttl: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReleaseArgs {
    claim_id: String,
    actor: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RunArgs {
    id: String,
    actor: String,
    command: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReviewArgs {
    id: String,
    actor: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DoneArgs {
    id: String,
    actor: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LogArgs {
    id: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReparentArgs {
    id: String,
    under: String,
    actor: String,
    confirm: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EditArgs {
    id: String,
    title: Option<String>,
    source: Option<String>,
    actor: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RecastArgs {
    id: String,
    kind: String,
    actor: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UnlinkArgs {
    from: String,
    to: String,
    kind: String,
    actor: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MergeArgs {
    from: String,
    into: String,
    actor: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    Cli::parse();
    let root = std::env::current_dir()?;
    let server = RumbMcp::new(root);
    let service = serve_server(server, transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

fn structured(value: Value) -> CallToolResult {
    CallToolResult::structured(value)
}

fn item_json(item: &Item) -> Value {
    json!({
        "id": item.id,
        "kind": item.kind,
        "status": item.status.to_string(),
        "title": item.title,
        "parent_id": item.parent_id,
        "source_ref": item.source_ref,
    })
}

fn edge_json(edge: &Edge) -> Value {
    json!({
        "from": edge.from,
        "to": edge.to,
        "kind": edge.kind.to_string(),
        "created_at": edge.created_at,
    })
}

fn claim_json(claim: &Claim) -> Value {
    json!({
        "id": claim.id,
        "item_id": claim.item_id,
        "actor_id": claim.actor_id,
        "status": claim.status.to_string(),
        "branch": claim.branch,
        "worktree_path": claim.worktree_path,
        "lease_until": claim.lease_until,
    })
}

fn unlink_json(outcome: &UnlinkOutcome) -> Value {
    json!({
        "edge": edge_json(&outcome.edge),
        "newly_ready": outcome.newly_ready.iter().map(item_json).collect::<Vec<_>>(),
    })
}

fn merge_json(outcome: &MergeOutcome) -> Value {
    json!({
        "from": item_json(&outcome.from),
        "into": item_json(&outcome.into),
        "moved_children": outcome.moved_children,
        "supersedes_edge": edge_json(&outcome.supersedes_edge),
    })
}

fn run_json(run: &RunRecord) -> Value {
    json!({
        "id": run.id,
        "item_id": run.item_id,
        "status": run.status.to_string(),
        "output_path": run.output_path,
    })
}

fn event_json(event: &Event) -> Value {
    let payload = serde_json::from_str::<Value>(&event.payload).unwrap_or_else(|_| {
        json!({
            "raw": event.payload,
        })
    });
    json!({
        "timestamp": event.timestamp,
        "action": event.action,
        "object_type": event.object_type,
        "object_id": event.object_id,
        "payload": payload,
    })
}

fn parse_status(value: &str) -> Result<Status, ErrorData> {
    Status::from_str(value).map_err(to_mcp_error)
}

fn parse_edge_kind(value: &str) -> Result<EdgeKind, ErrorData> {
    EdgeKind::from_str(value).map_err(to_mcp_error)
}

fn ttl_seconds(value: Option<&str>) -> Result<u64, ErrorData> {
    value
        .map(parse_ttl)
        .unwrap_or_else(|| Ok(default_ttl_seconds()))
        .map_err(to_mcp_error)
}

fn to_mcp_error(err: RumbError) -> ErrorData {
    ErrorData::internal_error(err.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rumb::{RunStatus, Status};

    #[test]
    fn item_json_has_mcp_shape() {
        let item = Item {
            id: "RUMB-0001".to_owned(),
            parent_id: Some("RUMB-0000".to_owned()),
            kind: "feature".to_owned(),
            title: "MCP".to_owned(),
            status: Status::Ready,
            source_ref: Some("README.md#mcp".to_owned()),
            created_at: 1,
            updated_at: 2,
        };

        assert_eq!(
            item_json(&item),
            json!({
                "id": "RUMB-0001",
                "kind": "feature",
                "status": "ready",
                "title": "MCP",
                "parent_id": "RUMB-0000",
                "source_ref": "README.md#mcp",
            })
        );
    }

    #[test]
    fn run_json_has_mcp_shape() {
        let run = RunRecord {
            id: "RUN-0001".to_owned(),
            item_id: "RUMB-0001".to_owned(),
            proposal_id: Some("PROP-0001".to_owned()),
            command: "cargo test".to_owned(),
            status: RunStatus::Passed,
            output_path: ".rumb/runs/RUN-0001.log".to_owned(),
            started_at: 1,
            finished_at: 2,
        };

        assert_eq!(
            run_json(&run),
            json!({
                "id": "RUN-0001",
                "item_id": "RUMB-0001",
                "status": "passed",
                "output_path": ".rumb/runs/RUN-0001.log",
            })
        );
    }
}
