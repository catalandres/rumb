use std::fs;
use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::RumbError;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct McpInstallOptions {
    pub root: PathBuf,
    pub name: String,
    pub command: Option<String>,
    pub target: PathBuf,
    pub force: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct McpInstallReport {
    pub name: String,
    pub target: PathBuf,
    pub command: String,
    pub args: Vec<String>,
}

pub fn install_mcp(options: McpInstallOptions) -> Result<McpInstallReport, RumbError> {
    if options.name.trim().is_empty() {
        return Err(RumbError::McpInstall(
            "server name must not be empty".to_owned(),
        ));
    }

    let target = resolve_target(&options.root, &options.target);
    let command = options.command.unwrap_or_else(|| "rumb".to_owned());
    let args = vec!["mcp".to_owned(), "serve".to_owned()];

    let mut document = read_mcp_config(&target)?;
    let root = document.as_object_mut().ok_or_else(|| {
        RumbError::McpInstall(format!("{} must contain a JSON object", target.display()))
    })?;
    let servers = ensure_mcp_servers(root)?;

    if servers.contains_key(&options.name) && !options.force {
        return Err(RumbError::McpInstall(format!(
            "mcp server '{}' already exists in {}; use --force to replace it",
            options.name,
            target.display()
        )));
    }

    servers.insert(
        options.name.clone(),
        json!({
            "command": command,
            "args": args,
        }),
    );

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let output = serde_json::to_string_pretty(&document)
        .map_err(|error| RumbError::McpInstall(error.to_string()))?;
    fs::write(&target, format!("{output}\n"))?;

    Ok(McpInstallReport {
        name: options.name,
        target,
        command,
        args: vec!["mcp".to_owned(), "serve".to_owned()],
    })
}

fn resolve_target(root: &std::path::Path, target: &std::path::Path) -> PathBuf {
    if target.is_absolute() {
        target.to_path_buf()
    } else {
        root.join(target)
    }
}

fn read_mcp_config(target: &std::path::Path) -> Result<Value, RumbError> {
    if !target.exists() {
        return Ok(json!({}));
    }
    let input = fs::read_to_string(target)?;
    serde_json::from_str(&input)
        .map_err(|error| RumbError::McpInstall(format!("invalid {}: {error}", target.display())))
}

fn ensure_mcp_servers(root: &mut Map<String, Value>) -> Result<&mut Map<String, Value>, RumbError> {
    let entry = root
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    entry
        .as_object_mut()
        .ok_or_else(|| RumbError::McpInstall("mcpServers must be a JSON object".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_mcp_writes_project_config() {
        let dir = tempfile::tempdir().unwrap();
        let report = install_mcp(McpInstallOptions {
            root: dir.path().to_path_buf(),
            name: "rumb".to_owned(),
            command: None,
            target: PathBuf::from(".mcp.json"),
            force: false,
        })
        .unwrap();

        assert_eq!(report.name, "rumb");
        assert_eq!(report.args, vec!["mcp", "serve"]);
        assert_eq!(report.target, dir.path().join(".mcp.json"));

        let config: Value =
            serde_json::from_str(&fs::read_to_string(report.target).unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["rumb"]["command"], "rumb");
        assert_eq!(
            config["mcpServers"]["rumb"]["args"],
            json!(["mcp", "serve"])
        );
    }

    #[test]
    fn install_mcp_preserves_existing_servers_and_requires_force() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join(".mcp.json");
        fs::write(
            &target,
            r#"{"mcpServers":{"other":{"command":"other","args":[]}}}"#,
        )
        .unwrap();

        install_mcp(McpInstallOptions {
            root: dir.path().to_path_buf(),
            name: "rumb".to_owned(),
            command: Some("rumb".to_owned()),
            target: PathBuf::from(".mcp.json"),
            force: false,
        })
        .unwrap();
        let duplicate = install_mcp(McpInstallOptions {
            root: dir.path().to_path_buf(),
            name: "rumb".to_owned(),
            command: Some("replacement".to_owned()),
            target: PathBuf::from(".mcp.json"),
            force: false,
        });
        assert!(duplicate.is_err());

        install_mcp(McpInstallOptions {
            root: dir.path().to_path_buf(),
            name: "rumb".to_owned(),
            command: Some("replacement".to_owned()),
            target: PathBuf::from(".mcp.json"),
            force: true,
        })
        .unwrap();

        let config: Value = serde_json::from_str(&fs::read_to_string(target).unwrap()).unwrap();
        assert_eq!(config["mcpServers"]["other"]["command"], "other");
        assert_eq!(config["mcpServers"]["rumb"]["command"], "replacement");
    }
}
