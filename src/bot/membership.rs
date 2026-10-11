//! Conservative coverage: disconnected or unwritten intervals are never filled in.
use crate::state::AppState;
use serenity::all::{Context, FullEvent, GuildId};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub struct MembershipTracker(Arc<Mutex<HashMap<GuildId, TrackedGuild>>>);
struct TrackedGuild {
    shard: serenity::all::ShardId,
    available: bool,
    connected: bool,
    coverage: Option<i64>,
    last_tick: i64,
}

pub fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

impl MembershipTracker {
    pub async fn handle(
        &self,
        ctx: &Context,
        event: &FullEvent,
        data: &AppState,
    ) -> anyhow::Result<()> {
        let mut guilds = self.0.lock().await;
        match event {
            FullEvent::GuildCreate { guild, .. } => {
                guilds.insert(
                    guild.id,
                    TrackedGuild {
                        shard: ctx.shard_id,
                        available: true,
                        connected: true,
                        coverage: None,
                        last_tick: now(),
                    },
                );
                data.database()
                    .observe_membership(guild.id.get(), now(), guild.member_count)
                    .await?;
            }
            FullEvent::GuildDelete { incomplete, .. } => {
                if incomplete.unavailable {
                    if let Some(guild) = guilds.get_mut(&incomplete.id) {
                        guild.available = false;
                        guild.coverage = None;
                    }
                } else {
                    guilds.remove(&incomplete.id);
                }
            }
            FullEvent::ShardStageUpdate { event } => {
                for guild in guilds.values_mut().filter(|g| g.shard == event.shard_id) {
                    guild.connected = event.new == serenity::gateway::ConnectionStage::Connected;
                    // Even resumed connections retain a conservative coverage gap.
                    guild.coverage = None;
                }
            }
            FullEvent::GuildMemberAddition { new_member } => {
                if let Err(error) = data
                    .database()
                    .record_membership_event(new_member.guild_id.get(), now(), true)
                    .await
                {
                    if let Some(guild) = guilds.get_mut(&new_member.guild_id) {
                        guild.coverage = None;
                    }
                    return Err(error);
                }
            }
            FullEvent::GuildMemberRemoval { guild_id, .. } => {
                if let Err(error) = data
                    .database()
                    .record_membership_event(guild_id.get(), now(), false)
                    .await
                {
                    if let Some(guild) = guilds.get_mut(guild_id) {
                        guild.coverage = None;
                    }
                    return Err(error);
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn sample(&self, ctx: &Context, data: &AppState) {
        let mut guilds = self.0.lock().await;
        let timestamp = now();
        for (&id, guild) in guilds.iter_mut() {
            if !guild.available || !guild.connected {
                continue;
            }
            let count = ctx.cache.guild(id).map(|cached| cached.member_count);
            let Some(count) = count else {
                guild.coverage = None;
                continue;
            };
            if timestamp - guild.last_tick > 90 {
                guild.coverage = None;
            }
            let result: anyhow::Result<()> = async {
                data.database()
                    .observe_membership(id.get(), timestamp, count)
                    .await?;
                match guild.coverage {
                    Some(coverage) => {
                        data.database()
                            .extend_membership_coverage(coverage, timestamp)
                            .await?
                    }
                    None => {
                        guild.coverage = Some(
                            data.database()
                                .start_membership_coverage(id.get(), timestamp)
                                .await?,
                        )
                    }
                }
                Ok(())
            }
            .await;
            guild.last_tick = timestamp;
            if let Err(error) = result {
                guild.coverage = None;
                tracing::warn!(guild_id = %id, %error, "failed to sample membership");
            }
        }
    }
}

pub async fn run_sampler(ctx: Context, data: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        data.membership_tracker().sample(&ctx, &data).await;
    }
}
