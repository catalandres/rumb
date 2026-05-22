use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

struct McpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpClient {
    fn start(cwd: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rumb-mcp"))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        writeln!(self.stdin, "{request}").unwrap();
        self.stdin.flush().unwrap();

        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(response["id"], id);
        assert!(
            response.get("error").is_none(),
            "MCP error response: {response}"
        );
        response["result"].clone()
    }

    fn notify(&mut self, method: &str, params: Value) {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        writeln!(self.stdin, "{notification}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn initialize(&mut self) -> Value {
        let result = self.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "rumb-mcp-smoke",
                    "version": "0.0.0",
                },
            }),
        );
        self.notify("notifications/initialized", json!({}));
        result
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.request(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments,
            }),
        );
        assert_eq!(result["isError"], false);
        result["structuredContent"].clone()
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn init_git_repo(path: &std::path::Path) {
    let status = Command::new("git")
        .arg("init")
        .current_dir(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn init_git_repo_with_commit(path: &std::path::Path) {
    init_git_repo(path);
    let status = Command::new("git")
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
    assert!(status.success());
}

#[test]
fn mcp_server_initializes_and_lists_rumb_tools() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let mut client = McpClient::start(dir.path());

    let initialize = client.initialize();
    assert_eq!(initialize["protocolVersion"], "2024-11-05");
    assert!(initialize["capabilities"]["tools"].is_object());

    let tools = client.request("tools/list", json!({}));
    let names = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();

    for expected in [
        "init",
        "doctor",
        "item_create",
        "item_status",
        "edge_add",
        "ready",
        "claim",
        "renew",
        "release",
        "run",
        "review",
        "done",
        "reparent",
        "edit",
        "recast",
        "unlink",
        "merge",
        "log",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
}

#[test]
fn mcp_tools_create_ready_and_log_structured_repo_state() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let mut client = McpClient::start(dir.path());
    client.initialize();

    let init = client.call_tool("init", json!({ "name": "rumb" }));
    assert_eq!(init["initialized"], true);
    assert_eq!(init["name"], "rumb");

    let item = client.call_tool(
        "item_create",
        json!({
            "kind": "feature",
            "title": "MCP smoke item",
            "parent": "RUMB-0000",
            "status": "ready",
            "source": "mcp-smoke",
        }),
    );
    assert_eq!(item["id"], "RUMB-0001");
    assert_eq!(item["kind"], "feature");
    assert_eq!(item["status"], "ready");
    assert_eq!(item["parent_id"], "RUMB-0000");
    assert_eq!(item["source_ref"], "mcp-smoke");

    let ready = client.call_tool("ready", json!({}));
    let items = ready["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], "RUMB-0001");
    assert_eq!(items[0]["title"], "MCP smoke item");
    assert_eq!(items[0]["status"], "ready");

    let log = client.call_tool("log", json!({ "id": "RUMB-0001" }));
    let events = log["events"].as_array().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["action"], "item.create");
    assert_eq!(events[0]["object_type"], "item");
    assert_eq!(events[0]["object_id"], "RUMB-0001");
    assert_eq!(events[0]["payload"]["kind"], "feature");
    assert_eq!(events[0]["payload"]["status"], "ready");
}

#[test]
fn mcp_grooming_verbs_reshape_the_graph() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo(dir.path());
    let mut client = McpClient::start(dir.path());
    client.initialize();

    client.call_tool("init", json!({ "name": "rumb" }));
    for title in ["A", "B", "Child"] {
        client.call_tool(
            "item_create",
            json!({ "kind": "feature", "title": title, "parent": "RUMB-0000", "status": "ready" }),
        );
    }
    // A = RUMB-0001, B = RUMB-0002, Child = RUMB-0003.

    let recast = client.call_tool(
        "recast",
        json!({ "id": "RUMB-0001", "kind": "spec", "actor": "mcp-smoke" }),
    );
    assert_eq!(recast["kind"], "spec");

    let edit = client.call_tool(
        "edit",
        json!({ "id": "RUMB-0001", "title": "A renamed", "actor": "mcp-smoke" }),
    );
    assert_eq!(edit["title"], "A renamed");

    let reparent = client.call_tool(
        "reparent",
        json!({ "id": "RUMB-0003", "under": "RUMB-0002", "actor": "mcp-smoke" }),
    );
    assert_eq!(reparent["parent_id"], "RUMB-0002");

    client.call_tool(
        "edge_add",
        json!({ "from": "RUMB-0002", "to": "RUMB-0001", "kind": "depends_on" }),
    );
    let unlink = client.call_tool(
        "unlink",
        json!({ "from": "RUMB-0002", "to": "RUMB-0001", "kind": "depends_on", "actor": "mcp-smoke" }),
    );
    assert_eq!(unlink["edge"]["from"], "RUMB-0002");

    let merge = client.call_tool(
        "merge",
        json!({ "from": "RUMB-0001", "into": "RUMB-0002", "actor": "mcp-smoke" }),
    );
    assert_eq!(merge["from"]["status"], "superseded");
    assert_eq!(merge["supersedes_edge"]["from"], "RUMB-0002");
    assert_eq!(merge["supersedes_edge"]["to"], "RUMB-0001");
}

#[test]
fn mcp_tools_claim_run_and_release_in_temp_git_repo() {
    let dir = tempfile::tempdir().unwrap();
    init_git_repo_with_commit(dir.path());
    let mut client = McpClient::start(dir.path());
    client.initialize();

    client.call_tool("init", json!({ "name": "rumb" }));
    let item = client.call_tool(
        "item_create",
        json!({
            "kind": "feature",
            "title": "MCP claim run",
            "parent": "RUMB-0000",
            "status": "ready",
            "source": "mcp-claim-run-smoke",
        }),
    );
    assert_eq!(item["id"], "RUMB-0001");

    let claim = client.call_tool(
        "claim",
        json!({
            "id": "RUMB-0001",
            "actor": "mcp-smoke",
            "confirm_foundation": true,
        }),
    );
    assert_eq!(claim["id"], "CLAIM-0001");
    assert_eq!(claim["item_id"], "RUMB-0001");
    assert_eq!(claim["actor_id"], "mcp-smoke");
    assert_eq!(claim["status"], "active");
    assert_eq!(claim["branch"], "rumb/RUMB-0001-mcp-claim-run");
    assert_eq!(
        claim["worktree_path"],
        ".rumb/worktrees/RUMB-0001-mcp-claim-run"
    );
    assert!(claim["lease_until"].as_u64().unwrap() > 0);
    assert!(dir
        .path()
        .join(claim["worktree_path"].as_str().unwrap())
        .is_dir());

    let run = client.call_tool(
        "run",
        json!({
            "id": "RUMB-0001",
            "actor": "mcp-smoke",
            "command": ["sh", "-c", "printf mcp-run-ok"],
        }),
    );
    assert_eq!(run["id"], "RUN-0001");
    assert_eq!(run["item_id"], "RUMB-0001");
    assert_eq!(run["status"], "passed");
    assert_eq!(run["output_path"], ".rumb/runs/RUN-0001.log");

    let log_path = dir.path().join(run["output_path"].as_str().unwrap());
    let log = std::fs::read_to_string(log_path).unwrap();
    assert!(log.contains("command\tsh -c printf mcp-run-ok"));
    assert!(log.contains("status\tpassed"));
    assert!(log.contains("[stdout]\nmcp-run-ok"));

    let release = client.call_tool(
        "release",
        json!({
            "claim_id": claim["id"],
            "actor": "mcp-smoke",
        }),
    );
    assert_eq!(release["id"], "CLAIM-0001");
    assert_eq!(release["status"], "released");
}
