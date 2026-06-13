-- VS-1.1.4 work-1.01 — 0001_init: the canonical SQLite schema all 1.03 queries
-- target. Ids TEXT (UUID hyphenated, human-inspectable); bools INTEGER 0/1;
-- timestamps TEXT (RFC3339/ISO-8601 UTC); JSON arrays TEXT; SchemaVersion TEXT.
--
-- `strategy.pinned_version_id REFERENCES strategy_version(id)` is a FORWARD
-- reference (the target table is declared below). SQLite resolves FK target names
-- lazily at row-insert/check time, not at CREATE TABLE time, so the
-- strategy-before-strategy_version order is valid; the column is nullable and only
-- set after a version exists, so there is no chicken-and-egg insert problem.

CREATE TABLE strategy (
  id                 TEXT PRIMARY KEY NOT NULL,
  name               TEXT NOT NULL,
  tags               TEXT NOT NULL DEFAULT '[]',           -- JSON array of strings
  owner              TEXT,                                 -- nullable
  pinned_version_id  TEXT REFERENCES strategy_version(id), -- nullable; set after a version exists
  archived           INTEGER NOT NULL DEFAULT 0,           -- bool
  created_at         TEXT NOT NULL
);

CREATE TABLE strategy_version (
  id                     TEXT PRIMARY KEY NOT NULL,
  strategy_id            TEXT NOT NULL REFERENCES strategy(id),
  parent_version_id      TEXT REFERENCES strategy_version(id),  -- nullable self-ref tree
  dsl_schema_version     TEXT NOT NULL,                         -- "MAJOR.MINOR.PATCH"
  dsl                    TEXT NOT NULL,                         -- migrated current DSL JSON (canonical query surface; reads re-derive from dsl_original — gate-7 C4)
  dsl_original           TEXT NOT NULL,                         -- VERBATIM pre-migration source bytes
  version_hash           TEXT NOT NULL,                         -- content hash (identity / integrity); NOT UNIQUE (gate-2 Q2)
  created_by             TEXT NOT NULL,                         -- CreatedBy enum text
  creating_llm_call_ids  TEXT NOT NULL DEFAULT '[]',            -- JSON array (no LLMCall table this slice)
  created_at             TEXT NOT NULL
);

CREATE INDEX idx_sv_strategy_id ON strategy_version(strategy_id);
CREATE INDEX idx_sv_parent      ON strategy_version(parent_version_id);

-- Immutability (FR-4): SQLite needs SEPARATE BEFORE UPDATE and BEFORE DELETE
-- triggers (no combined form). RAISE(ABORT, ...) rolls the statement back and
-- surfaces as a sqlx::Error → DataError::Db on the caller's side. This is the
-- DB-level half of the slice's dual immutability guard (the API half is 1.02/1.03).
CREATE TRIGGER strategy_version_no_update BEFORE UPDATE ON strategy_version
  BEGIN SELECT RAISE(ABORT, 'strategy_version is immutable'); END;
CREATE TRIGGER strategy_version_no_delete BEFORE DELETE ON strategy_version
  BEGIN SELECT RAISE(ABORT, 'strategy_version is immutable'); END;
