use std::{
    env,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use anyhow::{Context, anyhow};
use time::UtcDateTime;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

use crate::{bot, config::BootstrapConfig, db::Database, state::AppState};

pub static START_TIMESTAMP: OnceLock<UtcDateTime> = OnceLock::new();

pub async fn run() -> anyhow::Result<()> {
    START_TIMESTAMP
        .set(UtcDateTime::now())
        .map_err(|_| anyhow!("Unable to set start time. This should not be possible"))?;
    let config_path = env::var("MODBOT_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("config.toml"));
    let config = Arc::new(BootstrapConfig::load(&config_path)?);

    init_tracing(&config.logging.filter)?;
    info!(pingable_members = ?config.join_pinglist.members.len(), "Loaded pinglist with");
    info!(revive_role_id = ?config.moderation.revive_role_id, "Loaded ");
    info!(moderation_log_channel = ?config.moderation.default_log_channel_id , "Loaded ");

    let database = Database::connect(&config.database.url)
        .await
        .context("failed to initialize database")?;
    database
        .migrate()
        .await
        .context("failed to run database migrations")?;

    let highlights_database =
        crate::db::HighlightsDatabase::connect(&config.database.highlights_url)
            .await
            .context("failed to initialize highlights database")?;
    highlights_database
        .migrate()
        .await
        .context("failed to run highlights database migrations")?;

    let highlight_cache = crate::domain::highlights::build_initial_cache(&highlights_database)
        .await
        .context("failed to build initial highlight cache")?;

    let blacklist_database = crate::db::BlacklistDatabase::connect(&config.database.blacklist_url)
        .await
        .context("failed to initialize blacklist database")?;
    blacklist_database
        .migrate()
        .await
        .context("failed to run blacklist database migrations")?;

    let state = AppState::new(
        config,
        database,
        highlights_database,
        highlight_cache,
        blacklist_database,
    );

    bot::framework::run(state).await
}

fn init_tracing(filter: &str) -> anyhow::Result<()> {
    let env_filter = EnvFilter::try_new(filter).or_else(|_| EnvFilter::try_new("info"))?;

    fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .compact()
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize tracing subscriber: {error}"))?;

    Ok(())
}
