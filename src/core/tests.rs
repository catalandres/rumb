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
    assert!(events
        .iter()
        .any(|event| event.action == "item.review"
            && event.payload.contains("\"actor\":\"operator\"")));
    assert!(events.iter().any(
        |event| event.action == "item.done" && event.payload.contains("\"actor\":\"operator\"")
    ));
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
