use std::{
    collections::{HashMap, HashSet},
    time::{Duration, SystemTime},
};

use poise::serenity_prelude::Message;
use serenity::all::{ChannelId, GetMessages, GuildId, RoleId, UserId};
use tracing::{debug, info, instrument, trace, warn};

use crate::stats::{ChannelSnapshot, RoleMessageShare};

const DAY_SECONDS: i64 = 86_400;
const STARTUP_WINDOW_DAYS: u32 = 7;
pub const STARTUP_ROLE_ID: u64 = 1488935241249849396;
pub const STARTUP_CHANNEL_IDS: [u64; 2] = [1482030003993706653, 1488959010332872815];

#[derive(Debug, Clone)]
pub struct RoleActivityReport {
    pub role_id: u64,
    pub window_days: u32,
    pub total_messages: u64,
    pub role_messages: u64,
    pub percentage: f64,
    pub channels: Vec<RoleActivityChannelReport>,
}

#[derive(Debug, Clone)]
pub struct RoleActivityChannelReport {
    pub channel_id: u64,
    pub total_messages: u64,
    pub role_messages: u64,
    pub percentage: f64,
    pub pages_scanned: u64,
}

#[allow(dead_code)]
pub async fn run_startup_role_activity_report(
    ctx: &serenity::client::Context,
    guild_id: GuildId,
) -> anyhow::Result<RoleActivityReport> {
    let channel_ids: Vec<ChannelId> = STARTUP_CHANNEL_IDS
        .iter()
        .copied()
        .map(ChannelId::new)
        .collect();
    let report = role_message_report(
        ctx,
        guild_id,
        RoleId::new(STARTUP_ROLE_ID),
        &channel_ids,
        STARTUP_WINDOW_DAYS,
    )
    .await?;

    info!(
        guild_id = %guild_id,
        role_id = report.role_id,
        window_days = report.window_days,
        total_messages = report.total_messages,
        role_messages = report.role_messages,
        percentage = report.percentage,
        "role message activity report complete"
    );

    for channel in &report.channels {
        info!(
            channel_id = channel.channel_id,
            total_messages = channel.total_messages,
            role_messages = channel.role_messages,
            percentage = channel.percentage,
            pages_scanned = channel.pages_scanned,
            "role message activity channel result"
        );
    }

    Ok(report)
}

pub async fn fetch_live_channel_snapshot(
    ctx: &serenity::client::Context,
    guild_id: GuildId,
    channel_id: ChannelId,
    channel_name: String,
    window_days: u32,
    role_id: Option<RoleId>,
) -> anyhow::Result<ChannelSnapshot> {
    let role_members = match role_id {
        Some(role_id) => Some(role_members(ctx, guild_id, role_id).await?),
        None => None,
    };

    let (start_day, end_exclusive) = window_bounds(window_days);
    let mut daily_counts = vec![0u32; window_days as usize];
    let mut message_lengths = Vec::new();
    let mut hourly_buckets = [0u32; 24];
    let mut poster_counts = HashMap::<u64, u32>::new();
    let mut total_messages = 0u64;
    let mut role_messages = 0u64;
    scan_channel_messages(ctx, channel_id, start_day, end_exclusive, |message| {
        record_message(
            message,
            start_day,
            &mut daily_counts,
            &mut hourly_buckets,
            &mut message_lengths,
            &mut poster_counts,
        );
        total_messages += 1;
        if role_members
            .as_ref()
            .is_some_and(|members| members.contains(&message.author.id))
        {
            role_messages += 1;
        }
    })
    .await?;

    let unique_authors = poster_counts.len() as u32;
    let mut top_posters: Vec<(u64, u32)> = poster_counts.into_iter().collect();
    top_posters.sort_by(|(left_user, left_count), (right_user, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_user.cmp(right_user))
    });
    top_posters.truncate(5);

    let role_share = role_id.map(|role_id| RoleMessageShare {
        role_id: role_id.get(),
        role_messages,
        total_messages,
        percentage: percentage(role_messages, total_messages),
    });

    Ok(ChannelSnapshot {
        channel_id: channel_id.get(),
        channel_name,
        window_days,
        daily_counts,
        message_lengths,
        hourly_buckets,
        top_posters,
        unique_authors,
        total_messages,
        role_share,
    })
}

#[instrument(
    skip(ctx, channels),
    fields(
        guild_id = %guild_id,
        role_id = %role_id,
        channel_count = channels.len(),
        window_days
    )
)]
pub async fn role_message_report(
    ctx: &serenity::client::Context,
    guild_id: GuildId,
    role_id: RoleId,
    channels: &[ChannelId],
    window_days: u32,
) -> anyhow::Result<RoleActivityReport> {
    info!("starting role activity calculation");
    let role_members = role_members(ctx, guild_id, role_id).await?;
    let (start_day, end_exclusive) = window_bounds(window_days);

    let mut total_messages = 0u64;
    let mut role_messages = 0u64;
    let mut channel_reports = Vec::with_capacity(channels.len());

    for &channel_id in channels {
        info!(%channel_id, "starting channel scan");
        let mut before = None;
        let mut pages_scanned = 0u64;
        let mut channel_total = 0u64;
        let mut channel_role_total = 0u64;

        loop {
            let mut builder = GetMessages::new().limit(100);
            if let Some(message_id) = before {
                builder = builder.before(message_id);
            }

            let messages = channel_id.messages(&ctx.http, builder).await?;
            if messages.is_empty() {
                info!(
                    %channel_id,
                    pages_scanned,
                    channel_total,
                    channel_role_total,
                    "no more messages in channel"
                );
                break;
            }

            pages_scanned += 1;
            debug!(
                %channel_id,
                page = pages_scanned,
                batch_size = messages.len(),
                "received message batch"
            );

            let oldest_message = messages.last().expect("non-empty message page");
            let oldest_timestamp = oldest_message.timestamp.unix_timestamp();

            for message in &messages {
                let timestamp = message.timestamp.unix_timestamp();
                if timestamp < start_day {
                    trace!(%channel_id, message_id = %message.id, "reached cutoff message");
                    break;
                }

                if timestamp >= end_exclusive {
                    continue;
                }

                channel_total += 1;
                if role_members.contains(&message.author.id) {
                    channel_role_total += 1;
                }
            }

            if oldest_timestamp < start_day {
                break;
            }

            before = Some(oldest_message.id);
        }

        total_messages += channel_total;
        role_messages += channel_role_total;
        let channel_percentage = percentage(channel_role_total, channel_total);
        channel_reports.push(RoleActivityChannelReport {
            channel_id: channel_id.get(),
            total_messages: channel_total,
            role_messages: channel_role_total,
            percentage: channel_percentage,
            pages_scanned,
        });

        info!(
            %channel_id,
            pages_scanned,
            channel_total,
            channel_role_total,
            percentage = channel_percentage,
            "channel scan complete"
        );
    }

    if total_messages == 0 {
        warn!("no messages found within time window");
    }

    Ok(RoleActivityReport {
        role_id: role_id.get(),
        window_days,
        total_messages,
        role_messages,
        percentage: percentage(role_messages, total_messages),
        channels: channel_reports,
    })
}

async fn role_members(
    ctx: &serenity::client::Context,
    guild_id: GuildId,
    role_id: RoleId,
) -> anyhow::Result<HashSet<UserId>> {
    info!("fetching guild members");
    let members = guild_id.members(&ctx.http, None, None).await?;
    info!(total_members = members.len(), "guild member list retrieved");

    Ok(members
        .into_iter()
        .filter(|member| member.roles.contains(&role_id))
        .map(|member| member.user.id)
        .collect())
}

fn record_message(
    message: &Message,
    start_day: i64,
    daily_counts: &mut [u32],
    hourly_buckets: &mut [u32; 24],
    message_lengths: &mut Vec<u32>,
    poster_counts: &mut HashMap<u64, u32>,
) {
    let timestamp = message.timestamp.unix_timestamp();
    let day_index = ((timestamp - start_day) / DAY_SECONDS) as usize;
    if let Some(count) = daily_counts.get_mut(day_index) {
        *count += 1;
    }

    let hour = (timestamp.rem_euclid(DAY_SECONDS) / 3_600) as usize;
    if let Some(count) = hourly_buckets.get_mut(hour) {
        *count += 1;
    }

    if message_lengths.len() < 500 {
        message_lengths.push(message.content.chars().count() as u32);
    }

    *poster_counts.entry(message.author.id.get()).or_default() += 1;
}

fn percentage(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (part as f64 * 100.0) / total as f64
    }
}

fn window_bounds(window_days: u32) -> (i64, i64) {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64;
    let today = now - now.rem_euclid(DAY_SECONDS);
    let start_day = today - (window_days.saturating_sub(1) as i64 * DAY_SECONDS);
    let end_exclusive = today + DAY_SECONDS;
    (start_day, end_exclusive)
}

/// Scan existing messages in a fixed half-open UTC window; errors never yield a partial report.
pub async fn scan_channel_messages(
    ctx: &serenity::client::Context,
    channel_id: ChannelId,
    start: i64,
    end: i64,
    mut record: impl FnMut(&Message) + Send,
) -> anyhow::Result<()> {
    let mut before = None;
    loop {
        let mut builder = GetMessages::new().limit(100);
        if let Some(id) = before {
            builder = builder.before(id);
        }
        let messages = channel_id.messages(&ctx.http, builder).await?;
        let Some(oldest) = messages.last() else {
            break;
        };
        for message in &messages {
            let timestamp = message.timestamp.unix_timestamp();
            if timestamp >= start && timestamp < end {
                record(message);
            }
        }
        if oldest.timestamp.unix_timestamp() < start {
            break;
        }
        anyhow::ensure!(
            before != Some(oldest.id),
            "channel pagination did not advance"
        );
        before = Some(oldest.id);
    }
    Ok(())
}
