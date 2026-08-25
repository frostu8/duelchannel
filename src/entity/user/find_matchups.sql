-- SQL script for finding all matchups a player has played in a period.
-- Inputs:
--   $1: id of user
--   $2: time from
--   $3: time to
-- Outputs: opponent rating r.*, b.status, posiiton, mw.finish_time

WITH recent_ratings AS (
    SELECT r.*
    FROM
        rating r
    WHERE
    	r.period_id = (
    		SELECT id
    		FROM rating_period
            WHERE inserted_at < $3
    		ORDER BY inserted_at DESC
    		LIMIT 1
    	)
)
SELECT
    r.*,
    b.status,
    -- +1 to correct for self
    COUNT(*) + 1 - COUNT(NOT op.no_contest AND me.finish_time < op.finish_time) AS position,
    IIF(MIN(op.finish_time) IS NOT NULL, MIN(op.finish_time), me.finish_time) AS finish_time,
    me.no_contest
FROM
    battle b, participant op, participant me, recent_ratings r
WHERE
    me.match_id = b.id
    AND op.match_id = b.id
    AND op.user_id  = r.user_id
    -- Filter out opponents and "me"
    AND me.user_id  = $1
    AND NOT op.user_id = $1
    -- Only get matches between the bounds
    AND b.concluded_at >= $2
    AND b.concluded_at < $3
-- Group by battles to count how many we are ahead
GROUP BY b.id, b.status, b.inserted_at, me.finish_time, me.no_contest
-- we only want matches where two players participated
HAVING COUNT(*) = 1
ORDER BY b.inserted_at ASC
