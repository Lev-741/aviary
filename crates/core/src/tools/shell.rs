use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::process::Command;

use super::{Tool, str_arg};
use crate::error::{Error, Result};

pub struct ShellTool {
    workspace: PathBuf,
    timeout: Duration,
}

impl ShellTool {
    pub fn new(workspace: &Path, timeout_secs: u64) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            timeout: Duration::from_secs(timeout_secs.max(1)),
        }
    }
}

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str {
        "run_shell"
    }

    fn description(&self) -> &str {
        "Run a shell command in the workspace and return exit code, stdout and stderr."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"command": {"type": "string"}},
            "required": ["command"]
        })
    }

    async fn call(&self, args: Value) -> Result<String> {
        let command = str_arg(self.name(), &args, "command")?;
        let child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::tool(self.name(), e.to_string()))?;
        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| {
                Error::tool(
                    self.name(),
                    format!("timed out after {}s", self.timeout.as_secs()),
                )
            })?
            .map_err(|e| Error::tool(self.name(), e.to_string()))?;
        let code = output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".into());
        let mut text = format!("exit: {code}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stdout.trim().is_empty() {
            text.push_str("\nstdout:\n");
            text.push_str(stdout.trim_end());
        }
        if !stderr.trim().is_empty() {
            text.push_str("\nstderr:\n");
            text.push_str(stderr.trim_end());
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_in_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f"), "").unwrap();
        let tool = ShellTool::new(dir.path(), 5);
        let out = tool
            .call(json!({"command": "ls; echo oops >&2; exit 3"}))
            .await
            .unwrap();
        assert_eq!(out, "exit: 3\nstdout:\nf\nstderr:\noops");
    }

    #[tokio::test]
    async fn enforces_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let tool = ShellTool::new(dir.path(), 1);
        let err = tool.call(json!({"command": "sleep 5"})).await.unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }
}
