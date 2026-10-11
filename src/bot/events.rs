use crate::state::AppState;

pub async fn handle(
    _ctx: &poise::serenity_prelude::Context,
    event: &poise::serenity_prelude::FullEvent,
    _framework: poise::FrameworkContext<'_, AppState, anyhow::Error>,
    data: &AppState,
) -> Result<(), anyhow::Error> {
    if let Err(error) = data.membership_tracker().handle(_ctx, event, data).await {
        tracing::warn!(%error, "failed to track membership event");
    }
    match event {
        poise::serenity_prelude::FullEvent::Ready { data_about_bot } => {
            tracing::info!(user = %data_about_bot.user.tag(), "Discord gateway ready");
        }
        poise::serenity_prelude::FullEvent::Message { new_message } => {
            if let Err(error) = archive_message(data, new_message).await {
                tracing::warn!(
                    message_id = %new_message.id,
                    channel_id = %new_message.channel_id,
                    %error,
                    "failed to archive message"
                );
            }

            if let Err(error) =
                crate::domain::highlights::process_message(_ctx, data, new_message).await
            {
                tracing::warn!(
                    message_id = %new_message.id,
                    channel_id = %new_message.channel_id,
                    %error,
                    "failed to process highlights for message"
                );
            }
        }
        poise::serenity_prelude::FullEvent::MessageUpdate {
            new: Some(new_message),
            ..
        } => {
            if let Err(error) = archive_message(data, new_message).await {
                tracing::warn!(
                    message_id = %new_message.id,
                    channel_id = %new_message.channel_id,
                    %error,
                    "failed to refresh archived message"
                );
            }
        }
        poise::serenity_prelude::FullEvent::MessageDelete {
            deleted_message_id, ..
        } => {
            if let Err(error) = data
                .database()
                .delete_message_archive(deleted_message_id.get() as i64)
                .await
            {
                tracing::warn!(
                    message_id = %deleted_message_id,
                    %error,
                    "failed to delete archived message"
                );
            }
        }
        poise::serenity_prelude::FullEvent::MessageDeleteBulk {
            multiple_deleted_messages_ids,
            ..
        } => {
            for deleted_message_id in multiple_deleted_messages_ids {
                if let Err(error) = data
                    .database()
                    .delete_message_archive(deleted_message_id.get() as i64)
                    .await
                {
                    tracing::warn!(
                        message_id = %deleted_message_id,
                        %error,
                        "failed to delete archived message from bulk delete"
                    );
                }
            }
        }

        _ => {}
    }

    Ok(())
}

async fn archive_message(
    data: &AppState,
    message: &poise::serenity_prelude::Message,
) -> anyhow::Result<()> {
    let Some(guild_id) = message.guild_id else {
        return Ok(());
    };

    data.database()
        .upsert_message_archive(
            guild_id.get() as i64,
            message.channel_id.get() as i64,
            message.id.get() as i64,
            message.author.id.get() as i64,
            message.timestamp.unix_timestamp(),
            message.content.chars().count() as i64,
        )
        .await?;

    Ok(())
}
