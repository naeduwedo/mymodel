-- 0002_app_metadata.sql - record which application build last used this database.
--
-- Why this exists: an upgrade that fails needs to be diagnosed after the fact.
-- Knowing "schema v2 was written by app 0.1.0" turns "it broke" into a
-- reproducible report, and gives the updater a place to record a migration that
-- was rolled forward or backed out.

CREATE TABLE app_metadata (
    key        TEXT NOT NULL PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

-- Seed the row so the table is never ambiguous about its own meaning.
INSERT INTO app_metadata (key, value, updated_at)
VALUES ('schema_origin', '0002_app_metadata', CAST(strftime('%s','now') AS INTEGER) * 1000);
