-- Add migration script here
ALTER TABLE rating ADD COLUMN idle_periods REAL NOT NULL DEFAULT 0.0;
