use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use aviary_core::ColibriAgent;
use aviary_core::queue::TurnQueue;
use aviary_core::text::{LiveText, friendly_error, preview, split_message};
use teloxide::prelude::*;
use teloxide::types::{ChatAction, Me, MessageId, ReplyParameters};
use teloxide::utils::command::BotCommands;
use tokio::sync::mpsc;

const MESSAGE_LIMIT: usize = 4096;
const PLACEHOLDER: &str = "...";
const QUEUED: &str = "Queued after your previous message.";

#[derive(Debug, Clone)]
pub struct BotConfig {
    pub token: String,
    pub allowed_users: HashSet<u64>,
    pub edit_interval: Duration,
    pub api_url: Option<String>,
}

impl BotConfig {
    pub fn from_env() -> Result<Self> {
        let token = std::env::var("TELEGRAM_BOT_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty())
            .context("TELEGRAM_BOT_TOKEN is not set")?;
        let allowed_users =
            parse_ids(&std::env::var("TELEGRAM_ALLOWED_USERS").unwrap_or_default())?;
        let edit_interval = std::env::var("TELEGRAM_EDIT_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_millis(2500));
        let api_url = std::env::var("TELEGRAM_API_URL")
            .ok()
            .filter(|u| !u.trim().is_empty());
        Ok(Self {
            token,
            allowed_users,
            edit_interval,
            api_url,
        })
    }
}

pub fn parse_ids(value: &str) -> Result<HashSet<u64>> {
    value
        .split([',', ' '])
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.parse::<u64>()
                .with_context(|| format!("invalid user id `{v}`"))
        })
        .collect()
}

#[derive(BotCommands, Clone, Debug, PartialEq)]
#[command(
    rename_rule = "lowercase",
    description = "Aviary talks to a local Colibri model."
)]
pub enum Command {
    #[command(description = "show this help")]
    Help,
    #[command(description = "introduction")]
    Start,
    #[command(description = "Colibri status: RAM, experts, NVMe streaming")]
    Status,
    #[command(description = "forget this conversation")]
    Reset,
}

struct Shared {
    agent: Arc<ColibriAgent>,
    config: BotConfig,
    queue: TurnQueue,
}

impl Shared {
    fn allowed(&self, user: Option<&teloxide::types::User>) -> bool {
        self.config.allowed_users.is_empty()
            || user.is_some_and(|u| self.config.allowed_users.contains(&u.id.0))
    }
}

pub fn memory_key(user_id: u64) -> String {
    format!("tg:{user_id}")
}

pub async fn run(config: BotConfig, agent: ColibriAgent) -> Result<()> {
    let mut bot = Bot::new(&config.token);
    if let Some(url) = &config.api_url {
        bot = bot.set_api_url(url.parse().context("TELEGRAM_API_URL is not a valid URL")?);
    }
    bot.set_my_commands(Command::bot_commands())
        .await
        .context("cannot register bot commands, is TELEGRAM_BOT_TOKEN valid?")?;
    let shared = Arc::new(Shared {
        agent: Arc::new(agent),
        config,
        queue: TurnQueue::new(),
    });

    let handler = Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(on_command),
        )
        .branch(dptree::endpoint(on_message));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![shared])
        .default_handler(|_| async {})
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
    Ok(())
}

async fn on_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    shared: Arc<Shared>,
) -> ResponseResult<()> {
    if !shared.allowed(msg.from.as_ref()) {
        return Ok(());
    }
    let text = match cmd {
        Command::Help => Command::descriptions().to_string(),
        Command::Start => format!(
            "Hi. I answer with {} running locally on Colibri. Answers stream in as the model generates them, \
             which can take a while when experts are loaded from NVMe. Use /status to see what the server is doing.",
            shared.agent.config().model
        ),
        Command::Status => shared.agent.status().await.summary(),
        Command::Reset => match msg.from.as_ref() {
            Some(user) => match shared.agent.reset(&memory_key(user.id.0)).await {
                Ok(n) => format!("Forgot {n} messages."),
                Err(err) => format!("Could not reset: {err}"),
            },
            None => "No user on this message.".to_string(),
        },
    };
    bot.send_message(msg.chat.id, text)
        .reply_parameters(ReplyParameters::new(msg.id))
        .await?;
    Ok(())
}

pub fn extract_prompt(
    text: &str,
    bot_username: &str,
    private: bool,
    reply_to_bot: bool,
) -> Option<String> {
    let mention = format!("@{bot_username}");
    let mentioned = !bot_username.is_empty() && text.contains(&mention);
    if !(private || reply_to_bot || mentioned) {
        return None;
    }
    let cleaned = if mentioned {
        text.replace(&mention, " ")
    } else {
        text.to_string()
    };
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned = if cleaned.is_empty() || text.contains('\n') {
        text.replace(&mention, "").trim().to_string()
    } else {
        cleaned
    };
    (!cleaned.is_empty()).then_some(cleaned)
}

async fn on_message(bot: Bot, msg: Message, me: Me, shared: Arc<Shared>) -> ResponseResult<()> {
    let Some(text) = msg.text() else {
        return Ok(());
    };
    let Some(user) = msg.from.as_ref() else {
        return Ok(());
    };
    if !shared.allowed(Some(user)) {
        if msg.chat.is_private() {
            bot.send_message(msg.chat.id, "This bot is private.")
                .await?;
        }
        return Ok(());
    }
    let reply_to_bot = msg
        .reply_to_message()
        .and_then(|r| r.from.as_ref())
        .is_some_and(|u| u.id == me.id);
    let username = me.username.clone().unwrap_or_default();
    let Some(prompt) = extract_prompt(text, &username, msg.chat.is_private(), reply_to_bot) else {
        return Ok(());
    };

    let key = memory_key(user.id.0);
    let queued = shared.queue.is_busy(&key);
    let placeholder = bot
        .send_message(msg.chat.id, if queued { QUEUED } else { PLACEHOLDER })
        .reply_parameters(ReplyParameters::new(msg.id))
        .await?;
    let _turn = shared.queue.acquire(&key).await;
    answer(&bot, msg.chat.id, placeholder.id, &shared, &key, &prompt).await
}

async fn answer(
    bot: &Bot,
    chat: ChatId,
    placeholder: MessageId,
    shared: &Arc<Shared>,
    key: &str,
    prompt: &str,
) -> ResponseResult<()> {
    let typing = {
        let bot = bot.clone();
        tokio::spawn(async move {
            loop {
                let _ = bot.send_chat_action(chat, ChatAction::Typing).await;
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        })
    };

    let (tx, mut rx) = mpsc::unbounded_channel();
    let agent = shared.agent.clone();
    let run_key = key.to_string();
    let run_prompt = prompt.to_string();
    let mut task = tokio::spawn(async move {
        agent
            .run(&run_key, &run_prompt, move |event| {
                let _ = tx.send(event);
            })
            .await
    });

    let mut live = LiveText::default();
    let mut shown = String::from(PLACEHOLDER);
    let mut ticker = tokio::time::interval(shared.config.edit_interval);
    ticker.tick().await;
    let outcome = loop {
        tokio::select! {
            Some(event) = rx.recv() => live.apply(event),
            _ = ticker.tick() => {
                let text = live.render();
                if !text.is_empty() && text != shown {
                    let _ = bot.edit_message_text(chat, placeholder, preview(&text, MESSAGE_LIMIT)).await;
                    shown = text;
                }
            }
            result = &mut task => break result,
        }
    };
    typing.abort();

    let final_text = match outcome {
        Ok(Ok(reply)) if reply.content.trim().is_empty() => "(empty answer)".to_string(),
        Ok(Ok(reply)) => reply.content,
        Ok(Err(err)) => friendly_error(&err),
        Err(err) => format!("Internal error: {err}"),
    };
    deliver(bot, chat, placeholder, &final_text).await
}

async fn deliver(bot: &Bot, chat: ChatId, first: MessageId, text: &str) -> ResponseResult<()> {
    let mut chunks = split_message(text, MESSAGE_LIMIT).into_iter();
    if let Some(head) = chunks.next() {
        let _ = bot.edit_message_text(chat, first, head).await;
    }
    for chunk in chunks {
        bot.send_message(chat, chunk).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_allowed_ids() {
        let ids = parse_ids("12, 34 56").unwrap();
        assert_eq!(ids, HashSet::from([12, 34, 56]));
        assert!(parse_ids("").unwrap().is_empty());
        assert!(parse_ids("abc").is_err());
    }

    #[test]
    fn private_chats_always_answer() {
        assert_eq!(
            extract_prompt("hello", "aviary_bot", true, false).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn groups_need_mention_or_reply() {
        assert_eq!(extract_prompt("hello", "aviary_bot", false, false), None);
        assert_eq!(
            extract_prompt("@aviary_bot how fast is it", "aviary_bot", false, false).as_deref(),
            Some("how fast is it")
        );
        assert_eq!(
            extract_prompt("go on", "aviary_bot", false, true).as_deref(),
            Some("go on")
        );
        assert_eq!(
            extract_prompt("@aviary_bot", "aviary_bot", false, false),
            None
        );
    }

    #[test]
    fn multiline_prompts_keep_newlines() {
        let prompt = extract_prompt(
            "@aviary_bot fix this:\nline 1\nline 2",
            "aviary_bot",
            false,
            false,
        )
        .unwrap();
        assert_eq!(prompt, "fix this:\nline 1\nline 2");
    }

    #[test]
    fn commands_parse() {
        assert_eq!(
            Command::parse("/status", "aviary_bot").unwrap(),
            Command::Status
        );
        assert_eq!(
            Command::parse("/reset@aviary_bot", "aviary_bot").unwrap(),
            Command::Reset
        );
    }

    #[test]
    fn memory_keys_are_namespaced() {
        assert_eq!(memory_key(42), "tg:42");
    }
}
