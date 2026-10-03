use crate::agent::AgentEvent;
use crate::error::Error;

pub fn split_message(text: &str, limit: usize) -> Vec<String> {
    let limit = limit.max(1);
    let mut chunks = Vec::new();
    let mut rest = text.trim();
    while rest.chars().count() > limit {
        let hard = rest
            .char_indices()
            .nth(limit)
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        let window = &rest[..hard];
        let cut = window
            .rfind("\n\n")
            .or_else(|| window.rfind('\n'))
            .or_else(|| window.rfind(' '))
            .filter(|&i| i > hard / 2)
            .unwrap_or(hard);
        chunks.push(rest[..cut].trim_end().to_string());
        rest = rest[cut..].trim_start();
    }
    if !rest.is_empty() || chunks.is_empty() {
        chunks.push(rest.to_string());
    }
    chunks
}

pub fn preview(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    let keep = limit.saturating_sub(3);
    let start = text
        .char_indices()
        .nth(count - keep)
        .map(|(i, _)| i)
        .unwrap_or(0);
    format!("...{}", &text[start..])
}

pub fn friendly_error(err: &Error) -> String {
    match err {
        Error::Busy(_) => "Colibri is busy with other requests. Try again in a moment.".to_string(),
        Error::Connect { .. } => "Colibri is not reachable right now.".to_string(),
        other => format!("Error: {other}"),
    }
}

#[derive(Debug, Default)]
pub struct LiveText {
    text: String,
    tools: Vec<String>,
}

impl LiveText {
    pub fn apply(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Token(t) => self.text.push_str(&t),
            AgentEvent::ToolCall { name, .. } => {
                self.text.clear();
                self.tools.push(name);
            }
            _ => {}
        }
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        for tool in &self.tools {
            out.push_str(&format!("[{tool}]\n"));
        }
        out.push_str(self.text.trim_start());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_text_shows_tools_then_answer() {
        let mut live = LiveText::default();
        live.apply(AgentEvent::Token("thinking out loud".into()));
        live.apply(AgentEvent::ToolCall {
            name: "read_file".into(),
            arguments: "{}".into(),
        });
        live.apply(AgentEvent::Token("The answer".into()));
        assert_eq!(live.render(), "[read_file]\nThe answer");
    }

    #[test]
    fn short_text_is_one_chunk() {
        assert_eq!(split_message("hello", 10), vec!["hello"]);
        assert_eq!(split_message("", 10), vec![""]);
    }

    #[test]
    fn splits_on_paragraphs_then_words() {
        let text = "aaaa bbbb\n\ncccc dddd eeee";
        assert_eq!(
            split_message(text, 12),
            vec!["aaaa bbbb", "cccc dddd", "eeee"]
        );
    }

    #[test]
    fn hard_splits_long_words_on_char_boundaries() {
        let text = "\u{fc}".repeat(25);
        let chunks = split_message(&text, 10);
        assert_eq!(chunks.len(), 3);
        assert!(chunks.iter().all(|c| c.chars().count() <= 10));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn preview_keeps_the_tail() {
        assert_eq!(preview("abcdef", 10), "abcdef");
        assert_eq!(preview("abcdefghij", 6), "...hij");
    }
}
