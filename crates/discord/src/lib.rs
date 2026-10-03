use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use aviary_core::ColibriAgent;
use aviary_core::queue::TurnQueue;
use aviary_core::text::{LiveText, friendly_error, preview, split_message};
use serenity::all::{
    Client, Context, CreateMessage, EditMessage, EventHandler, GatewayIntents, Message, Ready,
    UserId,
};
use serenity::async_trait;
use tokio::sync::mpsc;

const MESSAGE_LIMIT: usize = 2000;
const PLACEHOLDER: &str = "...";
const QUEUED: &str = "Queued after your previous message.";

#[derive(Debug, Clone)]
pub struct BotConfig {
    pub token: String,
    pub prefix: String,
    pub allowed_users: HashSet<u64>,
    pub allowed_channels: HashSet<u64>,
    pub edit_interval: Duration,
}

impl BotConfig {
    pub fn from_env() -> Result<Self> {
        let token = std::env::var("DISCORD_BOT_TOKEN")
            .ok()
            .filter(|t| !t.trim().is_empty())
            .context("DISCORD_BOT_TOKEN is not set")?;
        let prefix = std::env::var("DISCORD_PREFIX")
            .ok()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| "!".to_string());
        Ok(Self {
            token,
            prefix,
            allowed_users: parse_ids(&std::env::var("DISCORD_ALLOWED_USERS").unwrap_or_default())?,
            allowed_channels: parse_ids(
                &std::env::var("DISCORD_ALLOWED_CHANNELS").unwrap_or_default(),
            )?,
            edit_interval: std::env::var("DISCORD_EDIT_INTERVAL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(Duration::from_millis)
                .unwrap_or(Duration::from_millis(2000)),
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
                .with_context(|| format!("invalid id `{v}`"))
        })
        .collect()
}

pub fn memory_key(user_id: u64) -> String {
    format!("dc:{user_id}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Ignore,
    Help,
    Status,
    Reset,
    Prompt(String),
}

pub fn parse_input(
    content: &str,
    bot_id: u64,
    direct: bool,
    mentioned: bool,
    prefix: &str,
) -> Input {
    let mut text = content.to_string();
    for mention in [format!("<@{bot_id}>"), format!("<@!{bot_id}>")] {
        text = text.replace(&mention, "");
    }
    let text = text.trim();

    if let Some(rest) = text.strip_prefix(prefix) {
        let (cmd, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let arg = arg.trim();
        return match cmd.to_ascii_lowercase().as_str() {
            "help" => Input::Help,
            "status" => Input::Status,
            "reset" => Input::Reset,
            "ask" if !arg.is_empty() => Input::Prompt(arg.to_string()),
            _ => Input::Ignore,
        };
    }
    if (direct || mentioned) && !text.is_empty() {
        Input::Prompt(text.to_string())
    } else {
        Input::Ignore
    }
}

fn help(prefix: &str, model: &str) -> String {
    format!(
        "I answer with {model} running locally on Colibri. Mention me or send a DM to ask something.\n\
         `{prefix}ask <question>` ask in a channel\n\
         `{prefix}status` Colibri status: RAM, experts, NVMe streaming\n\
         `{prefix}reset` forget our conversation\n\
         `{prefix}help` this message"
    )
}

struct Handler {
    agent: Arc<ColibriAgent>,
    config: BotConfig,
    queue: TurnQueue,
}

impl Handler {
    fn allowed(&self, msg: &Message) -> bool {
        let user_ok = self.config.allowed_users.is_empty()
            || self.config.allowed_users.contains(&msg.author.id.get());
        let channel_ok = msg.guild_id.is_none()
            || self.config.allowed_channels.is_empty()
            || self.config.allowed_channels.contains(&msg.channel_id.get());
        user_ok && channel_ok
    }

    async fn reply(&self, ctx: &Context, msg: &Message, text: &str) {
        for chunk in split_message(text, MESSAGE_LIMIT) {
            if let Err(err) = msg.reply(&ctx.http, chunk).await {
                tracing::warn!("cannot reply: {err}");
                return;
            }
        }
    }

    async fn answer(&self, ctx: &Context, msg: &Message, prompt: String) -> Result<()> {
        let key = memory_key(msg.author.id.get());
        let queued = self.queue.is_busy(&key);
        let mut placeholder = msg
            .reply(&ctx.http, if queued { QUEUED } else { PLACEHOLDER })
            .await?;
        let _turn = self.queue.acquire(&key).await;
        let typing = msg.channel_id.start_typing(&ctx.http);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let agent = self.agent.clone();
        let run_key = key.clone();
        let mut task = tokio::spawn(async move {
            agent
                .run(&run_key, &prompt, move |event| {
                    let _ = tx.send(event);
                })
                .await
        });

        let mut live = LiveText::default();
        let mut shown = placeholder.content.clone();
        let mut ticker = tokio::time::interval(self.config.edit_interval);
        ticker.tick().await;
        let outcome = loop {
            tokio::select! {
                Some(event) = rx.recv() => live.apply(event),
                _ = ticker.tick() => {
                    let text = live.render();
                    if !text.is_empty() && text != shown {
                        let edit = EditMessage::new().content(preview(&text, MESSAGE_LIMIT));
                        let _ = placeholder.edit(&ctx.http, edit).await;
                        shown = text;
                    }
                }
                result = &mut task => break result,
            }
        };
        drop(typing);

        let final_text = match outcome {
            Ok(Ok(reply)) if reply.content.trim().is_empty() => "(empty answer)".to_string(),
            Ok(Ok(reply)) => reply.content,
            Ok(Err(err)) => friendly_error(&err),
            Err(err) => format!("Internal error: {err}"),
        };
        let mut chunks = split_message(&final_text, MESSAGE_LIMIT).into_iter();
        if let Some(head) = chunks.next() {
            placeholder
                .edit(&ctx.http, EditMessage::new().content(head))
                .await?;
        }
        for chunk in chunks {
            msg.channel_id
                .send_message(&ctx.http, CreateMessage::new().content(chunk))
                .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        tracing::info!("connected to Discord as {}", ready.user.name);
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot || !self.allowed(&msg) {
            return;
        }
        let bot_id: UserId = ctx.cache.current_user().id;
        let mentioned = msg.mentions_user_id(bot_id);
        let input = parse_input(
            &msg.content,
            bot_id.get(),
            msg.guild_id.is_none(),
            mentioned,
            &self.config.prefix,
        );
        match input {
            Input::Ignore => {}
            Input::Help => {
                let text = help(&self.config.prefix, &self.agent.config().model);
                self.reply(&ctx, &msg, &text).await;
            }
            Input::Status => {
                let text = self.agent.status().await.summary();
                self.reply(&ctx, &msg, &text).await;
            }
            Input::Reset => {
                let text = match self.agent.reset(&memory_key(msg.author.id.get())).await {
                    Ok(n) => format!("Forgot {n} messages."),
                    Err(err) => format!("Could not reset: {err}"),
                };
                self.reply(&ctx, &msg, &text).await;
            }
            Input::Prompt(prompt) => {
                if let Err(err) = self.answer(&ctx, &msg, prompt).await {
                    tracing::warn!("failed to answer: {err}");
                }
            }
        }
    }
}

pub async fn run(config: BotConfig, agent: ColibriAgent) -> Result<()> {
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;
    let token = config.token.clone();
    let handler = Handler {
        agent: Arc::new(agent),
        config,
        queue: TurnQueue::new(),
    };
    let mut client = Client::builder(&token, intents)
        .event_handler(handler)
        .await
        .context("cannot create Discord client")?;
    let shard_manager = client.shard_manager.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            shard_manager.shutdown_all().await;
        }
    });
    client.start().await.context("Discord connection failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOT: u64 = 4242;

    #[test]
    fn dms_are_prompts() {
        assert_eq!(
            parse_input("hello", BOT, true, false, "!"),
            Input::Prompt("hello".into())
        );
    }

    #[test]
    fn channels_need_mention_or_ask() {
        assert_eq!(parse_input("hello", BOT, false, false, "!"), Input::Ignore);
        assert_eq!(
            parse_input("<@4242> how much RAM?", BOT, false, true, "!"),
            Input::Prompt("how much RAM?".into())
        );
        assert_eq!(
            parse_input("<@!4242> hi", BOT, false, true, "!"),
            Input::Prompt("hi".into())
        );
        assert_eq!(
            parse_input("!ask what is MoE", BOT, false, false, "!"),
            Input::Prompt("what is MoE".into())
        );
        assert_eq!(parse_input("!ask", BOT, false, false, "!"), Input::Ignore);
    }

    #[test]
    fn commands() {
        assert_eq!(
            parse_input("!status", BOT, false, false, "!"),
            Input::Status
        );
        assert_eq!(
            parse_input("<@4242> !reset", BOT, false, true, "!"),
            Input::Reset
        );
        assert_eq!(parse_input("?HELP", BOT, true, false, "?"), Input::Help);
        assert_eq!(
            parse_input("!unknown", BOT, true, false, "!"),
            Input::Ignore
        );
    }

    #[test]
    fn mention_only_is_ignored() {
        assert_eq!(parse_input("<@4242>", BOT, false, true, "!"), Input::Ignore);
    }

    #[test]
    fn ids_and_keys() {
        assert_eq!(parse_ids("1,2").unwrap(), HashSet::from([1, 2]));
        assert_eq!(memory_key(7), "dc:7");
    }

    #[test]
    fn help_mentions_prefix() {
        assert!(help("?", "glm-5.2-colibri").contains("`?status`"));
    }
}
