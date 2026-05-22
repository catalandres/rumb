use std::fs;
use std::process::Command;

use duckdb::params;
use serde_json::json;

use super::graph::{dependencies_satisfied, item_depth};
use super::model::*;
use super::store::*;
use super::RumbProject;

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

        let output = Command::new("git")
            .arg("worktree")
            .arg("add")
            .arg("-b")
            .arg(&claim.branch)
            .arg(&worktree)
            .current_dir(&self.root)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            Err(RumbError::GitFailed(format!(
                "git worktree add -b {} {} exited with {}{}",
                claim.branch,
                worktree.display(),
                output.status,
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr}")
                }
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
}
