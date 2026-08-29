-- Adds a cached rank column to users. Rank is derived from the ordinal and
-- only re-sorted in update_post_battle.

ALTER TABLE user ADD COLUMN rank VARCHAR(32);
