use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use clap::Parser;
use rumb::cli::{Cli, Command, EdgeCommand, ItemCommand, McpCommand, ViewCommand};
use rumb::{
    install_mcp, parse_ttl, AddEdge, Capture, ClaimItem, CreateItem, Digest, DoneItem, EditItem,
    GroomNote, InitOptions, Item, ItemDetails, McpInstallOptions, Merge, MergeOutcome, Recast,
    ReleaseClaim, RenewClaim, Reparent, ReviewItem, RumbProject, RunCommand, Unlink, UnlinkOutcome,
    UpdateItemStatus,
};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), rumb::RumbError> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;

    match cli.command {
        Command::Init { name } => {
            let project = RumbProject::open(cwd);
            project.init(&InitOptions { name: name.clone() })?;
            println!("initialized\t{name}");
        }
        Command::Doctor => {
            let project = RumbProject::discover(cwd)?;
            let report = project.doctor()?;
            print_bool("state_dir", report.state_dir_exists);
            print_bool("state_file", report.state_file_exists);
            print_bool("git_ignore", report.rumb_ignored_by_git);
            if report.ok() {
                println!("ok");
            } else {
                return Err(rumb::RumbError::DoctorFailed);
            }
        }
        Command::Item {
            command:
                ItemCommand::Create {
                    kind,
                    title,
                    parent,
                    status,
                    tier,
                    source,
                },
        } => {
            let project = RumbProject::discover(cwd)?;
            let item = project.create_item(CreateItem {
                kind,
                title,
                parent_id: parent,
                status,
                tier,
                source_ref: source,
            })?;
            println!(
                "{}\t{}\t{}\t{}",
                item.id, item.kind, item.status, item.title
            );
        }
        Command::Item {
            command: ItemCommand::Status { id, status, actor },
        } => {
            let project = RumbProject::discover(cwd)?;
            let item = project.update_item_status(UpdateItemStatus {
                item_id: id,
                status,
                actor,
            })?;
            println!(
                "{}\t{}\t{}\t{}",
                item.id, item.kind, item.status, item.title
            );
        }
        Command::Edge {
            command: EdgeCommand::Add { from, to, kind },
        } => {
            let project = RumbProject::discover(cwd)?;
            let edge = project.add_edge(AddEdge { from, to, kind })?;
            println!("{}\t{}\t{}", edge.from, edge.to, edge.kind);
        }
        Command::List => {
            let project = RumbProject::discover(cwd)?;
            print_item_tree(&project.list_items()?);
        }
        Command::View {
            command: ViewCommand::Item { reference },
        } => {
            let project = RumbProject::discover(cwd)?;
            print_item_details(&project.item_details(&reference)?);
        }
        Command::Ready => {
            let project = RumbProject::discover(cwd)?;
            for item in project.ready_items()? {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    item.id, item.kind, item.status, item.tier, item.title
                );
            }
        }
        Command::Claim {
            id,
            actor,
            ttl,
            confirm_foundation,
        } => {
            let project = RumbProject::discover(cwd)?;
            let claim = project.claim_item(ClaimItem {
                item_id: id,
                actor,
                ttl_seconds: parse_ttl(&ttl)?,
                confirm_foundation,
            })?;
            print_claim(&claim);
        }
        Command::Renew {
            claim_id,
            actor,
            ttl,
        } => {
            let project = RumbProject::discover(cwd)?;
            let claim = project.renew_claim(RenewClaim {
                claim_id,
                actor,
                ttl_seconds: parse_ttl(&ttl)?,
            })?;
            print_claim(&claim);
        }
        Command::Release { claim_id, actor } => {
            let project = RumbProject::discover(cwd)?;
            let claim = project.release_claim(ReleaseClaim { claim_id, actor })?;
            print_claim(&claim);
        }
        Command::Run { id, actor, command } => {
            let project = RumbProject::discover(cwd)?;
            let run = project.run_command(RunCommand {
                item_id: id,
                actor,
                command,
            })?;
            println!(
                "{}\t{}\t{}\t{}",
                run.id, run.item_id, run.status, run.output_path
            );
        }
        Command::Review { id, actor } => {
            let project = RumbProject::discover(cwd)?;
            let item = project.review_item(ReviewItem { item_id: id, actor })?;
            println!(
                "{}\t{}\t{}\t{}",
                item.id, item.kind, item.status, item.title
            );
        }
        Command::Done { id, actor } => {
            let project = RumbProject::discover(cwd)?;
            let item = project.done_item(DoneItem { item_id: id, actor })?;
            println!(
                "{}\t{}\t{}\t{}",
                item.id, item.kind, item.status, item.title
            );
        }
        Command::Reparent {
            id,
            under,
            actor,
            confirm,
            rejected,
            why,
        } => {
            let project = RumbProject::discover(cwd)?;
            let item = project.reparent(Reparent {
                item_id: id,
                new_parent_id: under,
                actor,
                confirm,
                note: GroomNote { rejected, why },
            })?;
            println!(
                "{}\t{}\t{}\t{}",
                item.id,
                item.parent_id.as_deref().unwrap_or(""),
                item.status,
                item.title
            );
        }
        Command::Edit {
            id,
            title,
            source,
            tier,
            actor,
            rejected,
            why,
        } => {
            let project = RumbProject::discover(cwd)?;
            let item = project.edit(EditItem {
                item_id: id,
                title,
                source_ref: source,
                tier,
                actor,
                note: GroomNote { rejected, why },
            })?;
            println!(
                "{}\t{}\t{}\t{}",
                item.id, item.kind, item.status, item.title
            );
        }
        Command::Recast {
            id,
            kind,
            actor,
            rejected,
            why,
        } => {
            let project = RumbProject::discover(cwd)?;
            let item = project.recast(Recast {
                item_id: id,
                kind,
                actor,
                note: GroomNote { rejected, why },
            })?;
            println!(
                "{}\t{}\t{}\t{}",
                item.id, item.kind, item.status, item.title
            );
        }
        Command::Unlink {
            from,
            to,
            kind,
            actor,
            rejected,
            why,
        } => {
            let project = RumbProject::discover(cwd)?;
            let outcome = project.unlink(Unlink {
                from,
                to,
                kind,
                actor,
                note: GroomNote { rejected, why },
            })?;
            print_unlink(&outcome);
        }
        Command::Merge {
            from,
            into,
            actor,
            rejected,
            why,
        } => {
            let project = RumbProject::discover(cwd)?;
            let outcome = project.merge(Merge {
                from_id: from,
                into_id: into,
                actor,
                note: GroomNote { rejected, why },
            })?;
            print_merge(&outcome);
        }
        Command::Capture { text } => {
            let project = RumbProject::discover(cwd)?;
            let item = project.capture(Capture { text })?;
            println!(
                "{}\t{}\t{}\t{}",
                item.id, item.kind, item.status, item.title
            );
        }
        Command::Digest => {
            let project = RumbProject::discover(cwd)?;
            print_digest(&project.digest()?);
        }
        Command::Undo => {
            let project = RumbProject::discover(cwd)?;
            let outcome = project.undo()?;
            println!(
                "undone\t{}\t{}\t{}",
                outcome.seq, outcome.verb, outcome.object_id
            );
        }
        Command::At { seq } => {
            let project = RumbProject::discover(cwd)?;
            let graph = project.at(seq)?;
            print_item_tree(&graph.items);
        }
        Command::Mcp {
            command: McpCommand::Serve,
        } => {
            serve_mcp()?;
        }
        Command::Mcp {
            command:
                McpCommand::Install {
                    name,
                    command,
                    target,
                    force,
                },
        } => {
            let root = mcp_install_root(&cwd);
            let command = command.or_else(|| default_mcp_install_command(&root));
            let report = install_mcp(McpInstallOptions {
                root,
                name,
                command,
                target,
                force,
            })?;
            println!(
                "{}\t{}\t{}\t{}",
                report.name,
                report.target.display(),
                report.command,
                report.args.join(" ")
            );
        }
        Command::Log { id } => {
            let project = RumbProject::discover(cwd)?;
            for event in project.events(id.as_deref())? {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    event.timestamp,
                    event.action,
                    event.object_type,
                    event.object_id,
                    event.payload
                );
            }
        }
    }

    Ok(())
}

fn print_bool(name: &str, value: bool) {
    println!("{name}\t{}", if value { "ok" } else { "fail" });
}

fn print_claim(claim: &rumb::Claim) {
    println!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        claim.id, claim.item_id, claim.actor_id, claim.status, claim.branch, claim.worktree_path
    );
}

fn print_digest(digest: &Digest) {
    println!("spirals");
    if digest.spirals.is_empty() {
        println!("none");
    } else {
        for spiral in &digest.spirals {
            println!(
                "{}\t{}\t{} failed runs",
                spiral.item.id, spiral.item.title, spiral.failed_runs
            );
        }
    }

    println!("\nthreads");
    if digest.threads.is_empty() {
        println!("none");
    } else {
        for thread in &digest.threads {
            println!("{}\t{}", thread.title, thread.item_ids.join(","));
        }
    }

    println!("\nstale_inbox");
    if digest.stale_inbox.is_empty() {
        println!("none");
    } else {
        for item in &digest.stale_inbox {
            println!("{}\t{}", item.id, item.title);
        }
    }

    println!("\nmomentum");
    if digest.momentum.is_empty() {
        println!("none");
    } else {
        for entry in &digest.momentum {
            println!("{}\t{}", entry.kind, entry.count);
        }
    }
}

fn print_unlink(outcome: &UnlinkOutcome) {
    let edge = &outcome.edge;
    println!("unlinked\t{}\t{}\t{}", edge.from, edge.to, edge.kind);
    for item in &outcome.newly_ready {
        println!("ready\t{}\t{}\t{}", item.id, item.kind, item.title);
    }
}

fn print_merge(outcome: &MergeOutcome) {
    println!(
        "merged\t{}\t{}\t{}",
        outcome.from.id, outcome.into.id, outcome.from.status
    );
    for child in &outcome.moved_children {
        println!("moved\t{child}\t{}", outcome.into.id);
    }
    let edge = &outcome.supersedes_edge;
    println!("supersedes\t{}\t{}\t{}", edge.from, edge.to, edge.kind);
}

fn print_item_tree(items: &[Item]) {
    print!("{}", format_item_tree(items));
}

fn format_item_tree(items: &[Item]) -> String {
    let mut by_parent: HashMap<Option<String>, Vec<&Item>> = HashMap::new();
    for item in items {
        by_parent
            .entry(item.parent_id.clone())
            .or_default()
            .push(item);
    }
    for children in by_parent.values_mut() {
        children.sort_by(|left, right| left.id.cmp(&right.id));
    }

    let mut printed = HashSet::new();
    let mut output = String::new();
    let roots = by_parent.remove(&None).unwrap_or_default();
    for (index, root) in roots.iter().enumerate() {
        print_tree_node(
            root,
            "",
            index + 1 == roots.len(),
            true,
            &by_parent,
            &mut printed,
            &mut output,
        );
    }

    let mut orphans = items
        .iter()
        .filter(|item| !printed.contains(item.id.as_str()))
        .collect::<Vec<_>>();
    orphans.sort_by(|left, right| left.id.cmp(&right.id));
    for item in orphans {
        output.push_str(&format!(
            "{} [{} {}] {}\n",
            item.id, item.kind, item.status, item.title
        ));
    }
    output
}

fn print_tree_node(
    item: &Item,
    prefix: &str,
    is_last: bool,
    is_root: bool,
    by_parent: &HashMap<Option<String>, Vec<&Item>>,
    printed: &mut HashSet<String>,
    output: &mut String,
) {
    let connector = if is_root {
        ""
    } else if is_last {
        "`-- "
    } else {
        "|-- "
    };
    output.push_str(&format!(
        "{}{}{} [{} {}] {}\n",
        prefix, connector, item.id, item.kind, item.status, item.title
    ));
    printed.insert(item.id.clone());

    let child_prefix = if is_root {
        String::new()
    } else if is_last {
        format!("{prefix}    ")
    } else {
        format!("{prefix}|   ")
    };
    if let Some(children) = by_parent.get(&Some(item.id.clone())) {
        for (index, child) in children.iter().enumerate() {
            print_tree_node(
                child,
                &child_prefix,
                index + 1 == children.len(),
                false,
                by_parent,
                printed,
                output,
            );
        }
    }
}

fn print_item_details(details: &ItemDetails) {
    print!("{}", format_item_details(details));
}

fn format_item_details(details: &ItemDetails) -> String {
    let item = &details.item;
    let mut output = String::new();
    output.push_str(&format!("id\t{}\n", item.id));
    output.push_str(&format!(
        "parent_id\t{}\n",
        item.parent_id.as_deref().unwrap_or("")
    ));
    output.push_str(&format!("kind\t{}\n", item.kind));
    output.push_str(&format!("title\t{}\n", item.title));
    output.push_str(&format!("status\t{}\n", item.status));
    output.push_str(&format!("tier\t{}\n", item.tier));
    output.push_str(&format!(
        "source_ref\t{}\n",
        item.source_ref.as_deref().unwrap_or("")
    ));
    output.push_str(&format!("body\t{}\n", item.body.as_deref().unwrap_or("")));
    output.push_str(&format!("created_at\t{}\n", item.created_at));
    output.push_str(&format!("updated_at\t{}\n", item.updated_at));
    output.push_str(&format!("depth\t{}\n", details.depth));

    output.push_str("\nchildren\n");
    if details.children.is_empty() {
        output.push_str("none\n");
    } else {
        for child in &details.children {
            output.push_str(&format!(
                "{}\t{}\t{}\t{}\n",
                child.id, child.kind, child.status, child.title
            ));
        }
    }

    output.push_str("\nincoming_edges\n");
    if details.incoming_edges.is_empty() {
        output.push_str("none\n");
    } else {
        for edge in &details.incoming_edges {
            output.push_str(&format!("{}\t{}\t{}\n", edge.from, edge.to, edge.kind));
        }
    }

    output.push_str("\noutgoing_edges\n");
    if details.outgoing_edges.is_empty() {
        output.push_str("none\n");
    } else {
        for edge in &details.outgoing_edges {
            output.push_str(&format!("{}\t{}\t{}\n", edge.from, edge.to, edge.kind));
        }
    }

    output.push_str("\nclaims\n");
    if details.claims.is_empty() {
        output.push_str("none\n");
    } else {
        for claim in &details.claims {
            output.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                claim.id,
                claim.actor_id,
                claim.status,
                claim.lease_until,
                claim.branch,
                claim.worktree_path,
                claim.updated_at
            ));
        }
    }

    output.push_str("\nproposals\n");
    if details.proposals.is_empty() {
        output.push_str("none\n");
    } else {
        for proposal in &details.proposals {
            output.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                proposal.id,
                proposal.status,
                proposal.branch,
                proposal.base_ref,
                proposal.head_ref.as_deref().unwrap_or(""),
                proposal.updated_at
            ));
        }
    }

    output.push_str("\nruns\n");
    if details.runs.is_empty() {
        output.push_str("none\n");
    } else {
        for run in &details.runs {
            output.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                run.id, run.status, run.command, run.output_path, run.started_at, run.finished_at
            ));
        }
    }

    output.push_str("\nevents\n");
    if details.events.is_empty() {
        output.push_str("none\n");
    } else {
        for event in &details.events {
            output.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\n",
                event.timestamp, event.action, event.object_type, event.object_id, event.payload
            ));
        }
    }
    output
}

fn serve_mcp() -> Result<(), rumb::RumbError> {
    let command = mcp_server_command()?;
    let status = ProcessCommand::new(&command).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(rumb::RumbError::McpInstall(format!(
            "{} exited with {status}",
            command.display()
        )))
    }
}

fn mcp_server_command() -> Result<PathBuf, rumb::RumbError> {
    if let Ok(shim) = std::env::var("RUMB_MCP_SHIM") {
        if !shim.trim().is_empty() {
            return Ok(PathBuf::from(shim));
        }
    }

    if let Ok(home) = std::env::var("RUMB_HOME") {
        let home = PathBuf::from(home);
        for candidate in [
            home.join("target/release/rumb-mcp"),
            home.join("target/debug/rumb-mcp"),
            home.join("bin/rumb-mcp"),
        ] {
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            let sibling = parent.join("rumb-mcp");
            if sibling.exists() {
                return Ok(sibling);
            }
        }
    }

    Ok(PathBuf::from("rumb-mcp"))
}

fn mcp_install_root(cwd: &Path) -> PathBuf {
    if let Ok(project) = RumbProject::discover(cwd) {
        return project.root().to_path_buf();
    }

    let output = ProcessCommand::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(cwd)
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !root.is_empty() {
                return PathBuf::from(root);
            }
        }
    }

    cwd.to_path_buf()
}

fn default_mcp_install_command(root: &Path) -> Option<String> {
    let shim = std::env::var("RUMB_SHIM")
        .ok()
        .filter(|value| !value.trim().is_empty())?;
    let path = PathBuf::from(&shim);
    if let Ok(relative) = path.strip_prefix(root) {
        return Some(relative.display().to_string());
    }
    Some(shim)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rumb::{
        Claim, ClaimStatus, Edge, EdgeKind, Event, Item, Proposal, RunRecord, RunStatus, Status,
        Tier,
    };

    fn item(id: &str, parent_id: Option<&str>, title: &str) -> Item {
        Item {
            id: id.to_owned(),
            parent_id: parent_id.map(str::to_owned),
            kind: "feature".to_owned(),
            title: title.to_owned(),
            status: Status::Ready,
            tier: Tier::Standard,
            source_ref: None,
            body: None,
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn tree_rendering_is_deterministic_for_roots_and_children() {
        let items = vec![
            item("RUMB-0003", Some("RUMB-0001"), "Child B"),
            item("RUMB-0002", Some("RUMB-0001"), "Child A"),
            item("RUMB-0001", None, "Root A"),
            item("RUMB-0004", None, "Root B"),
            item("RUMB-0005", Some("RUMB-4040"), "Orphan"),
        ];

        assert_eq!(
            format_item_tree(&items),
            concat!(
                "RUMB-0001 [feature ready] Root A\n",
                "|-- RUMB-0002 [feature ready] Child A\n",
                "`-- RUMB-0003 [feature ready] Child B\n",
                "RUMB-0004 [feature ready] Root B\n",
                "RUMB-0005 [feature ready] Orphan\n",
            )
        );
    }

    #[test]
    fn view_item_rendering_includes_expected_section_headers() {
        let details = ItemDetails {
            item: item("RUMB-0007", Some("RUMB-0000"), "Details"),
            depth: 1,
            children: vec![item("RUMB-0008", Some("RUMB-0007"), "Child")],
            incoming_edges: vec![Edge {
                from: "RUMB-0008".to_owned(),
                to: "RUMB-0007".to_owned(),
                kind: EdgeKind::DependsOn,
                created_at: 3,
            }],
            outgoing_edges: vec![Edge {
                from: "RUMB-0007".to_owned(),
                to: "RUMB-0001".to_owned(),
                kind: EdgeKind::RelatesTo,
                created_at: 4,
            }],
            claims: vec![Claim {
                id: "CLAIM-0001".to_owned(),
                item_id: "RUMB-0007".to_owned(),
                actor_id: "operator".to_owned(),
                lease_until: 10,
                status: ClaimStatus::Active,
                branch: "rumb/RUMB-0007-details".to_owned(),
                worktree_path: ".rumb/worktrees/RUMB-0007-details".to_owned(),
                created_at: 5,
                updated_at: 6,
            }],
            proposals: vec![Proposal {
                id: "PROP-0001".to_owned(),
                item_id: "RUMB-0007".to_owned(),
                branch: "rumb/RUMB-0007-details".to_owned(),
                base_ref: "main".to_owned(),
                head_ref: None,
                status: "open".to_owned(),
                created_at: 7,
                updated_at: 8,
            }],
            runs: vec![RunRecord {
                id: "RUN-0001".to_owned(),
                item_id: "RUMB-0007".to_owned(),
                proposal_id: Some("PROP-0001".to_owned()),
                command: "cargo test".to_owned(),
                status: RunStatus::Passed,
                output_path: ".rumb/runs/RUN-0001.log".to_owned(),
                started_at: 9,
                finished_at: 10,
            }],
            events: vec![Event {
                timestamp: 11,
                action: "item.done".to_owned(),
                object_type: "item".to_owned(),
                object_id: "RUMB-0007".to_owned(),
                payload: "{\"actor\":\"operator\"}".to_owned(),
            }],
        };

        let output = format_item_details(&details);
        for header in [
            "id\tRUMB-0007",
            "\nchildren\n",
            "\nincoming_edges\n",
            "\noutgoing_edges\n",
            "\nclaims\n",
            "\nproposals\n",
            "\nruns\n",
            "\nevents\n",
        ] {
            assert!(output.contains(header), "missing {header}");
        }
    }
}
