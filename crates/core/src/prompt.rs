use serde::Serialize;

use crate::types::{ChatMessage, ToolDefinition};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    Glm,
    DeepSeek,
    Kimi,
    Qwen,
    Inkling,
    Olmoe,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ModelProfile {
    pub family: ModelFamily,
    pub supports_tools: bool,
    pub supports_thinking: bool,
}

impl ModelProfile {
    pub fn detect(model_id: &str) -> Self {
        let id = model_id.to_ascii_lowercase();
        let family = if id.contains("glm") {
            ModelFamily::Glm
        } else if id.contains("deepseek") {
            ModelFamily::DeepSeek
        } else if id.contains("kimi") {
            ModelFamily::Kimi
        } else if id.contains("qwen") {
            ModelFamily::Qwen
        } else if id.contains("inkling") {
            ModelFamily::Inkling
        } else if id.contains("olmoe") {
            ModelFamily::Olmoe
        } else {
            ModelFamily::Unknown
        };
        let supports_tools = matches!(
            family,
            ModelFamily::Glm | ModelFamily::DeepSeek | ModelFamily::Kimi | ModelFamily::Unknown
        );
        let supports_thinking = matches!(
            family,
            ModelFamily::Glm | ModelFamily::DeepSeek | ModelFamily::Kimi | ModelFamily::Unknown
        );
        Self {
            family,
            supports_tools,
            supports_thinking,
        }
    }
}

const BASE_PROMPT: &str = "You are Aviary, a local assistant running on Colibri, a disk-streaming \
Mixture-of-Experts engine. Every prompt token costs real prefill time and every generated token \
costs roughly a second, so answer directly and keep replies short unless asked for detail. \
Do not repeat the question. Prefer one precise tool call over several speculative ones.";

const TOOL_RULES: &str = "Use a tool only when the answer depends on files, directories or \
command output you cannot know. After a tool result, answer from it without calling the same \
tool again with the same arguments.";

#[derive(Debug, Clone)]
pub struct PromptBuilder {
    profile: ModelProfile,
    context_tokens: usize,
    reply_tokens: usize,
    instructions: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BuiltPrompt {
    pub messages: Vec<ChatMessage>,
    pub dropped: usize,
    pub estimated_tokens: usize,
}

impl PromptBuilder {
    pub fn new(profile: ModelProfile, context_tokens: usize, reply_tokens: usize) -> Self {
        Self {
            profile,
            context_tokens,
            reply_tokens,
            instructions: None,
        }
    }

    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        let text = instructions.into();
        self.instructions = (!text.trim().is_empty()).then_some(text);
        self
    }

    pub fn profile(&self) -> ModelProfile {
        self.profile
    }

    pub fn system_prompt(&self, has_tools: bool) -> String {
        let mut prompt = String::from(BASE_PROMPT);
        if has_tools && self.profile.supports_tools {
            prompt.push(' ');
            prompt.push_str(TOOL_RULES);
        }
        if let Some(extra) = &self.instructions {
            prompt.push_str("\n\n");
            prompt.push_str(extra.trim());
        }
        prompt
    }

    pub fn budget(&self) -> usize {
        self.context_tokens.saturating_sub(self.reply_tokens)
    }

    pub fn build(
        &self,
        history: &[ChatMessage],
        turn: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> BuiltPrompt {
        let system = ChatMessage::system(self.system_prompt(!tools.is_empty()));
        let fixed = estimate_message(&system)
            + tools.iter().map(estimate_tool).sum::<usize>()
            + turn.iter().map(estimate_message).sum::<usize>();
        let budget = self.budget();
        let history_tokens: Vec<usize> = history.iter().map(estimate_message).collect();
        let total_history: usize = history_tokens.iter().sum();

        let mut start = 0;
        if fixed + total_history > budget {
            let target = budget.saturating_sub(fixed) * 3 / 4;
            let mut kept = total_history;
            while start < history.len() && kept > target {
                kept -= history_tokens[start];
                start += 1;
            }
            while start < history.len() && history[start].role != crate::types::Role::User {
                start += 1;
            }
        }

        let mut messages = Vec::with_capacity(1 + history.len() - start + turn.len());
        messages.push(system);
        messages.extend_from_slice(&history[start..]);
        messages.extend_from_slice(turn);
        let estimated_tokens = fixed + history_tokens[start..].iter().sum::<usize>();
        BuiltPrompt {
            messages,
            dropped: start,
            estimated_tokens,
        }
    }
}

pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(3)
}

fn estimate_message(message: &ChatMessage) -> usize {
    let calls: usize = message
        .tool_calls
        .iter()
        .map(|c| estimate_tokens(&c.function.name) + estimate_tokens(&c.function.arguments))
        .sum();
    4 + estimate_tokens(&message.content) + calls
}

fn estimate_tool(tool: &ToolDefinition) -> usize {
    8 + estimate_tokens(&tool.function.name)
        + estimate_tokens(&tool.function.description)
        + estimate_tokens(&tool.function.parameters.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Role;

    fn history(n: usize) -> Vec<ChatMessage> {
        (0..n)
            .map(|i| {
                if i % 2 == 0 {
                    ChatMessage::user(format!("question {i} {}", "x".repeat(300)))
                } else {
                    ChatMessage::assistant(format!("answer {i} {}", "y".repeat(300)))
                }
            })
            .collect()
    }

    #[test]
    fn detects_families() {
        assert_eq!(
            ModelProfile::detect("glm-5.2-colibri").family,
            ModelFamily::Glm
        );
        assert!(ModelProfile::detect("glm-5.2-colibri").supports_tools);
        assert!(!ModelProfile::detect("olmoe-1b-7b").supports_tools);
        assert!(!ModelProfile::detect("qwen3.8-flash-next").supports_tools);
        assert_eq!(ModelProfile::detect("kimi-k3").family, ModelFamily::Kimi);
    }

    #[test]
    fn system_prompt_is_stable() {
        let builder = PromptBuilder::new(ModelProfile::detect("glm-5.2"), 4096, 512);
        assert_eq!(builder.system_prompt(true), builder.system_prompt(true));
        assert!(builder.system_prompt(true).contains("tool"));
        assert!(!builder.system_prompt(false).contains("Use a tool"));
    }

    #[test]
    fn keeps_everything_when_it_fits() {
        let builder = PromptBuilder::new(ModelProfile::detect("glm-5.2"), 4096, 512);
        let built = builder.build(&history(4), &[ChatMessage::user("hi")], &[]);
        assert_eq!(built.dropped, 0);
        assert_eq!(built.messages.len(), 6);
        assert_eq!(built.messages[0].role, Role::System);
    }

    #[test]
    fn trims_oldest_and_starts_on_user_turn() {
        let builder = PromptBuilder::new(ModelProfile::detect("glm-5.2"), 900, 300);
        let built = builder.build(&history(10), &[ChatMessage::user("now")], &[]);
        assert!(built.dropped > 0);
        assert_eq!(built.messages[1].role, Role::User);
        assert!(built.estimated_tokens <= builder.budget());
        assert_eq!(built.messages.last().unwrap().content, "now");
    }

    #[test]
    fn instructions_are_appended() {
        let builder = PromptBuilder::new(ModelProfile::detect("glm-5.2"), 4096, 512)
            .with_instructions("Answer in German.");
        assert!(builder.system_prompt(false).ends_with("Answer in German."));
    }
}
