-- Adds a field that tells the public API to hide a user's rating
-- (probably because their DR is still calibrating)
ALTER TABLE user ADD COLUMN hide_rating BOOLEAN NOT NULL DEFAULT FALSE;
