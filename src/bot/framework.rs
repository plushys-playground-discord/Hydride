use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::Context;
use poise::{
    BoxFuture,
    serenity_prelude::{ClientBuilder, GatewayIntents, GuildId, Message, UserId},
};
use serenity::all::ChannelId;
use tracing::{error, info};

use crate::{
    bot::{activity, event_handler::Handler},
    commands,
    state::AppState,
};

pub async fn run(state: AppState) -> anyhow::Result<()> {
    let token = state.config().discord.token.clone();
    let owners: HashSet<UserId> = state
        .config()
        .discord
        .owner_ids
        .iter()
        .copied()
        .map(UserId::new)
        .collect();

    let setup_state = state.clone();

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: commands::all(),
            event_handler: |ctx, event, framework, data| {
                Box::pin(crate::bot::events::handle(ctx, event, framework, data))
            },
            prefix_options: poise::PrefixFrameworkOptions {
                prefix: None,
                stripped_dynamic_prefix: Some(case_insensitive_prefix),
                mention_as_prefix: true,
                edit_tracker: Some(Arc::new(poise::EditTracker::for_timespan(
                    Duration::from_mins(5), // 5 minutes
                ))),
                ..Default::default()
            },

            owners,
            // This code is run before every command
            pre_command: |ctx| {
                Box::pin(async move {
                    let channel_name = &ctx
                        .channel_id()
                        .name(&ctx)
                        .await
                        .unwrap_or_else(|_| "<unknown>".to_owned());
                    let author = &ctx.author().name;

                    info!(
                        "{} in {} used slash command '{}'",
                        author,
                        channel_name,
                        &ctx.invoked_command_name()
                    );
                })
            },
            // This code is run after a command if it was successful (returned Ok)
            post_command: |ctx| {
                Box::pin(async move {
                    info!("Executed command {}!", ctx.command().qualified_name);
                })
            },
            on_error: |err| {
                Box::pin(async move {
                    if let Some(ctx) = err.ctx() {
                        error!("Command error: {err}");

                        if let Some(channel) = ctx.data().config().moderation.error_log_channel {
                            let _ = ChannelId::new(channel)
                                .say(ctx.serenity_context(), format!("Command error: {err}"))
                                .await;
                        }
                    } else {
                        error!("Framework error: {}", err);
                    }
                    if let Err(e) = poise::builtins::on_error(err).await {
                        tracing::error!("Fatal error while sending error message: {}", e);
                    }
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            let state = setup_state.clone();
            Box::pin(async move {
                tracing::info!(bot = %ready.user.tag(), "registering commands");
                // run_startup_activity_reports(ctx, state.config()).await;

                register_application_commands(ctx, framework, state.config()).await?;
                crate::commands::moderation::reconcile_expired_tempbans(ctx, &state, ready.user.id)
                    .await
                    .context("failed to reconcile expired tempbans")?;
                tokio::spawn(crate::bot::membership::run_sampler(
                    ctx.clone(),
                    state.clone(),
                ));
                Ok(state)
            })
        })
        .build();

    let intents = GatewayIntents::non_privileged()
        | GatewayIntents::MESSAGE_CONTENT
        | GatewayIntents::GUILD_MEMBERS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_MODERATION;

    let handler = Handler::new(state.clone());
    let mut client = ClientBuilder::new(token, intents)
        .event_handler(handler)
        .framework(framework)
        .await
        .context("failed to create Discord client")?;

    client.start().await.context("Discord client exited")
}

fn case_insensitive_prefix<'a>(
    _ctx: &'a poise::serenity_prelude::Context,
    msg: &'a Message,
    state: &'a AppState,
) -> BoxFuture<'a, Result<Option<(&'a str, &'a str)>, anyhow::Error>> {
    Box::pin(async move {
        let prefix = state.config().discord.prefix.as_str();
        let content = msg.content.as_str();

        if content.len() < prefix.len() {
            return Ok(None);
        }

        let Some((candidate, rest)) = content.split_at_checked(prefix.len()) else {
            return Ok(None);
        };
        if candidate.eq_ignore_ascii_case(prefix) {
            Ok(Some((candidate, rest)))
        } else {
            Ok(None)
        }
    })
}

async fn register_application_commands(
    ctx: &poise::serenity_prelude::Context,
    framework: &poise::Framework<AppState, anyhow::Error>,
    config: &Arc<crate::config::BootstrapConfig>,
) -> anyhow::Result<()> {
    if config.discord.register_globally {
        poise::builtins::register_globally(ctx, &framework.options().commands).await?;
    } else {
        for guild_id in &config.discord.dev_guild_ids {
            poise::builtins::register_in_guild(
                ctx,
                &framework.options().commands,
                GuildId::new(*guild_id),
            )
            .await?;
        }
    }

    Ok(())
}

#[allow(dead_code)]
async fn run_startup_activity_reports(
    ctx: &poise::serenity_prelude::Context,
    config: &Arc<crate::config::BootstrapConfig>,
) {
    for guild_id in &config.discord.dev_guild_ids {
        if let Err(error) =
            activity::run_startup_role_activity_report(ctx, GuildId::new(*guild_id)).await
        {
            tracing::warn!(
                guild_id = *guild_id,
                %error,
                "startup role activity report failed"
            );
        }
    }
}
