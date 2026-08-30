-- The score the player ended the battle with.
ALTER TABLE participant ADD COLUMN score INTEGER NOT NULL DEFAULT 0;
