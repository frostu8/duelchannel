-- Players on Duel Channel. They may optionally authenticate with discord auth
-- (see below)
CREATE TABLE user (
    id INTEGER PRIMARY KEY,
    -- The short ID of the user.
    short_id CHAR(6) NOT NULL UNIQUE,
    -- The display name of the user.
    display_name VARCHAR(255) NOT NULL,
    -- The avatar URL of the user.
    avatar_url VARCHAR(255),
    -- User flags.
    flags INTEGER NOT NULL DEFAULT 0,
    -- The cached rating ordinal of the player.
    ordinal INTEGER,
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

-- Servers registered to the ingest.
CREATE TABLE server (
    id INTEGER PRIMARY KEY,
    -- The human-readable name of the server.
    server_name VARCHAR(255) NOT NULL UNIQUE,
    -- The hash of the key used to authenticate as the server.
    key_hash CHAR(64) NOT NULL UNIQUE,
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

-- Map configs for each server
CREATE TABLE map_config (
    id INTEGER PRIMARY KEY,
    parent_id INTEGER NOT NULL REFERENCES server(id),
    -- The name of the map
    lumpname VARCHAR(255) NOT NULL,
    -- The banned status of the map
    status INTEGER NOT NULL,
    -- A user-defined note of the map's configs
    note VARCHAR(2048),
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL,

    UNIQUE(parent_id, lumpname)
);

-- A single rating period on the client.
CREATE TABLE rating_period (
    id INTEGER PRIMARY KEY,
    -- When the rating period started
    inserted_at TIMESTAMP NOT NULL
);

-- The rating of a single user. Each row records the most recent rating of a
-- user.
CREATE TABLE rating (
    id INTEGER PRIMARY KEY,
    -- The period that this rating is valid for.
    period_id INTEGER NOT NULL REFERENCES rating_period(id),
    -- The ID of the user.
    user_id INTEGER NOT NULL REFERENCES user(id),
    -- Rating informaton
    rating REAL NOT NULL,
    deviation REAL NOT NULL,
    extra TEXT NOT NULL,
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL,

    UNIQUE(period_id, user_id)
);

-- Discord authentication for users.
CREATE TABLE discord_auth (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL UNIQUE REFERENCES user(id),
    -- The Discord snowflake of the user.
    discord_id BIGINT NOT NULL UNIQUE,
    -- The refresh token of the user.
    refresh_token VARCHAR(255) NOT NULL,
    -- When the discord data was last fetched.
    last_fetched_at TIMESTAMP NOT NULL,
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

-- A player, representing a single profile in Ring Racers.
CREATE TABLE profile (
    id INTEGER PRIMARY KEY,
    -- The parent user ID of the profile.
    parent_id INTEGER NOT NULL REFERENCES user(id),
    -- The public key of their profile.
    public_key CHAR(64) NOT NULL UNIQUE,
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

-- A single battle between players.
CREATE TABLE battle (
    id INTEGER PRIMARY KEY,
    -- The server the battle took place on.
    server_id INTEGER NOT NULL REFERENCES server(id),
    -- The unique identiifer of the battle.
    uuid CHAR(36) NOT NULL UNIQUE,
    -- The name of the level of the battle.
    level_name VARCHAR(255) NOT NULL,
    -- Level status.
    status INTEGER NOT NULL DEFAULT 0,
    -- The final overtimecheckpoints of the battle.
    margin_score INTEGER NOT NULL DEFAULT 0,
    -- The replay hash and filename of the replay.
    replay_hash CHAR(64),
    replay_filename VARCHAR(256),
    -- When the battle concluded.
    concluded_at TIMESTAMP,
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

-- A player in a battle.
CREATE TABLE participant (
    id INTEGER PRIMARY KEY,
    -- The battle the player played in.
    match_id INTEGER NOT NULL REFERENCES battle(id),
    -- The profile of the participant.
    profile_id INTEGER NOT NULL REFERENCES profile(id),
    -- The user playing on the profile.
    user_id INTEGER NOT NULL REFERENCES user(id),
    -- The display name of the player at the time of the match.
    name VARCHAR(255) NOT NULL,
    -- The team the player was on.
    team INTEGER NOT NULL,
    -- The finish time of the player.
    finish_time INTEGER,
    -- Whether or not the player no-contest'd
    no_contest BOOLEAN NOT NULL DEFAULT FALSE,
    -- The skin of the player.
    skin VARCHAR(255) NOT NULL,
    -- The human-readable name of the skin.
    skin_name VARCHAR(255) NOT NULL,
    -- The player's kart speed at the time of the match.
    kart_speed INTEGER NOT NULL,
    -- The player's kart weight at the time of the match.
    kart_weight INTEGER NOT NULL,

    UNIQUE(match_id, profile_id)
);
