use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::core::{EdgeKind, Status, Tier};

#[derive(Debug, Parser)]
#[command(name = "rumb", version, about = "Local agent work coordinator")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Init {
        #[arg(long)]
        name: String,
    },
    Doctor,
    Item {
        #[command(subcommand)]
        command: ItemCommand,
    },
    Edge {
        #[command(subcommand)]
        command: EdgeCommand,
    },
    List,
    View {
        #[command(subcommand)]
        command: ViewCommand,
    },
    Ready,
    Claim {
        id: String,
        #[arg(long)]
        actor: String,
        #[arg(long, default_value = "4h")]
        ttl: String,
        #[arg(long)]
        confirm_foundation: bool,
    },
    Renew {
        claim_id: String,
        #[arg(long)]
        actor: String,
        #[arg(long, default_value = "4h")]
        ttl: String,
    },
    Release {
        claim_id: String,
        #[arg(long)]
        actor: String,
    },
    Run {
        id: String,
        #[arg(long)]
        actor: String,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    Review {
        id: String,
        #[arg(long)]
        actor: String,
    },
    Done {
        id: String,
        #[arg(long)]
        actor: String,
    },
    Reparent {
        id: String,
        #[arg(long)]
        under: String,
        #[arg(long)]
        actor: String,
        #[arg(long)]
        confirm: bool,
    },
    Edit {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        tier: Option<Tier>,
        #[arg(long)]
        actor: String,
    },
    Recast {
        id: String,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        actor: String,
    },
    Unlink {
        from: String,
        to: String,
        #[arg(long)]
        kind: EdgeKind,
        #[arg(long)]
        actor: String,
    },
    Merge {
        from: String,
        #[arg(long)]
        into: String,
        #[arg(long)]
        actor: String,
    },
    Capture {
        text: String,
    },
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    Log {
        id: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ItemCommand {
    Create {
        #[arg(long)]
        kind: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        parent: String,
        #[arg(long, default_value_t = Status::Draft)]
        status: Status,
        #[arg(long, default_value_t = Tier::Standard)]
        tier: Tier,
        #[arg(long)]
        source: Option<String>,
    },
    Status {
        id: String,
        status: Status,
        #[arg(long)]
        actor: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum EdgeCommand {
    Add {
        from: String,
        to: String,
        #[arg(long)]
        kind: EdgeKind,
    },
}

#[derive(Debug, Subcommand)]
pub enum ViewCommand {
    Item { reference: String },
}

#[derive(Debug, Subcommand)]
pub enum McpCommand {
    Serve,
    Install {
        #[arg(long, default_value = "rumb")]
        name: String,
        #[arg(long)]
        command: Option<String>,
        #[arg(long, default_value = ".mcp.json")]
        target: PathBuf,
        #[arg(long)]
        force: bool,
    },
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command, EdgeCommand, ItemCommand, McpCommand, ViewCommand};
    use crate::core::{EdgeKind, Status, Tier};

    #[test]
    fn parses_item_create_with_defaults() {
        let cli = Cli::parse_from([
            "rumb",
            "item",
            "create",
            "--kind",
            "feature",
            "--title",
            "Claim flow",
            "--parent",
            "RUMB-0000",
        ]);

        match cli.command {
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
                assert_eq!(kind, "feature");
                assert_eq!(title, "Claim flow");
                assert_eq!(parent, "RUMB-0000");
                assert_eq!(status, Status::Draft);
                assert_eq!(tier, Tier::Standard);
                assert_eq!(source, None);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_edge_kind() {
        let cli = Cli::parse_from([
            "rumb",
            "edge",
            "add",
            "RUMB-0001",
            "RUMB-0002",
            "--kind",
            "depends_on",
        ]);

        match cli.command {
            Command::Edge {
                command: EdgeCommand::Add { kind, .. },
            } => assert_eq!(kind, EdgeKind::DependsOn),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_item_status() {
        let cli = Cli::parse_from([
            "rumb",
            "item",
            "status",
            "RUMB-0001",
            "done",
            "--actor",
            "operator",
        ]);

        match cli.command {
            Command::Item {
                command: ItemCommand::Status { id, status, actor },
            } => {
                assert_eq!(id, "RUMB-0001");
                assert_eq!(status, Status::Done);
                assert_eq!(actor, "operator");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_claim_with_defaults() {
        let cli = Cli::parse_from(["rumb", "claim", "RUMB-0001", "--actor", "operator"]);

        match cli.command {
            Command::Claim {
                id,
                actor,
                ttl,
                confirm_foundation,
            } => {
                assert_eq!(id, "RUMB-0001");
                assert_eq!(actor, "operator");
                assert_eq!(ttl, "4h");
                assert!(!confirm_foundation);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_run_trailing_command() {
        let cli = Cli::parse_from([
            "rumb",
            "run",
            "RUMB-0001",
            "--actor",
            "operator",
            "--",
            "cargo",
            "test",
        ]);

        match cli.command {
            Command::Run { id, actor, command } => {
                assert_eq!(id, "RUMB-0001");
                assert_eq!(actor, "operator");
                assert_eq!(command, vec!["cargo", "test"]);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_review() {
        let cli = Cli::parse_from(["rumb", "review", "RUMB-0001", "--actor", "operator"]);

        match cli.command {
            Command::Review { id, actor } => {
                assert_eq!(id, "RUMB-0001");
                assert_eq!(actor, "operator");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_done() {
        let cli = Cli::parse_from(["rumb", "done", "RUMB-0001", "--actor", "operator"]);

        match cli.command {
            Command::Done { id, actor } => {
                assert_eq!(id, "RUMB-0001");
                assert_eq!(actor, "operator");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_reparent_with_confirm() {
        let cli = Cli::parse_from([
            "rumb",
            "reparent",
            "RUMB-0002",
            "--under",
            "RUMB-0000",
            "--actor",
            "operator",
            "--confirm",
        ]);

        match cli.command {
            Command::Reparent {
                id,
                under,
                actor,
                confirm,
            } => {
                assert_eq!(id, "RUMB-0002");
                assert_eq!(under, "RUMB-0000");
                assert_eq!(actor, "operator");
                assert!(confirm);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_edit_optional_fields() {
        let cli = Cli::parse_from([
            "rumb",
            "edit",
            "RUMB-0002",
            "--title",
            "New title",
            "--actor",
            "operator",
        ]);

        match cli.command {
            Command::Edit {
                id,
                title,
                source,
                tier,
                actor,
            } => {
                assert_eq!(id, "RUMB-0002");
                assert_eq!(title.as_deref(), Some("New title"));
                assert_eq!(source, None);
                assert_eq!(tier, None);
                assert_eq!(actor, "operator");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_recast() {
        let cli = Cli::parse_from([
            "rumb",
            "recast",
            "RUMB-0002",
            "--kind",
            "spec",
            "--actor",
            "operator",
        ]);

        match cli.command {
            Command::Recast { id, kind, actor } => {
                assert_eq!(id, "RUMB-0002");
                assert_eq!(kind, "spec");
                assert_eq!(actor, "operator");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_unlink_edge_kind() {
        let cli = Cli::parse_from([
            "rumb",
            "unlink",
            "RUMB-0001",
            "RUMB-0002",
            "--kind",
            "depends_on",
            "--actor",
            "operator",
        ]);

        match cli.command {
            Command::Unlink {
                from,
                to,
                kind,
                actor,
            } => {
                assert_eq!(from, "RUMB-0001");
                assert_eq!(to, "RUMB-0002");
                assert_eq!(kind, EdgeKind::DependsOn);
                assert_eq!(actor, "operator");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_item_create_with_tier() {
        let cli = Cli::parse_from([
            "rumb",
            "item",
            "create",
            "--kind",
            "feature",
            "--title",
            "T",
            "--parent",
            "RUMB-0000",
            "--tier",
            "hard",
        ]);

        match cli.command {
            Command::Item {
                command: ItemCommand::Create { tier, .. },
            } => assert_eq!(tier, Tier::Hard),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_capture() {
        let cli = Cli::parse_from(["rumb", "capture", "a quick thought"]);

        match cli.command {
            Command::Capture { text } => assert_eq!(text, "a quick thought"),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_merge() {
        let cli = Cli::parse_from([
            "rumb",
            "merge",
            "RUMB-0003",
            "--into",
            "RUMB-0002",
            "--actor",
            "operator",
        ]);

        match cli.command {
            Command::Merge { from, into, actor } => {
                assert_eq!(from, "RUMB-0003");
                assert_eq!(into, "RUMB-0002");
                assert_eq!(actor, "operator");
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_list() {
        let cli = Cli::parse_from(["rumb", "list"]);

        match cli.command {
            Command::List => {}
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_view_item_reference() {
        let cli = Cli::parse_from(["rumb", "view", "item", "0007"]);

        match cli.command {
            Command::View {
                command: ViewCommand::Item { reference },
            } => assert_eq!(reference, "0007"),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_mcp_install_defaults() {
        let cli = Cli::parse_from(["rumb", "mcp", "install"]);

        match cli.command {
            Command::Mcp {
                command:
                    McpCommand::Install {
                        name,
                        command,
                        target,
                        force,
                    },
            } => {
                assert_eq!(name, "rumb");
                assert_eq!(command, None);
                assert_eq!(target, std::path::PathBuf::from(".mcp.json"));
                assert!(!force);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_mcp_serve() {
        let cli = Cli::parse_from(["rumb", "mcp", "serve"]);

        match cli.command {
            Command::Mcp {
                command: McpCommand::Serve,
            } => {}
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
