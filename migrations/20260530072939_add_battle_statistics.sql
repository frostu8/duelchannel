-- This table stores cached battle statistics.
-- These are just kind of a pain to re-evaluate so they're lazily calculated.
CREATE TABLE battle_statistics (
    id INTEGER PRIMARY KEY,
    match_id INTEGER NOT NULL REFERENCES battle(id),
    avg_mmr INTEGER,
    quality REAL,
    finish_time INTEGER,
    inserted_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL
);
