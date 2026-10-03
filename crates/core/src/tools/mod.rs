mod fs;
mod shell;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::types::{ToolCall, ToolDefinition};

pub use fs::{ListDirTool, ReadFileTool, WriteFileTool};
pub use shell::ShellTool;

pub const MAX_TOOL_OUTPUT: usize = 8 * 1024;

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    async fn call(&self, args: Value) -> Result<String>;

    fn definition(&self) -> ToolDefinition {
        ToolDefinition::function(self.name(), self.description(), self.parameters())
    }
}

#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.tools.keys()).finish()
    }
}

#[derive(Debug, Clone)]
pub struct ToolOutcome {
    pub call_id: String,
    pub name: String,
    pub output: String,
    pub ok: bool,
}

#[derive(Debug, Clone)]
pub struct ToolSettings {
    pub workspace: PathBuf,
    pub allow_write: bool,
    pub allow_shell: bool,
    pub shell_timeout_secs: u64,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            workspace: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            allow_write: false,
            allow_shell: false,
            shell_timeout_secs: 30,
        }
    }
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_defaults(settings: &ToolSettings) -> Self {
        let mut registry = Self::new();
        registry.register(ReadFileTool::new(&settings.workspace));
        registry.register(ListDirTool::new(&settings.workspace));
        if settings.allow_write {
            registry.register(WriteFileTool::new(&settings.workspace));
        }
        if settings.allow_shell {
            registry.register(ShellTool::new(
                &settings.workspace,
                settings.shell_timeout_secs,
            ));
        }
        registry
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) -> &mut Self {
        self.tools.insert(tool.name().to_string(), Arc::new(tool));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(String::as_str).collect()
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    pub async fn execute(&self, call: &ToolCall) -> ToolOutcome {
        let result = self.try_execute(call).await;
        let (output, ok) = match result {
            Ok(output) => (truncate(output), true),
            Err(err) => (format!("error: {err}"), false),
        };
        ToolOutcome {
            call_id: call.id.clone(),
            name: call.function.name.clone(),
            output,
            ok,
        }
    }

    async fn try_execute(&self, call: &ToolCall) -> Result<String> {
        let tool = self
            .tools
            .get(&call.function.name)
            .ok_or_else(|| Error::UnknownTool(call.function.name.clone()))?;
        let args = call.parsed_arguments().map_err(|e| {
            Error::tool(
                &call.function.name,
                format!("arguments are not valid JSON: {e}"),
            )
        })?;
        tool.call(args).await
    }
}

fn truncate(mut output: String) -> String {
    if output.len() > MAX_TOOL_OUTPUT {
        let mut cut = MAX_TOOL_OUTPUT;
        while !output.is_char_boundary(cut) {
            cut -= 1;
        }
        let total = output.len();
        output.truncate(cut);
        output.push_str(&format!("\n[truncated, {total} bytes total]"));
    }
    output
}

pub(crate) fn str_arg<'a>(tool: &str, args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::tool(tool, format!("missing string argument `{key}`")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Echo;

    #[async_trait]
    impl Tool for Echo {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echo text."
        }
        fn parameters(&self) -> Value {
            json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]})
        }
        async fn call(&self, args: Value) -> Result<String> {
            Ok(str_arg("echo", &args, "text")?.to_string())
        }
    }

    #[tokio::test]
    async fn executes_registered_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(Echo);
        let outcome = registry
            .execute(&ToolCall::new("1", "echo", r#"{"text":"hi"}"#))
            .await;
        assert!(outcome.ok);
        assert_eq!(outcome.output, "hi");
    }

    #[tokio::test]
    async fn reports_unknown_tool_and_bad_json() {
        let registry = ToolRegistry::new();
        let outcome = registry.execute(&ToolCall::new("1", "nope", "{}")).await;
        assert!(!outcome.ok);
        assert!(outcome.output.contains("unknown tool"));

        let mut registry = ToolRegistry::new();
        registry.register(Echo);
        let outcome = registry
            .execute(&ToolCall::new("1", "echo", "{not json"))
            .await;
        assert!(!outcome.ok);
        assert!(outcome.output.contains("not valid JSON"));
    }

    #[test]
    fn definitions_are_sorted_for_stable_prefix() {
        let settings = ToolSettings {
            allow_write: true,
            allow_shell: true,
            ..Default::default()
        };
        let registry = ToolRegistry::with_defaults(&settings);
        assert_eq!(
            registry.names(),
            vec!["list_dir", "read_file", "run_shell", "write_file"]
        );
    }

    #[test]
    fn truncates_long_output_on_char_boundary() {
        let text = "\u{e4}".repeat(MAX_TOOL_OUTPUT);
        let out = truncate(text);
        assert!(out.contains("[truncated"));
    }
}
