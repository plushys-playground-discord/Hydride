# Moderation Bot

A Discord moderation bot written in Rust with `poise`, `serenity`, and `sqlx`.

It supports both prefix commands and slash commands, stores moderation cases in SQLite, and sends audit-style moderation logs to a configured channel.

## Features

- Warn, timeout, kick, ban, and unban members
- Purge recent messages from a channel
- Persist moderation cases in SQLite
- View a single case or recent cases for a user
- Per-guild moderation settings stored in the database
- Configurable moderator roles in addition to Discord permissions
- Automatic database migrations on startup
- Slash command registration either globally or to development guilds only

## Tech Stack

- Rust 2024
- `poise` / `serenity` for Discord bot framework and API access
- `tokio` for async runtime
- `sqlx` with SQLite for persistence
- `tracing` for logging

## Configuration

The bot reads configuration from `config.toml` by default.

You can override the config path with:

```bash
MODBOT_CONFIG=/path/to/config.toml cargo run
```

Use `config.example.toml` as a starting point:

```toml
[discord]
token = "DISCORD_BOT_TOKEN"
prefix = "!"
owner_ids = []
dev_guild_ids = [123456789012345678]
register_globally = false

[database]
url = "sqlite://moderation_bot.db"

[logging]
filter = "info,serenity=warn,sqlx=warn"

[moderation]
default_log_channel_id = 123456789012345678
require_reason = true
ephemeral_slash_responses = true
max_case_results = 10
```

### Config Fields

`[discord]`

- `token`: Bot token
- `prefix`: Prefix for text commands
- `owner_ids`: Optional Discord user IDs treated as bot owners by `poise`
- `dev_guild_ids`: Guild IDs used for slash command registration when `register_globally = false`
- `register_globally`: Register application commands globally instead of per dev guild

`[database]`

- `url`: SQLite connection string

`[logging]`

- `filter`: `tracing_subscriber` env filter string

`[moderation]`

- `default_log_channel_id`: Default moderation log channel used if a guild-specific value is not set
- `require_reason`: Require a reason for moderation commands
- `ephemeral_slash_responses`: Send slash command confirmations ephemerally
- `max_case_results`: Maximum number of cases returned by the `cases` command

## Running

1. Copy `config.example.toml` to `config.toml`.
2. Fill in your Discord bot token and guild/channel IDs.
3. Run the bot:

```bash
cargo run
```

On startup, the bot:

- loads the bootstrap config
- initializes tracing
- connects to SQLite
- runs database migrations from `migrations/`
- registers slash commands
- starts the Discord gateway client

## Required Discord Intents

The bot enables these gateway intents:

- non-privileged intents
- `MESSAGE_CONTENT`
- `GUILD_MEMBERS`
- `GUILD_MESSAGES`
- `GUILD_MODERATION`

Make sure the matching privileged intents are enabled in the Discord developer portal if your bot needs them.

## Commands

All commands are available as both prefix commands and slash commands unless noted otherwise.

### Meta

- `ping`
- `help [command]`

### Moderation

- `warn <user> [reason]`
- `timeout <user> <duration> [reason]`
- `kick <user> [reason]`
- `ban <user> [reason]`
- `unban <user> [reason]`
- `purge [user] <count> [reason]`

Duration format for `timeout` supports `s`, `m`, `h`, and `d`, for example `30m`, `4h`, or `7d`.

### Cases

- `case <case_id>`
- `cases <user> [limit]`

### Configuration

- `config view`
- `config set_log_channel <channel>`
- `config clear_log_channel`
- `config set_require_reason <true|false>`
- `config set_ephemeral <true|false>`
- `config add_mod_role <role>`
- `config remove_mod_role <role>`

## Permissions Model

Moderation commands check both Discord permissions and guild-specific moderator roles.

- Guild owners are always allowed
- Administrators are allowed
- Users with the required Discord permission are allowed
- Users with a configured moderator role are allowed

The bot also checks role hierarchy before acting on a target member.

## Data Model

The SQLite schema includes:

- `guild_settings`: per-guild moderation settings
- `guild_mod_roles`: extra roles allowed to moderate
- `moderation_cases`: stored moderation actions
- `case_notes`: note records attached to cases

Each moderation action creates a case record. If audit log delivery succeeds, the audit message channel and message IDs are stored with the case.

## Audit Logging

Moderation actions are posted as embeds to the configured moderation log channel. Embeds include:

- action type
- moderator
- target
- creation time
- reason
- optional duration / expiry
- optional details

## Current Limitations

- `notes_enabled` and `appeals_enabled` are stored in guild settings but do not have full user-facing flows yet.
- `case_notes` support exists in the database and code, but there is no public command wired in for adding notes.
- `purge` with a specific target user is not complete yet; the filtered fetch helper still ends in `todo!()`.

## Development

Useful commands:

```bash
cargo fmt
cargo check
cargo run
```

For local development, keep `register_globally = false` and set `discord.dev_guild_ids` so slash commands update quickly in a test server.

### Activity comparison graphs

Use `/activity_graph [channel] [days]` (or the configured prefix followed by
`activity_graph`) to receive three PNG graphs comparing daily messages with
active members, total server membership, and joins/departures. The default is
`#general` and 30 days; lookbacks are clamped to 7–30 days. If `general` is missing
or duplicated, specify the channel explicitly. Text and announcement channels
are supported. Both you and the bot need View Channel and Read Message History.
Slash replies are private; prefix replies appear in the invoking channel.

All graphs share UTC calendar days and mark today as incomplete. Messages and
active authors exclude bot and webhook messages; membership metrics include all
accounts. Active members means unique authors in the selected channel, including
people who have since left. Message history counts messages still available in
Discord, so deleted messages are excluded. Large scans time out after ten minutes.

Membership collection begins at deployment and persists in the main SQLite
database. The bot samples the gateway guild member count once per minute and
records joins and departures through its existing GUILD_MEMBERS intent (which
must also be enabled in the Discord Developer Portal). Total membership is the
last observed count for each day, not a reconstruction of earlier membership.
Disconnects, restarts, and failed writes leave coverage gaps. A `~` marks partial
coverage (join/departure counts may undercount); `?` marks unavailable data.
Missing observations are never plotted as zero or connected across gaps. The
current day's membership coverage may be partial until the next sample.

Charts render locally using Plotters and PNG encoding; no external chart service
receives activity data. Rendering requires a system sans-serif font and the
platform font libraries used by Plotters' TTF backend. On Debian/Ubuntu install
`pkg-config libfontconfig1-dev libfreetype6-dev fonts-dejavu-core`; macOS uses
its system fonts. Keep global registration disabled when testing in a dev guild.

## Build locally and upload to Oracle Linux

The upload script builds an x86_64 Linux release on your Mac inside an emulated
Oracle Linux 8 Docker container. It requires Docker Desktop (or another Docker
setup with Buildx and `linux/amd64` emulation). Compilation never runs on the
server. Oracle Linux 8 is the build baseline; the script checks that the server
is x86_64 Oracle Linux 8 or newer, then checks its shared libraries before
replacing the executable.

```bash
export SERVER_IP='161.118.168.235'
export SSH_USER='opc'
export SSH_KEY='/Users/jayansunil/Downloads/ssh-key-2026-08-25 (1).key'

# Preview without connecting or building:
./scripts/upload.sh --dry-run

# Build locally and upload:
./scripts/upload.sh
```

The default upload directory is `/home/opc/moderation_bot`. Set `REMOTE_DIR` to
your existing bot directory if different. The script uploads only the compiled
binary; it preserves config and SQLite files. All migrations are embedded in
the binary and run on the bot's next startup. Keep your existing service's
working directory and `MODBOT_CONFIG` pointing at the existing configuration
and database locations.

Before the first upload, verify and accept the server SSH host key using your
normal SSH connection; the script uses batch mode and retains host-key checking.
The server needs runtime fonts and libraries for graphs:

```bash
ssh -i "$SSH_KEY" "$SSH_USER@$SERVER_IP"
sudo dnf install fontconfig freetype dejavu-sans-fonts
```

Uploads are checked with SHA-256 and installed by an atomic rename. An existing
binary is copied to `moderation_bot.previous-<timestamp>-<pid>` before replacement.
The bot process is left running until you restart it. To restart an **existing**
systemd service after upload (requires noninteractive sudo permission):

```bash
./scripts/upload.sh --restart moderation_bot.service
```

Use `./scripts/upload.sh --build-only` to build without SSH, or
`./scripts/upload.sh --skip-build` to upload the existing artifact at
`target/oracle-linux-x86_64/moderation_bot`. Override `ARTIFACT_DIR`, `RUST_VERSION`
(default `1.95.0`), or `BUILD_JOBS` (default `2`) as needed. An ARM or macOS binary
is rejected. Builds use `Cargo.lock` and a source-only staging context, excluding
local bot tokens, database files, and Cargo environment overrides. If the server
is Oracle Linux 7, this build baseline is incompatible and the script stops.
