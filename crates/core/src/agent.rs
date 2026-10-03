use std::time::Instant;

use serde::Serialize;

use crate::cache::{CacheStats, ExpertCache, SlotLease};
use crate::colibri::ColibriClient;
use crate::config::ColibriConfig;
use crate::error::{Error, Result};
use crate::memory::Memory;
use crate::prompt::{ModelProfile, PromptBuilder};
use crate::telemetry::{ColibriStatus, RoutingStats};
use crate::tools::ToolRegistry;
use crate::types::{ChatMessage, ChatRequest, Completion, StreamEvent, Usage};

#[derive(Debug, Clone)]
pub struct AgentOptions {
    pub max_tool_rounds: usize,
    pub streaming: bool,
    pub track_routing: bool,
    pub instructions: Option<String>,
}

impl Default for AgentOptions {
    fn default() -> Self {
        Self {
            max_tool_rounds: 4,
            streaming: true,
            track_routing: true,
            instructions: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    Started {
        slot: usize,
        warm: bool,
        trimmed: usize,
    },
    Reasoning(String),
    Token(String),
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

#[derive(Debug, Clone, Serialize)]
pub struct AgentReply {
    pub content: String,
    pub reasoning: Option<String>,
    pub usage: Usage,
    pub tool_calls: usize,
    pub slot: usize,
    pub warm_slot: bool,
    pub routing: Option<RoutingStats>,
    pub elapsed_ms: u64,
    pub queue_wait_ms: Option<u64>,
}

impl AgentReply {
    pub fn tokens_per_sec(&self) -> Option<f64> {
        let secs = self.elapsed_ms as f64 / 1000.0;
        (secs > 0.0 && self.usage.completion_tokens > 0)
            .then(|| self.usage.completion_tokens as f64 / secs)
    }
}

pub struct AgentBuilder {
    config: ColibriConfig,
    memory: Option<Memory>,
    tools: ToolRegistry,
    options: AgentOptions,
}

impl AgentBuilder {
    pub fn memory(mut self, memory: Memory) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn tools(mut self, tools: ToolRegistry) -> Self {
        self.tools = tools;
        self
    }

    pub fn options(mut self, options: AgentOptions) -> Self {
        self.options = options;
        self
    }

    pub fn instructions(mut self, instructions: impl Into<String>) -> Self {
        self.options.instructions = Some(instructions.into());
        self
    }

    pub async fn build(self) -> Result<ColibriAgent> {
        let client = ColibriClient::new(self.config.clone())?;
        let memory = match self.memory {
            Some(memory) => memory,
            None => Memory::in_memory().await?,
        };
        let mut prompt = PromptBuilder::new(
            ModelProfile::detect(&self.config.model),
            self.config.context_tokens,
            self.config.max_tokens as usize,
        );
        if let Some(instructions) = &self.options.instructions {
            prompt = prompt.with_instructions(instructions.clone());
        }
        Ok(ColibriAgent {
            cache: ExpertCache::new(self.config.kv_slots),
            client,
            memory,
            tools: self.tools,
            prompt,
            options: self.options,
        })
    }
}

pub struct ColibriAgent {
    client: ColibriClient,
    memory: Memory,
    tools: ToolRegistry,
    cache: ExpertCache,
    prompt: PromptBuilder,
    options: AgentOptions,
}

impl ColibriAgent {
    pub fn builder(config: ColibriConfig) -> AgentBuilder {
        AgentBuilder {
            config,
            memory: None,
            tools: ToolRegistry::new(),
            options: AgentOptions::default(),
        }
    }

    pub fn client(&self) -> &ColibriClient {
        &self.client
    }

    pub fn config(&self) -> &ColibriConfig {
        self.client.config()
    }

    pub fn memory(&self) -> &Memory {
        &self.memory
    }

    pub fn tools(&self) -> &ToolRegistry {
        &self.tools
    }

    pub fn profile(&self) -> ModelProfile {
        self.prompt.profile()
    }

    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats()
    }

    pub async fn status(&self) -> ColibriStatus {
        self.client.status().await
    }

    pub async fn history(&self, user_id: &str) -> Result<Vec<ChatMessage>> {
        self.memory.recent(user_id).await
    }

    pub async fn reset(&self, user_id: &str) -> Result<u64> {
        self.cache.forget(user_id);
        self.memory.clear(user_id).await
    }

    pub async fn chat(&self, user_id: &str, input: &str) -> Result<AgentReply> {
        self.run(user_id, input, |_| {}).await
    }

    pub async fn run<F>(&self, user_id: &str, input: &str, mut on_event: F) -> Result<AgentReply>
    where
        F: FnMut(AgentEvent) + Send,
    {
        let started = Instant::now();
        let history = self.memory.recent(user_id).await?;
        let definitions = if self.prompt.profile().supports_tools {
            self.tools.definitions()
        } else {
            Vec::new()
        };
        let lease = self.cache.acquire(user_id);
        let mut turn = vec![ChatMessage::user(input)];
        let mut usage = Usage::default();
        let mut tool_calls = 0;
        let mut queue_wait_ms = None;

        for round in 0..=self.options.max_tool_rounds {
            let built = self.prompt.build(&history, &turn, &definitions);
            if round == 0 {
                on_event(AgentEvent::Started {
                    slot: lease.slot,
                    warm: lease.warm,
                    trimmed: built.dropped,
                });
            }
            let last_round = round == self.options.max_tool_rounds;
            let request = self.request(built.messages, &definitions, lease, last_round);
            let completion = self.generate(request, &mut on_event).await?;
            usage.add(&completion.usage);
            queue_wait_ms = queue_wait_ms.or(completion.queue_wait_ms);
            let message = completion.message;

            if message.tool_calls.is_empty() || last_round {
                if !message.tool_calls.is_empty() && message.content.trim().is_empty() {
                    return Err(Error::ToolLoop(self.options.max_tool_rounds));
                }
                let content = message.content.trim().to_string();
                self.memory.append_turn(user_id, input, &content).await?;
                let routing = self.observe_routing(lease.slot).await;
                return Ok(AgentReply {
                    content,
                    reasoning: message.reasoning_content,
                    usage,
                    tool_calls,
                    slot: lease.slot,
                    warm_slot: lease.warm,
                    routing,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    queue_wait_ms,
                });
            }

            let calls = message.tool_calls.clone();
            turn.push(ChatMessage::assistant_tool_calls(
                message.content,
                calls.clone(),
            ));
            for call in &calls {
                tool_calls += 1;
                on_event(AgentEvent::ToolCall {
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                });
                let outcome = self.tools.execute(call).await;
                on_event(AgentEvent::ToolResult {
                    name: outcome.name.clone(),
                    output: outcome.output.clone(),
                    ok: outcome.ok,
                });
                turn.push(ChatMessage::tool_result(outcome.call_id, outcome.output));
            }
        }

        Err(Error::ToolLoop(self.options.max_tool_rounds))
    }

    fn request(
        &self,
        messages: Vec<ChatMessage>,
        definitions: &[crate::types::ToolDefinition],
        lease: SlotLease,
        last_round: bool,
    ) -> ChatRequest {
        let config = self.config();
        let tools_active = !definitions.is_empty();
        ChatRequest {
            model: config.model.clone(),
            messages,
            tools: definitions.to_vec(),
            tool_choice: (tools_active && last_round).then(|| "none".to_string()),
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            stream: self.options.streaming,
            stream_options: None,
            cache_slot: (self.cache.slot_count() > 1).then_some(lease.slot),
            enable_thinking: config.thinking && self.prompt.profile().supports_thinking,
        }
    }

    async fn generate<F>(&self, request: ChatRequest, on_event: &mut F) -> Result<Completion>
    where
        F: FnMut(AgentEvent) + Send,
    {
        if !self.options.streaming {
            let completion = self.client.complete(request).await?;
            if let Some(reasoning) = &completion.message.reasoning_content {
                on_event(AgentEvent::Reasoning(reasoning.clone()));
            }
            if !completion.message.content.is_empty() {
                on_event(AgentEvent::Token(completion.message.content.clone()));
            }
            return Ok(completion);
        }
        self.client
            .stream(request, |event| match event {
                StreamEvent::Content(text) => on_event(AgentEvent::Token(text)),
                StreamEvent::Reasoning(text) => on_event(AgentEvent::Reasoning(text)),
                StreamEvent::Done(_) => {}
            })
            .await
    }

    async fn observe_routing(&self, slot: usize) -> Option<RoutingStats> {
        if !self.options.track_routing {
            return None;
        }
        let snapshot = self.client.experts().await.ok()?;
        let stats = snapshot.decode()?.stats();
        self.cache.record_routing(slot, &stats);
        Some(stats)
    }
}
