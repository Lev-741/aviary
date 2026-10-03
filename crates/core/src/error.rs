use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot reach Colibri at {url}: {source}")]
    Connect {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("HTTP error talking to Colibri: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Colibri returned {status}: {message}")]
    Api {
        status: u16,
        code: Option<String>,
        message: String,
    },

    #[error("Colibri queue is full or timed out: {0}")]
    Busy(String),

    #[error("invalid response from Colibri: {0}")]
    Protocol(String),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("memory error: {0}")]
    Memory(#[from] sqlx::Error),

    #[error("tool `{name}` failed: {message}")]
    Tool { name: String, message: String },

    #[error("unknown tool `{0}`")]
    UnknownTool(String),

    #[error("agent stopped after {0} tool rounds without a final answer")]
    ToolLoop(usize),

    #[error("invalid configuration: {0}")]
    Config(String),
}

impl Error {
    pub fn tool(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Tool {
            name: name.into(),
            message: message.into(),
        }
    }

    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Busy(_))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
