-- r1.s1.w2 — 0007 down: drop the `key_source` provenance column.
--
-- `ALTER TABLE ... DROP COLUMN` needs SQLite >= 3.35; sqlx `=0.8.6` bundles a
-- newer engine than that, and `tests/migration_roundtrip.rs` proves the up/down
-- round trip with 0007 in the embedded set.
--
-- Reversing this migration discards recorded provenance for every call made since
-- it was applied. That is the honest semantics of an undo here — the column is the
-- only place the label lives — and it is why the undo path exists for recovery,
-- not for routine use.

ALTER TABLE llm_call DROP COLUMN key_source;
