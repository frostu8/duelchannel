-- I've done this so much I've gotten stockholmed by it
-- it's ok said NOBODY!!!
PRAGMA defer_foreign_keys = ON;

CREATE TABLE battle_new (
    id INTEGER PRIMARY KEY,
    -- The server the battle took place on.
    server_id INTEGER REFERENCES server(id),
    -- The unique identiifer of the battle.
    uuid CHAR(36) NOT NULL UNIQUE,
    -- The name of the level of the battle.
    level_name VARCHAR(255) NOT NULL,
    -- The internal identifier of the level (the map lumpname).
    level_id VARCHAR(255) NOT NULL,
    -- Level status.
    status INTEGER NOT NULL DEFAULT 0,
    -- The final overtimecheckpoints of the battle.
    margin_score INTEGER NOT NULL DEFAULT 0,
    -- The replay hash and filename of the replay.
    replay_hash CHAR(64),
    replay_filename VARCHAR(256),
    -- Whether the match contributes to player ratings.
    rated BOOLEAN NOT NULL,
    -- When the battle concluded.
    concluded_at TIMESTAMP,
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);

INSERT INTO battle_new (
    id, server_id, uuid, level_name, level_id, status, margin_score,
    replay_hash, replay_filename, concluded_at, inserted_at, updated_at, rated
)
SELECT
    b.id, b.server_id, b.uuid, b.level_name, b.level_id, b.status,
    b.margin_score, b.replay_hash, b.replay_filename, b.concluded_at,
    b.inserted_at, b.updated_at,
    (
        b.status = 1 -- the match was concluded normally
        OR (b.status = 2 AND COALESCE(MAX(p.finish_time) > 35 * 30, FALSE))
        -- the match was cancelled but 30 seconds passed
        -- TICRATE * 30s
    )
FROM battle b
LEFT JOIN participant p ON b.id = p.match_id
GROUP BY
    b.id, b.server_id, b.uuid, b.level_name, b.level_id, b.status,
    b.margin_score, b.replay_hash, b.replay_filename, b.concluded_at,
    b.inserted_at, b.updated_at;

DROP TABLE battle;
ALTER TABLE battle_new RENAME TO battle;

PRAGMA defer_foreign_keys = OFF;
