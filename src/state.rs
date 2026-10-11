use std::sync::Arc;

use poise::serenity_prelude::{ChannelId, GuildId};

use crate::{
    config::{BootstrapConfig, RuntimeGuildSettings, RuntimeGuildSettingsDefaults},
    db::Database,
    error::{AppError, AppResult},
};

#[derive(Clone)]
pub struct AppState {
    membership_tracker: crate::bot::membership::MembershipTracker,
    config: Arc<BootstrapConfig>,
    database: Database,
    highlights_database: crate::db::HighlightsDatabase,
    highlight_cache: crate::domain::highlights::SharedHighlightCache,
    blacklist_database: crate::db::BlacklistDatabase,
}

impl AppState {
    pub fn new(
        config: Arc<BootstrapConfig>,
        database: Database,
        highlights_database: crate::db::HighlightsDatabase,
        highlight_cache: crate::domain::highlights::SharedHighlightCache,
        blacklist_database: crate::db::BlacklistDatabase,
    ) -> Self {
        Self {
            membership_tracker: Default::default(),
            config,
            database,
            highlights_database,
            highlight_cache,
            blacklist_database,
        }
    }

    pub fn membership_tracker(&self) -> &crate::bot::membership::MembershipTracker {
        &self.membership_tracker
    }

    pub fn config(&self) -> &Arc<BootstrapConfig> {
        &self.config
    }

    pub fn database(&self) -> &Database {
        &self.database
    }

    pub fn highlights_database(&self) -> &crate::db::HighlightsDatabase {
        &self.highlights_database
    }

    pub fn highlight_cache(&self) -> &crate::domain::highlights::SharedHighlightCache {
        &self.highlight_cache
    }
    pub fn blacklist_database(&self) -> &crate::db::BlacklistDatabase {
        &self.blacklist_database
    }

    pub async fn guild_settings(&self, guild_id: GuildId) -> AppResult<RuntimeGuildSettings> {
        self.database
            .ensure_guild_settings(guild_id.get() as i64, self.guild_defaults())
            .await
    }

    pub fn guild_defaults(&self) -> RuntimeGuildSettingsDefaults {
        RuntimeGuildSettingsDefaults {
            log_channel_id: self.config.moderation.default_log_channel_id,
            leave_log_channel_id: self.config.moderation.default_leave_log_channel_id,
            require_reason: self.config.moderation.require_reason,
            ephemeral_slash_responses: self.config.moderation.ephemeral_slash_responses,
            notes_enabled: false,
            appeals_enabled: false,
        }
    }

    pub async fn audit_log_channel(&self, guild_id: GuildId) -> AppResult<ChannelId> {
        let settings = self.guild_settings(guild_id).await?;
        let channel_id = settings
            .log_channel_id
            .or(self.config.moderation.default_log_channel_id)
            .ok_or(AppError::MissingLogChannel {
                guild_id: guild_id.get(),
            })?;

        Ok(ChannelId::new(channel_id))
    }

    pub async fn leave_log_channel(&self, guild_id: GuildId) -> AppResult<ChannelId> {
        let settings = self.guild_settings(guild_id).await?;
        let channel_id = settings
            .leave_log_channel_id
            .or(self.config.moderation.default_leave_log_channel_id)
            .ok_or(AppError::MissingLogChannel {
                guild_id: guild_id.get(),
            })?;

        Ok(ChannelId::new(channel_id))
    }
}
