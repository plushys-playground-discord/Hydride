pub mod activity_graph;
pub mod banlist;
pub mod cases;
pub mod config;
pub mod highlights;
pub mod leave;
pub mod meta;
pub mod moderation;
pub mod stats;

use poise::{
    CreateReply,
    serenity_prelude::{GuildId, Member, PartialGuild, Permissions, UserId},
};
use serenity::all::CreateMessage;

use crate::{
    config::RuntimeGuildSettings,
    db::models::ModerationCaseRecord,
    domain::{
        actions::NewModerationCase,
        logging::{self, case_embed},
        permissions,
    },
    error::{AppError, AppResult},
    state::AppState,
};

pub type Error = anyhow::Error;
pub type Context<'a> = poise::Context<'a, AppState, Error>;

pub fn all() -> Vec<poise::Command<AppState, Error>> {
    vec![
        meta::ping(),
        meta::help(),
        meta::uptime(),
        meta::pinglist(),
        meta::revive(),
        meta::set_role_icon(),
        meta::register(),
        meta::source(),
        meta::verify(),
        banlist::banlist(),
        moderation::warn(),
        moderation::timeout(),
        moderation::kick(),
        moderation::ban(),
        moderation::tempban(),
        moderation::unban(),
        moderation::purge(),
        moderation::nick(),
        leave::leave(),
        cases::case(),
        cases::cases(),
        cases::remove_warn(),
        cases::update_reason(),
        stats::stats(),
        activity_graph::activity_graph(),
        config::config(),
        highlights::highlight(),
    ]
}

pub async fn guild_id(ctx: Context<'_>) -> AppResult<GuildId> {
    ctx.guild_id().ok_or(AppError::GuildOnly)
}

pub async fn guild_settings(ctx: Context<'_>) -> AppResult<(GuildId, RuntimeGuildSettings)> {
    let guild_id = guild_id(ctx).await?;
    let settings = ctx.data().guild_settings(guild_id).await?;
    Ok((guild_id, settings))
}

pub async fn require_moderator(
    ctx: Context<'_>,
    settings: &RuntimeGuildSettings,
    required_permission: Permissions,
) -> AppResult<(GuildId, PartialGuild, Member)> {
    let guild_id = guild_id(ctx).await?;
    let (guild, member) =
        permissions::fetch_guild_and_member(ctx.serenity_context(), guild_id, ctx.author().id)
            .await?;

    permissions::ensure_moderator_access(&guild, &member, settings, required_permission)?;
    Ok((guild_id, guild, member))
}

pub async fn require_config_manager(
    ctx: Context<'_>,
    settings: &RuntimeGuildSettings,
) -> AppResult<(GuildId, PartialGuild, Member)> {
    require_moderator(ctx, settings, Permissions::MANAGE_GUILD).await
}

pub async fn fetch_target_member(
    ctx: Context<'_>,
    guild_id: GuildId,
    user_id: UserId,
) -> AppResult<Member> {
    guild_id
        .member(ctx.serenity_context(), user_id)
        .await
        .map_err(|source| AppError::Discord {
            source: Box::new(source),
        })
}

pub async fn ensure_action_target(
    ctx: Context<'_>,
    guild: &PartialGuild,
    actor: &Member,
    target: &Member,
    required_permission: Permissions,
) -> AppResult<Member> {
    let bot_member =
        permissions::ensure_bot_permissions(ctx.serenity_context(), guild, required_permission)
            .await?;
    permissions::ensure_targetable(guild, actor, target)?;
    permissions::ensure_bot_can_target(guild, &bot_member, target)?;
    Ok(bot_member)
}

pub fn normalized_reason(
    settings: &RuntimeGuildSettings,
    reason: Option<String>,
) -> AppResult<Option<String>> {
    match reason {
        Some(reason) if !reason.trim().is_empty() => Ok(Some(reason.trim().to_string())),
        _ if settings.require_reason => Err(AppError::InvalidInput {
            message: "a reason is required by this guild's moderation settings".into(),
        }),
        _ => Ok(None),
    }
}

pub fn normalized_message_id(message_id: Option<u64>) -> AppResult<Option<i64>> {
    message_id
        .map(|message_id| {
            i64::try_from(message_id).map_err(|_| AppError::InvalidInput {
                message: "message-id is too large".into(),
            })
        })
        .transpose()
}

pub async fn create_case_and_log(
    ctx: Context<'_>,
    new_case: NewModerationCase,
) -> Result<(ModerationCaseRecord, bool), Error> {
    let case = ctx.data().database().create_case(&new_case).await?;
    let channel_id = ctx
        .data()
        .audit_log_channel(GuildId::new(case.guild_id as u64))
        .await?;

    let logged = match logging::send_case_log(ctx.serenity_context(), channel_id, &case).await {
        Ok(message) => {
            ctx.data()
                .database()
                .update_case_audit_message(
                    case.id,
                    channel_id.get() as i64,
                    message.id.get() as i64,
                )
                .await?;
            true
        }
        Err(error) => {
            tracing::error!(case_id = case.id, %error, "failed to send moderation audit log");
            false
        }
    };

    let message = CreateMessage::new().embed(case_embed(&case));
    if let Some(user) = new_case.target_user_id {
        let user = UserId::new(user as u64);
        match user.create_dm_channel(ctx).await {
            Ok(private_channel) => {
                let _ = private_channel.send_message(ctx, message).await;
            }
            Err(e) => {
                tracing::error!(case_id = case.id, user = ?new_case.target_user_id, %e, "Failed to DM user with moderation log")
            }
        }
    }

    Ok((case, logged))
}

pub async fn send_status(
    ctx: Context<'_>,
    settings: &RuntimeGuildSettings,
    content: impl Into<String>,
) -> Result<(), Error> {
    let ephemeral =
        matches!(ctx, poise::Context::Application(_)) && settings.ephemeral_slash_responses;
    ctx.send(
        CreateReply::default()
            .content(content.into())
            .ephemeral(ephemeral),
    )
    .await?;
    Ok(())
}
