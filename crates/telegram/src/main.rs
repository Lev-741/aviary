use anyhow::Result;
use aviary_telegram::{BotConfig, run};

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
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
    if config.allowed_users.is_empty() {
        tracing::warn!("TELEGRAM_ALLOWED_USERS is empty, anyone who finds the bot can use it");
    }
    tracing::info!("starting Telegram bot");
    run(config, agent).await
}
