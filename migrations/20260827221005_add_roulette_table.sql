-- Add a table for tracking roulette pulls
CREATE TABLE roulette (
    id INTEGER PRIMARY KEY,
    participant_id INTEGER NOT NULL REFERENCES participant(id),
    -- The id of the item.
    item VARCHAR(60) NOT NULL,
    -- The roulette item's multiplicity.
    multiplicity INTEGER NOT NULL,
    -- How many times the item was pulled.
    count INTEGER NOT NULL DEFAULT 0,

    CONSTRAINT item_multiplicity_unique UNIQUE (item, multiplicity)
);
