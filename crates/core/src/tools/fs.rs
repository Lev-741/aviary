use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Tool, str_arg};
use crate::error::{Error, Result};

const MAX_READ_BYTES: u64 = 256 * 1024;
const MAX_LIST_ENTRIES: usize = 200;

#[derive(Debug, Clone)]
struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(root: &Path) -> Self {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        Self { root }
    }

    fn resolve(&self, tool: &str, requested: &str) -> Result<PathBuf> {
        let requested = Path::new(requested.trim());
        let joined = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.root.join(requested)
        };
        let normalized = normalize(&joined);
        let real = normalized.canonicalize().ok().or_else(|| {
            let parent = normalized.parent()?.canonicalize().ok()?;
            Some(parent.join(normalized.file_name()?))
        });
        let outside = || {
            Error::tool(
                tool,
                format!("{} is outside the workspace", requested.display()),
            )
        };
        match real {
            Some(path) if path.starts_with(&self.root) => Ok(path),
            Some(_) => Err(outside()),
            None if !normalized.starts_with(&self.root) => Err(outside()),
            None => Err(Error::tool(
                tool,
                format!("{} does not exist", requested.display()),
            )),
        }
    }

    fn display(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .map(|p| {
                if p.as_os_str().is_empty() {
                    ".".into()
                } else {
                    p.display().to_string()
                }
            })
            .unwrap_or_else(|_| path.display().to_string())
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

pub struct ReadFileTool {
    sandbox: Sandbox,
}

impl ReadFileTool {
    pub fn new(workspace: &Path) -> Self {
        Self {
            sandbox: Sandbox::new(workspace),
        }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 text file from the workspace."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"path": {"type": "string", "description": "Path relative to the workspace"}},
            "required": ["path"]
        })
    }

    async fn call(&self, args: Value) -> Result<String> {
        let path = self
            .sandbox
            .resolve(self.name(), str_arg(self.name(), &args, "path")?)?;
        let meta = tokio::fs::metadata(&path).await.map_err(|e| {
            Error::tool(self.name(), format!("{}: {e}", self.sandbox.display(&path)))
        })?;
        if !meta.is_file() {
            return Err(Error::tool(self.name(), "path is not a file"));
        }
        if meta.len() > MAX_READ_BYTES {
            return Err(Error::tool(
                self.name(),
                format!("file is {} bytes, limit is {MAX_READ_BYTES}", meta.len()),
            ));
        }
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|e| Error::tool(self.name(), e.to_string()))?;
        String::from_utf8(bytes).map_err(|_| Error::tool(self.name(), "file is not valid UTF-8"))
    }
}

pub struct ListDirTool {
    sandbox: Sandbox,
}

impl ListDirTool {
    pub fn new(workspace: &Path) -> Self {
        Self {
            sandbox: Sandbox::new(workspace),
        }
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List entries of a workspace directory. Directories end with a slash."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"path": {"type": "string", "description": "Directory relative to the workspace, default ."}}
        })
    }

    async fn call(&self, args: Value) -> Result<String> {
        let requested = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = self.sandbox.resolve(self.name(), requested)?;
        let mut reader = tokio::fs::read_dir(&path).await.map_err(|e| {
            Error::tool(self.name(), format!("{}: {e}", self.sandbox.display(&path)))
        })?;
        let mut entries = Vec::new();
        while let Some(entry) = reader
            .next_entry()
            .await
            .map_err(|e| Error::tool(self.name(), e.to_string()))?
        {
            let mut name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                name.push('/');
            }
            entries.push(name);
        }
        entries.sort();
        let total = entries.len();
        entries.truncate(MAX_LIST_ENTRIES);
        let mut out = entries.join("\n");
        if total > MAX_LIST_ENTRIES {
            out.push_str(&format!("\n[{} more entries]", total - MAX_LIST_ENTRIES));
        }
        if out.is_empty() {
            out.push_str("(empty)");
        }
        Ok(out)
    }
}

pub struct WriteFileTool {
    sandbox: Sandbox,
}

impl WriteFileTool {
    pub fn new(workspace: &Path) -> Self {
        Self {
            sandbox: Sandbox::new(workspace),
        }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Create or overwrite a text file in the workspace."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "Path relative to the workspace"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"]
        })
    }

    async fn call(&self, args: Value) -> Result<String> {
        let path = self
            .sandbox
            .resolve(self.name(), str_arg(self.name(), &args, "path")?)?;
        let content = str_arg(self.name(), &args, "content")?;
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| Error::tool(self.name(), e.to_string()))?;
        Ok(format!(
            "wrote {} bytes to {}",
            content.len(),
            self.sandbox.display(&path)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_and_lists_inside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let read = ReadFileTool::new(dir.path());
        assert_eq!(read.call(json!({"path": "a.txt"})).await.unwrap(), "hello");

        let list = ListDirTool::new(dir.path());
        assert_eq!(list.call(json!({})).await.unwrap(), "a.txt\nsub/");
    }

    #[tokio::test]
    async fn blocks_escape_from_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let read = ReadFileTool::new(dir.path());
        let err = read
            .call(json!({"path": "../../etc/passwd"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("outside the workspace"));
        let err = read.call(json!({"path": "/etc/passwd"})).await.unwrap_err();
        assert!(err.to_string().contains("outside the workspace"));
    }

    #[tokio::test]
    async fn missing_paths_outside_are_still_outside() {
        let dir = tempfile::tempdir().unwrap();
        let read = ReadFileTool::new(dir.path());
        let err = read
            .call(json!({"path": "../../no/such/dir/file"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("outside the workspace"));
        let err = read
            .call(json!({"path": "missing/deeper/file"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn blocks_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/etc", dir.path().join("link")).unwrap();
        let read = ReadFileTool::new(dir.path());
        let err = read
            .call(json!({"path": "link/hostname"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("outside the workspace"));
    }

    #[tokio::test]
    async fn writes_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let write = WriteFileTool::new(dir.path());
        let msg = write
            .call(json!({"path": "notes.md", "content": "abc"}))
            .await
            .unwrap();
        assert_eq!(msg, "wrote 3 bytes to notes.md");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("notes.md")).unwrap(),
            "abc"
        );
    }
}
