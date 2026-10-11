use sqlx::{FromRow, query, query_as};

use super::Database;

#[derive(Debug, Clone, FromRow)]
pub struct MembershipDay {
    pub day: i64,
    pub member_count: Option<i64>,
    pub observed_at: Option<i64>,
    pub joins: i64,
    pub departures: i64,
}

impl Database {
    pub async fn observe_membership(&self, guild: u64, now: i64, count: u64) -> anyhow::Result<()> {
        query(
            "INSERT INTO membership_daily (guild_id, day, member_count, observed_at)
               VALUES (?1, ?2, ?3, ?4) ON CONFLICT(guild_id, day) DO UPDATE SET
               member_count = excluded.member_count, observed_at = excluded.observed_at
               WHERE excluded.observed_at >= COALESCE(membership_daily.observed_at, 0)",
        )
        .bind(guild as i64)
        .bind(now.div_euclid(86400) * 86400)
        .bind(count as i64)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_membership_event(
        &self,
        guild: u64,
        now: i64,
        joining: bool,
    ) -> anyhow::Result<()> {
        query(
            "INSERT INTO membership_daily (guild_id, day, joins, departures)
               VALUES (?1, ?2, ?3, ?4) ON CONFLICT(guild_id, day) DO UPDATE SET
               joins = joins + excluded.joins, departures = departures + excluded.departures",
        )
        .bind(guild as i64)
        .bind(now.div_euclid(86400) * 86400)
        .bind(i64::from(joining))
        .bind(i64::from(!joining))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn start_membership_coverage(&self, guild: u64, now: i64) -> anyhow::Result<i64> {
        Ok(query(
            "INSERT INTO membership_coverage (guild_id, started_at, ended_at) VALUES (?1, ?2, ?2)",
        )
        .bind(guild as i64)
        .bind(now)
        .execute(&self.pool)
        .await?
        .last_insert_rowid())
    }

    pub async fn extend_membership_coverage(&self, id: i64, now: i64) -> anyhow::Result<()> {
        query("UPDATE membership_coverage SET ended_at = MAX(ended_at, ?2) WHERE id = ?1")
            .bind(id)
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn membership_history(
        &self,
        guild: u64,
        start: i64,
        end: i64,
    ) -> anyhow::Result<(Vec<MembershipDay>, Vec<(i64, i64)>)> {
        let days = query_as::<_, MembershipDay>(
            "SELECT day, member_count, observed_at, joins, departures
            FROM membership_daily WHERE guild_id = ?1 AND day >= ?2 AND day < ?3 ORDER BY day",
        )
        .bind(guild as i64)
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await?;
        let intervals = query_as::<_, (i64, i64)>(
            "SELECT started_at, ended_at FROM membership_coverage
            WHERE guild_id = ?1 AND ended_at >= ?2 AND started_at < ?3 ORDER BY started_at",
        )
        .bind(guild as i64)
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await?;
        Ok((days, intervals))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn membership_storage_preserves_counts_and_isolates_guilds() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "modbot-activity-{}-{}.db",
            std::process::id(),
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ));
        let db = Database::connect(&format!("sqlite://{}", dir.display())).await?;
        db.migrate().await?;
        db.observe_membership(1, 86401, 10).await?;
        db.observe_membership(1, 86400, 99).await?;
        db.record_membership_event(1, 86402, true).await?;
        db.record_membership_event(1, 86403, false).await?;
        db.record_membership_event(1, 172800, true).await?;
        let id = db.start_membership_coverage(1, 86400).await?;
        db.extend_membership_coverage(id, 86460).await?;
        let (days, intervals) = db.membership_history(1, 86400, 172800).await?;
        assert_eq!(days.len(), 1);
        assert_eq!(days[0].member_count, Some(10));
        assert_eq!((days[0].joins, days[0].departures), (1, 1));
        assert_eq!(intervals, vec![(86400, 86460)]);
        assert!(db.membership_history(2, 0, 200000).await?.0.is_empty());
        db.pool.close().await;
        let reopened = Database::connect(&format!("sqlite://{}", dir.display())).await?;
        assert_eq!(
            reopened.membership_history(1, 86400, 172800).await?.0[0].member_count,
            Some(10)
        );
        reopened.pool.close().await;
        std::fs::remove_file(dir)?;
        Ok(())
    }
}
