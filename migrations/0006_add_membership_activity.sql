CREATE TABLE membership_daily (
    guild_id INTEGER NOT NULL,
    day INTEGER NOT NULL,
    member_count INTEGER,
    observed_at INTEGER,
    joins INTEGER NOT NULL DEFAULT 0,
    departures INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (guild_id, day)
);
CREATE TABLE membership_coverage (
    id INTEGER PRIMARY KEY,
    guild_id INTEGER NOT NULL,
    started_at INTEGER NOT NULL,
    ended_at INTEGER NOT NULL
);
CREATE INDEX idx_membership_coverage_guild_time
    ON membership_coverage (guild_id, started_at, ended_at);
