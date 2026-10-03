use std::path::PathBuf;

use crate::agent::{AgentOptions, ColibriAgent};
use crate::config::ColibriConfig;
use crate::error::{Error, Result};
use crate::memory::Memory;
use crate::tools::{ToolRegistry, ToolSettings};

pub const DEFAULT_DATABASE_URL: &str = "sqlite://aviary.db";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolAccess {
    Off,
    Read,
    Write,
}

impl ToolAccess {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "off" | "0" | "false" | "none" => Ok(ToolAccess::Off),
            "read" | "ro" | "1" | "true" => Ok(ToolAccess::Read),
            "write" | "rw" => Ok(ToolAccess::Write),
            other => Err(Error::Config(format!(
                "AVIARY_TOOLS must be off, read or write, got `{other}`"
            ))),
        }
    }
}

pub fn tools_from_env() -> Result<ToolRegistry> {
    let access = ToolAccess::parse(&std::env::var("AVIARY_TOOLS").unwrap_or_default())?;
    if access == ToolAccess::Off {
        return Ok(ToolRegistry::new());
    }
    let workspace = std::env::var("AVIARY_WORKSPACE")
        .ok()
        .filter(|w| !w.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| Error::Config("AVIARY_TOOLS needs AVIARY_WORKSPACE".into()))?;
    if !workspace.is_dir() {
        return Err(Error::Config(format!(
            "AVIARY_WORKSPACE {} is not a directory",
            workspace.display()
        )));
    }
    Ok(ToolRegistry::with_defaults(&ToolSettings {
        workspace,
        allow_write: access == ToolAccess::Write,
        allow_shell: false,
        ..Default::default()
    }))
}

pub async fn agent_from_env() -> Result<ColibriAgent> {
    let config = ColibriConfig::from_env()?;
    let url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_DATABASE_URL.to_string());
    let memory = Memory::open(&url).await?;
    let instructions = std::env::var("AVIARY_INSTRUCTIONS")
        .ok()
        .filter(|i| !i.trim().is_empty());
    ColibriAgent::builder(config)
        .memory(memory)
        .tools(tools_from_env()?)
        .options(AgentOptions {
            instructions,
            ..Default::default()
        })
        .build()
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tool_access() {
        assert_eq!(ToolAccess::parse("").unwrap(), ToolAccess::Off);
        assert_eq!(ToolAccess::parse("READ").unwrap(), ToolAccess::Read);
        assert_eq!(ToolAccess::parse("write").unwrap(), ToolAccess::Write);
        assert!(ToolAccess::parse("shell").is_err());
    }
}
