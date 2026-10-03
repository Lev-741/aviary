use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use aviary_core::agent::{AgentEvent, AgentOptions, AgentReply};
use aviary_core::memory::StoredMessage;
use aviary_core::telemetry::ExpertSnapshot;
use aviary_core::{
    CacheStats, ColibriAgent, ColibriClient, ColibriConfig, ColibriStatus, Conversation, Memory,
    ToolRegistry, ToolSettings,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;

const CONVERSATION_PREFIX: &str = "desktop:";
const CHAT_EVENT: &str = "aviary://chat";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub colibri: ColibriConfig,
    pub instructions: String,
    pub tools_enabled: bool,
    pub workspace: Option<String>,
    pub allow_write: bool,
    pub allow_shell: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            colibri: ColibriConfig::from_env().unwrap_or_default(),
            instructions: String::new(),
            tools_enabled: true,
            workspace: std::env::var("HOME").ok(),
            allow_write: false,
            allow_shell: false,
        }
    }
}

impl Settings {
    fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    fn tools(&self) -> ToolRegistry {
        if !self.tools_enabled {
            return ToolRegistry::new();
        }
        let mut settings = ToolSettings {
            allow_write: self.allow_write,
            allow_shell: self.allow_shell,
            ..Default::default()
        };
        if let Some(dir) = self.workspace.as_deref().filter(|d| !d.trim().is_empty()) {
            settings.workspace = PathBuf::from(dir);
        }
        ToolRegistry::with_defaults(&settings)
    }
}

struct AppState {
    agent: RwLock<Arc<ColibriAgent>>,
    settings: RwLock<Settings>,
    settings_path: PathBuf,
    db_url: String,
    running: Mutex<HashMap<String, tokio::task::AbortHandle>>,
}

async fn build_agent(settings: &Settings, db_url: &str) -> Result<ColibriAgent, String> {
    let memory = Memory::open(db_url).await.map_err(|e| e.to_string())?;
    let instructions = settings.instructions.trim();
    ColibriAgent::builder(settings.colibri.clone())
        .memory(memory)
        .tools(settings.tools())
        .options(AgentOptions {
            instructions: (!instructions.is_empty()).then(|| instructions.to_string()),
            ..Default::default()
        })
        .build()
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChatEvent {
    Started {
        slot: usize,
        warm: bool,
        trimmed: usize,
    },
    Token {
        text: String,
    },
    Reasoning {
        text: String,
    },
    ToolCall {
        name: String,
        arguments: String,
    },
    ToolResult {
        name: String,
        output: String,
        ok: bool,
    },
}

impl From<AgentEvent> for ChatEvent {
    fn from(event: AgentEvent) -> Self {
        match event {
            AgentEvent::Started {
                slot,
                warm,
                trimmed,
            } => ChatEvent::Started {
                slot,
                warm,
                trimmed,
            },
            AgentEvent::Token(text) => ChatEvent::Token { text },
            AgentEvent::Reasoning(text) => ChatEvent::Reasoning { text },
            AgentEvent::ToolCall { name, arguments } => ChatEvent::ToolCall { name, arguments },
            AgentEvent::ToolResult { name, output, ok } => {
                ChatEvent::ToolResult { name, output, ok }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct ChatEnvelope {
    conversation: String,
    #[serde(flatten)]
    event: ChatEvent,
}

#[derive(Debug, Serialize)]
struct StatusPayload {
    status: ColibriStatus,
    cache: CacheStats,
}

fn conversation_id(id: &str) -> Result<String, String> {
    if id.starts_with(CONVERSATION_PREFIX) && id.len() > CONVERSATION_PREFIX.len() {
        Ok(id.to_string())
    } else {
        Err(format!("invalid conversation id `{id}`"))
    }
}

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(state.settings.read().await.clone())
}

#[tauri::command]
async fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<Settings, String> {
    settings.colibri.validate().map_err(|e| e.to_string())?;
    let agent = build_agent(&settings, &state.db_url).await?;
    settings.save(&state.settings_path)?;
    *state.agent.write().await = Arc::new(agent);
    *state.settings.write().await = settings.clone();
    Ok(settings)
}

#[tauri::command]
async fn probe_models(base_url: String, api_key: Option<String>) -> Result<Vec<String>, String> {
    let config = ColibriConfig {
        base_url,
        api_key: api_key.filter(|k| !k.trim().is_empty()),
        ..Default::default()
    };
    let client = ColibriClient::new(config).map_err(|e| e.to_string())?;
    let models = client.models().await.map_err(|e| e.to_string())?;
    Ok(models.into_iter().map(|m| m.id).collect())
}

#[tauri::command]
async fn get_status(state: State<'_, AppState>) -> Result<StatusPayload, String> {
    let agent = state.agent.read().await.clone();
    Ok(StatusPayload {
        status: agent.status().await,
        cache: agent.cache_stats(),
    })
}

#[tauri::command]
async fn get_experts(state: State<'_, AppState>) -> Result<ExpertSnapshot, String> {
    let agent = state.agent.read().await.clone();
    agent.client().experts().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_conversations(state: State<'_, AppState>) -> Result<Vec<Conversation>, String> {
    let agent = state.agent.read().await.clone();
    agent
        .memory()
        .conversations(CONVERSATION_PREFIX)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_history(
    state: State<'_, AppState>,
    conversation: String,
) -> Result<Vec<StoredMessage>, String> {
    let id = conversation_id(&conversation)?;
    let agent = state.agent.read().await.clone();
    agent.memory().stored(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_conversation(
    state: State<'_, AppState>,
    conversation: String,
) -> Result<u64, String> {
    let id = conversation_id(&conversation)?;
    let agent = state.agent.read().await.clone();
    agent.reset(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation: String,
    text: String,
) -> Result<AgentReply, String> {
    let id = conversation_id(&conversation)?;
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("message is empty".into());
    }
    let agent = state.agent.read().await.clone();
    let emitter = app.clone();
    let event_id = id.clone();
    let task = tokio::spawn(async move {
        agent
            .run(&event_id, &text, |event| {
                let envelope = ChatEnvelope {
                    conversation: event_id.clone(),
                    event: event.into(),
                };
                let _ = emitter.emit(CHAT_EVENT, envelope);
            })
            .await
    });
    {
        let mut running = state.running.lock().map_err(|e| e.to_string())?;
        if let Some(previous) = running.insert(id.clone(), task.abort_handle()) {
            previous.abort();
        }
    }
    let result = task.await;
    if let Ok(mut running) = state.running.lock() {
        running.remove(&id);
    }
    match result {
        Ok(reply) => reply.map_err(|e| e.to_string()),
        Err(err) if err.is_cancelled() => Err("cancelled".into()),
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
async fn cancel_message(state: State<'_, AppState>, conversation: String) -> Result<bool, String> {
    let running = state.running.lock().map_err(|e| e.to_string())?;
    Ok(match running.get(&conversation) {
        Some(handle) => {
            handle.abort();
            true
        }
        None => false,
    })
}

pub fn run() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    tauri::Builder::default()
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let settings_path = config_dir.join("settings.json");
            let settings = Settings::load(&settings_path);
            let db_url = format!("sqlite://{}", data_dir.join("aviary.db").display());
            let agent = tauri::async_runtime::block_on(async {
                match build_agent(&settings, &db_url).await {
                    Ok(agent) => Ok(agent),
                    Err(err) => {
                        tracing::warn!("settings rejected, using defaults: {err}");
                        build_agent(&Settings::default(), &db_url).await
                    }
                }
            })?;
            app.manage(AppState {
                agent: RwLock::new(Arc::new(agent)),
                settings: RwLock::new(settings),
                settings_path,
                db_url,
                running: Mutex::new(HashMap::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            probe_models,
            get_status,
            get_experts,
            list_conversations,
            get_history,
            delete_conversation,
            send_message,
            cancel_message,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Aviary desktop");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_ids_are_namespaced() {
        assert!(conversation_id("desktop:123").is_ok());
        assert!(conversation_id("desktop:").is_err());
        assert!(conversation_id("tg:1").is_err());
    }

    #[test]
    fn settings_round_trip() {
        let dir = std::env::temp_dir().join(format!("aviary-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        let mut settings = Settings::default();
        settings.colibri.model = "glm-5.2-colibri".into();
        settings.allow_shell = true;
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.colibri, settings.colibri);
        assert!(loaded.allow_shell);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: Settings =
            serde_json::from_str(r#"{"colibri": {"base_url": "http://nas:8000/v1"}}"#).unwrap();
        assert_eq!(settings.colibri.base_url, "http://nas:8000/v1");
        assert_eq!(settings.colibri.model, "glm-5.2-colibri");
        assert!(settings.tools_enabled);
    }

    #[test]
    fn chat_event_serializes_flat() {
        let envelope = ChatEnvelope {
            conversation: "desktop:1".into(),
            event: ChatEvent::Token { text: "hi".into() },
        };
        let value = serde_json::to_value(envelope).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"conversation": "desktop:1", "kind": "token", "text": "hi"})
        );
    }
}
