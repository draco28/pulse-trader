-- r1.s2.w2 — 0005 down: reverse-order teardown of the coaching schema.
--
-- Drops the index, then `coaching_proposals` (the child, whose FK points at
-- `coaching_sessions`), then `coaching_sessions`, then the `llm_call.prompt_version`
-- column. Each DROP uses IF EXISTS for idempotent re-runs (mirrors
-- `0001_init.down.sql` / `0004_llm_call.down.sql`).
--
-- `ALTER TABLE ... DROP COLUMN` needs SQLite >= 3.35; sqlx `=0.8.6` bundles a newer
-- engine than that, and `0007_llm_call_key_source.down.sql` already relies on it.
--
-- Reversing this migration discards every recorded coaching turn and every
-- coach-prompt version stamped since it was applied. That is the honest semantics
-- of an undo here — these tables and that column are the only place the record
-- lives — and it is why the undo path exists for recovery, not for routine use.

DROP INDEX IF EXISTS idx_coaching_sessions_run;
DROP TABLE IF EXISTS coaching_proposals;
DROP TABLE IF EXISTS coaching_sessions;
ALTER TABLE llm_call DROP COLUMN prompt_version;
