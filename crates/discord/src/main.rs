use anyhow::Result;
use aviary_discord::{BotConfig, run};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,serenity=warn")),
        )
        .init();

    let config = BotConfig::from_env()?;
    let agent = aviary_core::env::agent_from_env().await?;
    let status = agent.status().await;
    if status.reachable {
        tracing::info!("Colibri reachable at {}", status.base_url);
    } else {
        tracing::warn!("{}", status.summary());
    }
    if config.allowed_users.is_empty() && config.allowed_channels.is_empty() {
        tracing::warn!(
            "no DISCORD_ALLOWED_USERS or DISCORD_ALLOWED_CHANNELS set, every server member can use the bot"
        );
    }
    tracing::info!("starting Discord bot");
    run(config, agent).await
}
