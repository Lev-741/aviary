use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const DEFAULT_BASE_URL: &str = "http://localhost:8000/v1";
pub const DEFAULT_MODEL: &str = "glm-5.2-colibri";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColibriConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub kv_slots: usize,
    pub request_timeout_secs: u64,
    pub max_tokens: u32,
    pub context_tokens: usize,
    pub temperature: Option<f32>,
    pub thinking: bool,
}

impl Default for ColibriConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
            api_key: None,
            kv_slots: 1,
            request_timeout_secs: 3600,
            max_tokens: 1024,
            context_tokens: 4096,
            temperature: None,
            thinking: false,
        }
    }
}

impl ColibriConfig {
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();
        if let Some(url) = env("COLIBRI_BASE_URL") {
            config.base_url = url;
        }
        if let Some(model) = env("COLIBRI_MODEL") {
            config.model = model;
        }
        config.api_key = env("COLIBRI_API_KEY");
        if let Some(slots) = env("COLIBRI_KV_SLOTS") {
            config.kv_slots = parse("COLIBRI_KV_SLOTS", &slots)?;
        }
        if let Some(timeout) = env("COLIBRI_TIMEOUT_SECS") {
            config.request_timeout_secs = parse("COLIBRI_TIMEOUT_SECS", &timeout)?;
        }
        if let Some(max) = env("COLIBRI_MAX_TOKENS") {
            config.max_tokens = parse("COLIBRI_MAX_TOKENS", &max)?;
        }
        if let Some(ctx) = env("COLIBRI_CONTEXT_TOKENS") {
            config.context_tokens = parse("COLIBRI_CONTEXT_TOKENS", &ctx)?;
        }
        if let Some(temp) = env("COLIBRI_TEMPERATURE") {
            config.temperature = Some(parse("COLIBRI_TEMPERATURE", &temp)?);
        }
        if let Some(thinking) = env("COLIBRI_THINKING") {
            config.thinking = matches!(thinking.as_str(), "1" | "true" | "yes" | "on");
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if !(self.base_url.starts_with("http://") || self.base_url.starts_with("https://")) {
            return Err(Error::Config(format!(
                "base URL must start with http:// or https://, got `{}`",
                self.base_url
            )));
        }
        if self.model.trim().is_empty() {
            return Err(Error::Config("model id must not be empty".into()));
        }
        if !(1..=16).contains(&self.kv_slots) {
            return Err(Error::Config(format!(
                "Colibri supports 1 to 16 KV slots, got {}",
                self.kv_slots
            )));
        }
        if self.context_tokens <= self.max_tokens as usize {
            return Err(Error::Config(format!(
                "context window ({}) must be larger than max_tokens ({})",
                self.context_tokens, self.max_tokens
            )));
        }
        Ok(())
    }

    pub fn api_base(&self) -> &str {
        self.base_url.trim_end_matches('/')
    }

    pub fn server_root(&self) -> &str {
        let base = self.api_base();
        base.strip_suffix("/v1").unwrap_or(base)
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.request_timeout_secs)
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn parse<T: std::str::FromStr>(key: &str, value: &str) -> Result<T> {
    value
        .trim()
        .parse()
        .map_err(|_| Error::Config(format!("{key} has an invalid value: `{value}`")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_root_strips_v1() {
        let config = ColibriConfig {
            base_url: "http://10.0.0.2:8000/v1/".into(),
            ..Default::default()
        };
        assert_eq!(config.api_base(), "http://10.0.0.2:8000/v1");
        assert_eq!(config.server_root(), "http://10.0.0.2:8000");
    }

    #[test]
    fn rejects_bad_slot_count() {
        let config = ColibriConfig {
            kv_slots: 17,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_non_http_url() {
        let config = ColibriConfig {
            base_url: "localhost:8000".into(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
