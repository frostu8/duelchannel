-- Adds a fixed-games provisional counter to users and rating snapshots.

ALTER TABLE user ADD COLUMN matches_until_rated INTEGER NOT NULL DEFAULT 0;
ALTER TABLE rating ADD COLUMN matches_until_rated INTEGER NOT NULL DEFAULT 0;

-- Backfill user counter - matches since season start. This is the easiest one
-- since it's just matches after a certain point
UPDATE user
SET matches_until_rated = (
    SELECT MAX(0, 10 - COALESCE(count, 0))
    FROM (
        SELECT COUNT(*) AS count
        FROM participant p, battle b
        WHERE b.id = p.match_id
            AND p.user_id = user.id
            AND b.status = 1
            AND b.concluded_at >= (
                SELECT MIN(inserted_at) FROM rating_period
            )
    )
);

-- Backfill rating snapshots - this is a lot harder. Each rating record records
-- the rating of a player at the *start* of a rating period, so it should
-- account for everything from the start of the last rating period to the end
-- of the last rating period. This slightly misleading detail is an artifact of
-- new rating insertions when a player joins for the first time.

-- Set a default for all of them
UPDATE rating SET matches_until_rated = 10;

-- Do the actual rating
UPDATE rating
SET matches_until_rated = MAX(0, 10 - COALESCE(count, 0))
FROM (
    SELECT r.id, COUNT(*) AS count
    FROM rating r, rating_period rp, participant p, battle b
    -- Get the rating period before this one to find where it starts
    WHERE b.id = p.match_id
        AND r.period_id = rp.id
        AND p.user_id = r.user_id
        AND b.status = 1
        -- Get matches from before the rating period ended
        AND b.concluded_at <= rp.inserted_at
		-- ..and after the season started
		AND b.concluded_at >= (SELECT MIN(inserted_at) FROM rating_period)
    GROUP BY r.id
) AS detail
WHERE detail.id = rating.id;
