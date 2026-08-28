-- FUCK YOU SQLITE DEVELOPERS!!!!
PRAGMA defer_foreign_keys = ON;

-- Rebuild the user table with ordinal as REAL.
CREATE TABLE user_new (
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
    ordinal REAL,
    -- Whether to hide the user's rating from the public API.
    hide_rating BOOLEAN NOT NULL DEFAULT FALSE,
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

INSERT INTO user_new
    (id, short_id, display_name, avatar_url, flags, ordinal, inserted_at, updated_at, hide_rating)
SELECT
    id, short_id, display_name, avatar_url, flags, ordinal, inserted_at, updated_at, hide_rating
FROM user;

DROP TABLE user;
ALTER TABLE user_new RENAME TO user;

-- Rebuild battle_statistics with avg_mmr as REAL.
CREATE TABLE battle_statistics_new (
    id INTEGER PRIMARY KEY,
    match_id INTEGER NOT NULL REFERENCES battle(id),
    avg_mmr REAL,
    quality REAL,
    finish_time INTEGER,
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

INSERT INTO battle_statistics_new
    (id, match_id, avg_mmr, quality, finish_time, inserted_at, updated_at)
SELECT
    id, match_id, avg_mmr, quality, finish_time, inserted_at, updated_at
FROM battle_statistics;

DROP TABLE battle_statistics;
ALTER TABLE battle_statistics_new RENAME TO battle_statistics;

PRAGMA defer_foreign_keys = OFF;
