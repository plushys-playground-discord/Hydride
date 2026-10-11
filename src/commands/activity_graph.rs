use crate::{
    activity_graph::{self as graph, Coverage},
    bot::{activity, membership},
    commands::{Context, Error},
};
use anyhow::{Context as _, ensure};
use poise::{
    CreateReply,
    serenity_prelude::{ChannelId, ChannelType, CreateAttachment, CreateEmbed, Permissions},
};
use std::collections::HashSet;

fn resolve_general(
    channels: impl IntoIterator<Item = (ChannelId, String, ChannelType)>,
) -> anyhow::Result<ChannelId> {
    let matches: Vec<_> = channels
        .into_iter()
        .filter(|(_, name, kind)| {
            name == "general" && matches!(kind, ChannelType::Text | ChannelType::News)
        })
        .map(|(id, _, _)| id)
        .collect();
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => anyhow::bail!("No #general text channel found. Specify a channel."),
        _ => anyhow::bail!("Multiple #general channels found. Specify a channel."),
    }
}

fn ensure_history_access(caller: Permissions, bot: Permissions) -> anyhow::Result<()> {
    let required = Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY;
    ensure!(
        caller.contains(required),
        "You need View Channel and Read Message History in the selected channel."
    );
    ensure!(
        bot.contains(required),
        "The bot needs View Channel and Read Message History in the selected channel."
    );
    Ok(())
}

/// Compare human messages with active members, server membership, and joins/departures.
#[poise::command(
    prefix_command,
    slash_command,
    guild_only,
    category = "Statistics",
    broadcast_typing
)]
pub async fn activity_graph(
    ctx: Context<'_>,
    #[description = "Channel to analyse (defaults to #general)"] channel: Option<ChannelId>,
    #[description = "Lookback days (7–30, default 30)"] days: Option<u32>,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let guild_id = ctx.guild_id().ok_or(crate::error::AppError::GuildOnly)?;
    let days = days.unwrap_or(30).clamp(7, 30);
    let channel_id = match channel {
        Some(id) => id,
        None => resolve_general(
            guild_id
                .channels(&ctx.http())
                .await?
                .into_values()
                .map(|c| (c.id, c.name, c.kind)),
        )?,
    };
    let channel = channel_id
        .to_channel(ctx)
        .await?
        .guild()
        .context("Choose a guild text channel.")?;
    ensure!(
        channel.guild_id == guild_id,
        "Choose a channel in this server."
    );
    // Threads inherit parent permissions but require additional membership checks;
    // v1 accepts ordinary text and announcement channels only.
    ensure!(
        matches!(channel.kind, ChannelType::Text | ChannelType::News),
        "Choose a text or announcement channel."
    );
    let guild = guild_id.to_partial_guild(ctx.http()).await?;
    let caller = guild_id.member(ctx.http(), ctx.author().id).await?;
    let bot_id = ctx.serenity_context().cache.current_user().id;
    let bot = guild_id.member(ctx.http(), bot_id).await?;
    ensure_history_access(
        guild.user_permissions_in(&channel, &caller),
        guild.user_permissions_in(&channel, &bot),
    )?;

    let captured_at = membership::now();
    let (start, end) = graph::window_bounds(captured_at, days);
    let mut messages = vec![0u64; days as usize];
    let mut authors = vec![HashSet::new(); days as usize];
    tokio::time::timeout(
        std::time::Duration::from_secs(600),
        activity::scan_channel_messages(
            ctx.serenity_context(),
            channel_id,
            start,
            captured_at + 1,
            |message| {
                if let Some(i) = graph::message_day(
                    message.timestamp.unix_timestamp(),
                    start,
                    end,
                    message.author.bot,
                    message.webhook_id.is_some(),
                ) {
                    messages[i] += 1;
                    authors[i].insert(message.author.id.get());
                }
            },
        ),
    )
    .await
    .context("Channel scan timed out. Try a shorter window.")??;
    let (membership, intervals) = ctx
        .data()
        .database()
        .membership_history(guild_id.get(), start, end)
        .await?;
    let snapshot = graph::build_snapshot(
        channel.name,
        captured_at,
        start,
        messages,
        authors,
        &membership,
        &intervals,
    );
    let partial = snapshot
        .days
        .iter()
        .filter(|d| d.coverage == Coverage::Partial)
        .count();
    let unavailable = snapshot
        .days
        .iter()
        .filter(|d| d.coverage == Coverage::Unavailable)
        .count();
    let coverage = format!(
        "Membership coverage: {partial} partial days, {unavailable} unavailable days. ~ partial; ? unavailable. Today (*) is incomplete. Membership metrics include all accounts."
    );
    let name = snapshot.channel_name.clone();
    let first_date = time::OffsetDateTime::from_unix_timestamp(start)?.date();
    let last_date = time::OffsetDateTime::from_unix_timestamp(end - 1)?.date();
    let total: u64 = snapshot.days.iter().map(|d| d.messages).sum();
    let images = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<Vec<u8>>> {
        (0..3)
            .map(|metric| graph::render(&snapshot, metric))
            .collect()
    })
    .await
    .context("Graph rendering task failed")??;
    let definitions = [
        "Unique human authors per UTC day in this channel. Messages exclude bots and webhooks.",
        "Last observed server member count per UTC day. Earlier history is unavailable; gaps are not interpolated.",
        "Observed server joins and departures per UTC day. Departures include leaves, kicks, and bans; partial days may undercount.",
    ];
    let mut reply = CreateReply::default().ephemeral(true).content(format!(
        "Activity in #{name} · {first_date} to {last_date} UTC ({days} days) · {total} human messages\n{coverage}"
    ));
    for (metric, bytes) in images.into_iter().enumerate() {
        let filename = format!("activity-{metric}.png");
        reply = reply
            .attachment(CreateAttachment::bytes(bytes, filename.clone()))
            .embed(
                CreateEmbed::new()
                    .title(graph::TITLES[metric])
                    .description(definitions[metric])
                    .image(format!("attachment://{filename}")),
            );
    }
    ctx.send(reply).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_access_requires_both_permissions_for_both_accounts() {
        let full = Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY;
        assert!(ensure_history_access(full, full).is_ok());
        assert!(ensure_history_access(Permissions::VIEW_CHANNEL, full).is_err());
        assert!(ensure_history_access(full, Permissions::READ_MESSAGE_HISTORY).is_err());
    }
    #[test]
    fn default_channel_must_be_unambiguous_text_channel() {
        assert!(resolve_general([]).is_err());
        assert_eq!(
            resolve_general([(ChannelId::new(1), "general".into(), ChannelType::Text)]).unwrap(),
            ChannelId::new(1)
        );
        assert!(
            resolve_general([(ChannelId::new(1), "general".into(), ChannelType::Voice)]).is_err()
        );
        assert!(
            resolve_general([
                (ChannelId::new(1), "general".into(), ChannelType::Text),
                (ChannelId::new(2), "general".into(), ChannelType::Text)
            ])
            .is_err()
        );
    }
}
