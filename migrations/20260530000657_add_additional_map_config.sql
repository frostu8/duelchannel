-- Add alternate win-con, mmr ranges
ALTER TABLE map_config ADD COLUMN win_condition INTEGER;

ALTER TABLE map_config ADD COLUMN skill_lower INTEGER;
ALTER TABLE map_config ADD COLUMN skill_upper INTEGER;
