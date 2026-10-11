use snafu::{OptionExt, ResultExt};
use sqlx::{SqlitePool, query, query_as};

use crate::{
    config::{RuntimeGuildSettings, RuntimeGuildSettingsDefaults},
    db::{
        models::{
            CaseNoteRecord, GuildSettingsRecord, LeaveApplicationRecord, ModerationCaseRecord,
        },
        pool,
    },
    domain::actions::{NewLeaveApplication, NewModerationCase},
    error::{AppResult, DatabaseMigrationSnafu, DatabaseSnafu, NotFoundSnafu},
};

#[derive(Clone)]
pub struct Database {
    pub(super) pool: SqlitePool,
}

impl Database {
    pub async fn connect(database_url: &str) -> AppResult<Self> {
        let pool = pool::connect(database_url).await?;
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> AppResult<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .context(DatabaseMigrationSnafu)
    }

    pub async fn ensure_guild_settings(
        &self,
        guild_id: i64,
        defaults: RuntimeGuildSettingsDefaults,
    ) -> AppResult<RuntimeGuildSettings> {
        query(
            "INSERT INTO guild_settings (
                guild_id,
                log_channel_id,
                leave_log_channel_id,
                require_reason,
                ephemeral_slash_responses,
                notes_enabled,
                appeals_enabled,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%s', 'now'), strftime('%s', 'now'))
            ON CONFLICT(guild_id) DO NOTHING",
        )
        .bind(guild_id)
        .bind(defaults.log_channel_id.map(|value| value as i64))
        .bind(defaults.leave_log_channel_id.map(|value| value as i64))
        .bind(defaults.require_reason)
        .bind(defaults.ephemeral_slash_responses)
        .bind(defaults.notes_enabled)
        .bind(defaults.appeals_enabled)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        let record = query_as::<_, GuildSettingsRecord>(
            "SELECT guild_id, log_channel_id, leave_log_channel_id, require_reason, ephemeral_slash_responses,
                notes_enabled, appeals_enabled, created_at, updated_at
             FROM guild_settings
             WHERE guild_id = ?1",
        )
        .bind(guild_id)
        .fetch_optional(&self.pool)
        .await
        .context(DatabaseSnafu)?
        .context(NotFoundSnafu {
            entity: "guild_settings",
            id: guild_id.to_string(),
        })?;

        let mod_role_ids = self.mod_role_ids(guild_id).await?;
        Ok(RuntimeGuildSettings::from_record(record, mod_role_ids))
    }

    pub async fn set_log_channel(&self, guild_id: i64, channel_id: Option<i64>) -> AppResult<()> {
        query(
            "UPDATE guild_settings
             SET log_channel_id = ?2, updated_at = strftime('%s', 'now')
             WHERE guild_id = ?1",
        )
        .bind(guild_id)
        .bind(channel_id)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(())
    }

    pub async fn set_leave_log_channel(
        &self,
        guild_id: i64,
        channel_id: Option<i64>,
    ) -> AppResult<()> {
        query(
            "UPDATE guild_settings
             SET leave_log_channel_id = ?2, updated_at = strftime('%s', 'now')
             WHERE guild_id = ?1",
        )
        .bind(guild_id)
        .bind(channel_id)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(())
    }

    pub async fn set_require_reason(&self, guild_id: i64, value: bool) -> AppResult<()> {
        query(
            "UPDATE guild_settings
             SET require_reason = ?2, updated_at = strftime('%s', 'now')
             WHERE guild_id = ?1",
        )
        .bind(guild_id)
        .bind(value)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(())
    }

    pub async fn set_ephemeral_slash_responses(&self, guild_id: i64, value: bool) -> AppResult<()> {
        query(
            "UPDATE guild_settings
             SET ephemeral_slash_responses = ?2, updated_at = strftime('%s', 'now')
             WHERE guild_id = ?1",
        )
        .bind(guild_id)
        .bind(value)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(())
    }

    pub async fn add_mod_role(&self, guild_id: i64, role_id: i64) -> AppResult<()> {
        query(
            "INSERT INTO guild_mod_roles (guild_id, role_id)
             VALUES (?1, ?2)
             ON CONFLICT(guild_id, role_id) DO NOTHING",
        )
        .bind(guild_id)
        .bind(role_id)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(())
    }

    pub async fn remove_mod_role(&self, guild_id: i64, role_id: i64) -> AppResult<()> {
        query("DELETE FROM guild_mod_roles WHERE guild_id = ?1 AND role_id = ?2")
            .bind(guild_id)
            .bind(role_id)
            .execute(&self.pool)
            .await
            .context(DatabaseSnafu)?;

        Ok(())
    }

    pub async fn upsert_message_archive(
        &self,
        guild_id: i64,
        channel_id: i64,
        message_id: i64,
        author_id: i64,
        created_at: i64,
        content_len: i64,
    ) -> AppResult<()> {
        query(
            "INSERT INTO message_archive (
                message_id,
                guild_id,
                channel_id,
                author_id,
                created_at,
                content_len
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(message_id) DO UPDATE SET
                guild_id = excluded.guild_id,
                channel_id = excluded.channel_id,
                author_id = excluded.author_id,
                created_at = excluded.created_at,
                content_len = excluded.content_len",
        )
        .bind(message_id)
        .bind(guild_id)
        .bind(channel_id)
        .bind(author_id)
        .bind(created_at)
        .bind(content_len)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(())
    }

    pub async fn delete_message_archive(&self, message_id: i64) -> AppResult<()> {
        query("DELETE FROM message_archive WHERE message_id = ?1")
            .bind(message_id)
            .execute(&self.pool)
            .await
            .context(DatabaseSnafu)?;

        Ok(())
    }

    pub async fn create_case(
        &self,
        new_case: &NewModerationCase,
    ) -> AppResult<ModerationCaseRecord> {
        let result = query(
            "INSERT INTO moderation_cases (
                guild_id,
                action_type,
                target_user_id,
                moderator_user_id,
                message_id,
                reason,
                duration_seconds,
                details,
                created_at,
                expires_at,
                audit_log_channel_id,
                audit_log_message_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%s', 'now'), ?9, NULL, NULL)",
        )
        .bind(new_case.guild_id)
        .bind(new_case.action_type.as_str())
        .bind(new_case.target_user_id)
        .bind(new_case.moderator_user_id)
        .bind(new_case.message_id)
        .bind(new_case.reason.clone())
        .bind(new_case.duration_seconds)
        .bind(new_case.details.clone())
        .bind(new_case.expires_at)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        self.case_by_id(result.last_insert_rowid()).await
    }

    pub async fn update_case_reason(&self, case_id: i64, reason: &str) -> AppResult<()> {
        query("UPDATE moderation_cases SET reason = ?1 WHERE id = ?2")
            .bind(reason)
            .bind(case_id)
            .execute(&self.pool)
            .await
            .context(DatabaseSnafu)?;
        Ok(())
    }

    pub async fn update_case_audit_message(
        &self,
        case_id: i64,
        channel_id: i64,
        message_id: i64,
    ) -> AppResult<()> {
        query(
            "UPDATE moderation_cases
             SET audit_log_channel_id = ?2, audit_log_message_id = ?3
             WHERE id = ?1",
        )
        .bind(case_id)
        .bind(channel_id)
        .bind(message_id)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(())
    }

    pub async fn case_by_id(&self, case_id: i64) -> AppResult<ModerationCaseRecord> {
        query_as::<_, ModerationCaseRecord>(
            "SELECT id, guild_id, action_type, target_user_id, moderator_user_id, reason,
                message_id, duration_seconds, details, created_at, expires_at, audit_log_channel_id, audit_log_message_id
             FROM moderation_cases
             WHERE id = ?1",
        )
        .bind(case_id)
        .fetch_optional(&self.pool)
        .await
        .context(DatabaseSnafu)?
        .context(NotFoundSnafu {
            entity: "case",
            id: case_id.to_string(),
        })
    }

    pub async fn guild_case_by_id(
        &self,
        guild_id: i64,
        case_id: i64,
    ) -> AppResult<ModerationCaseRecord> {
        query_as::<_, ModerationCaseRecord>(
            "SELECT id, guild_id, action_type, target_user_id, moderator_user_id, reason,
                message_id, duration_seconds, details, created_at, expires_at, audit_log_channel_id, audit_log_message_id
             FROM moderation_cases
             WHERE guild_id = ?1 AND id = ?2",
        )
        .bind(guild_id)
        .bind(case_id)
        .fetch_optional(&self.pool)
        .await
        .context(DatabaseSnafu)?
        .context(NotFoundSnafu {
            entity: "case",
            id: case_id.to_string(),
        })
    }

    pub async fn list_cases_for_user(
        &self,
        guild_id: i64,
        user_id: i64,
        limit: u8,
    ) -> AppResult<Vec<ModerationCaseRecord>> {
        query_as::<_, ModerationCaseRecord>(
            "SELECT id, guild_id, action_type, target_user_id, moderator_user_id, reason,
                message_id, duration_seconds, details, created_at, expires_at, audit_log_channel_id, audit_log_message_id
             FROM moderation_cases
             WHERE guild_id = ?1 AND target_user_id = ?2
             ORDER BY created_at DESC
             LIMIT ?3",
        )
        .bind(guild_id)
        .bind(user_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .context(DatabaseSnafu)
    }

    pub async fn list_expired_tempban_cases(
        &self,
        now: i64,
    ) -> AppResult<Vec<ModerationCaseRecord>> {
        query_as::<_, ModerationCaseRecord>(
            "SELECT id, guild_id, action_type, target_user_id, moderator_user_id, reason,
                message_id, duration_seconds, details, created_at, expires_at, audit_log_channel_id, audit_log_message_id
             FROM moderation_cases
             WHERE action_type = 'ban'
               AND expires_at IS NOT NULL
               AND expires_at <= ?1
             ORDER BY expires_at ASC, created_at ASC",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .context(DatabaseSnafu)
    }

    pub async fn add_note(
        &self,
        case_id: i64,
        author_user_id: i64,
        content: &str,
    ) -> AppResult<CaseNoteRecord> {
        let result = query(
            "INSERT INTO case_notes (case_id, author_user_id, content, created_at)
             VALUES (?1, ?2, ?3, strftime('%s', 'now'))",
        )
        .bind(case_id)
        .bind(author_user_id)
        .bind(content)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        query_as::<_, CaseNoteRecord>(
            "SELECT id, case_id, author_user_id, content, created_at
             FROM case_notes
             WHERE id = ?1",
        )
        .bind(result.last_insert_rowid())
        .fetch_optional(&self.pool)
        .await
        .context(DatabaseSnafu)?
        .context(NotFoundSnafu {
            entity: "case_note",
            id: result.last_insert_rowid().to_string(),
        })
    }

    pub async fn create_leave_application(
        &self,
        new_leave: &NewLeaveApplication,
    ) -> AppResult<LeaveApplicationRecord> {
        let result = query(
            "INSERT INTO leave_applications (
                guild_id,
                applicant_user_id,
                applicant_name,
                duration_text,
                reason,
                created_by_user_id,
                starts_at,
                ends_at,
                is_active,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, strftime('%s', 'now'), strftime('%s', 'now'))",
        )
        .bind(new_leave.guild_id)
        .bind(new_leave.applicant_user_id)
        .bind(new_leave.applicant_name.clone())
        .bind(new_leave.duration_text.clone())
        .bind(new_leave.reason.clone())
        .bind(new_leave.created_by_user_id)
        .bind(new_leave.starts_at)
        .bind(new_leave.ends_at)
        .bind(new_leave.is_active)
        .execute(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        query_as::<_, LeaveApplicationRecord>(
            "SELECT id, guild_id, applicant_user_id, applicant_name, duration_text, reason,
                created_by_user_id, starts_at, ends_at, is_active, created_at, updated_at
             FROM leave_applications
             WHERE id = ?1",
        )
        .bind(result.last_insert_rowid())
        .fetch_optional(&self.pool)
        .await
        .context(DatabaseSnafu)?
        .context(NotFoundSnafu {
            entity: "leave_application",
            id: result.last_insert_rowid().to_string(),
        })
    }

    pub async fn list_active_leave_applications(
        &self,
        guild_id: i64,
    ) -> AppResult<Vec<LeaveApplicationRecord>> {
        query_as::<_, LeaveApplicationRecord>(
            "SELECT id, guild_id, applicant_user_id, applicant_name, duration_text, reason,
                created_by_user_id, starts_at, ends_at, is_active, created_at, updated_at
             FROM leave_applications
             WHERE guild_id = ?1
               AND is_active = 1
               AND (starts_at IS NULL OR starts_at <= strftime('%s', 'now'))
               AND (ends_at IS NULL OR ends_at >= strftime('%s', 'now'))
             ORDER BY created_at DESC",
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await
        .context(DatabaseSnafu)
    }

    pub async fn list_leave_applications_for_user(
        &self,
        guild_id: i64,
        applicant_user_id: i64,
    ) -> AppResult<Vec<LeaveApplicationRecord>> {
        query_as::<_, LeaveApplicationRecord>(
            "SELECT id, guild_id, applicant_user_id, applicant_name, duration_text, reason,
                created_by_user_id, starts_at, ends_at, is_active, created_at, updated_at
             FROM leave_applications
             WHERE guild_id = ?1 AND applicant_user_id = ?2
             ORDER BY created_at DESC",
        )
        .bind(guild_id)
        .bind(applicant_user_id)
        .fetch_all(&self.pool)
        .await
        .context(DatabaseSnafu)
    }

    async fn mod_role_ids(&self, guild_id: i64) -> AppResult<Vec<u64>> {
        let role_rows = query_as::<_, (i64,)>(
            "SELECT role_id
             FROM guild_mod_roles
             WHERE guild_id = ?1
             ORDER BY role_id ASC",
        )
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await
        .context(DatabaseSnafu)?;

        Ok(role_rows
            .into_iter()
            .map(|(role_id,)| role_id as u64)
            .collect())
    }
}
