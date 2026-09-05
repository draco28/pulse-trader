-- r1.s4.w4 — 0008 down: reconstruct the EXACT 0005 shape, or refuse.
--
-- ADR-0018 asks a down migration to be TRUTHFUL, which for this one means it has
-- to answer "can 0005 hold what is in these tables?" before it does anything. Four
-- states say no:
--
--   * a `pending` session — 0005 has no pre-call outcome to demote it to;
--   * a session tagged `inapplicable_advice`, `missing_backtest_inputs` or
--     `interrupted` — 0005's CHECK enumerates seven kinds and none of them means
--     any of these;
--   * a proposal carrying `accepted_run_id` — 0005 has no column for it;
--   * a proposal carrying an accept failure — likewise.
--
-- Each is refused, TRANSACTIONALLY and before a single row moves, so a downgrade
-- either restores 0005 exactly or leaves 0008 exactly as it was. It never coerces
-- a new state into an old tag (that would put a false reason in the audit trail —
-- the same argument that added `transport_failure` rather than reusing one of the
-- six) and it never drops the row that carries the state to make the downgrade
-- pass. `RAISE(ABORT, ...)` is trigger-only, hence the scratch table below: one
-- trigger per reason, so the refusal names WHICH state blocked it.
--
-- What IS discarded, knowingly: the `request_fingerprint` of a terminal row. Under
-- 0005 a settled session simply has no such field, the value is a single-flight key
-- for a turn that has already ended, and nothing reads it after the turn settles.
-- That is a lossless downgrade of a live guarantee, not a lost record.

-- ---------------------------------------------------------------------------
-- 0. Refuse anything 0005 cannot say.
-- ---------------------------------------------------------------------------
CREATE TABLE _0008_down_guard (reason TEXT NOT NULL);

CREATE TRIGGER _0008_down_guard_pending BEFORE INSERT ON _0008_down_guard
WHEN NEW.reason = 'pending_session'
BEGIN
  SELECT RAISE(ABORT, 'migration 0008 down: a pending coaching session has no representation under 0005; refusing rather than discarding a live claim');
END;

CREATE TRIGGER _0008_down_guard_failure_tag BEFORE INSERT ON _0008_down_guard
WHEN NEW.reason = 'new_failure_tag'
BEGIN
  SELECT RAISE(ABORT, 'migration 0008 down: a session records inapplicable_advice, missing_backtest_inputs or interrupted; 0005 enumerates none of these and coercing one into an old tag would falsify the audit trail');
END;

CREATE TRIGGER _0008_down_guard_accepted_run BEFORE INSERT ON _0008_down_guard
WHEN NEW.reason = 'accepted_run_link'
BEGIN
  SELECT RAISE(ABORT, 'migration 0008 down: an accepted proposal names its re-backtest run and 0005 has no column for it; refusing rather than dropping the link');
END;

CREATE TRIGGER _0008_down_guard_accept_failure BEFORE INSERT ON _0008_down_guard
WHEN NEW.reason = 'accept_failure_field'
BEGIN
  SELECT RAISE(ABORT, 'migration 0008 down: a proposal records a typed accept failure and 0005 has no column for it; refusing rather than dropping the record');
END;

INSERT INTO _0008_down_guard (reason)
SELECT 'pending_session' FROM coaching_sessions WHERE outcome = 'pending' LIMIT 1;

INSERT INTO _0008_down_guard (reason)
SELECT 'new_failure_tag' FROM coaching_sessions
WHERE failure_kind IN ('inapplicable_advice', 'missing_backtest_inputs', 'interrupted') LIMIT 1;

INSERT INTO _0008_down_guard (reason)
SELECT 'accepted_run_link' FROM coaching_proposals WHERE accepted_run_id IS NOT NULL LIMIT 1;

INSERT INTO _0008_down_guard (reason)
SELECT 'accept_failure_field' FROM coaching_proposals
WHERE accept_failure_stage IS NOT NULL OR accept_failure_detail IS NOT NULL LIMIT 1;

DROP TRIGGER _0008_down_guard_pending;
DROP TRIGGER _0008_down_guard_failure_tag;
DROP TRIGGER _0008_down_guard_accepted_run;
DROP TRIGGER _0008_down_guard_accept_failure;
DROP TABLE _0008_down_guard;

-- ---------------------------------------------------------------------------
-- 1. Drop 0008's rules, then rebuild 0005's tables (same rename ordering as up).
-- ---------------------------------------------------------------------------
DROP TRIGGER IF EXISTS coaching_proposals_transition;
DROP TRIGGER IF EXISTS coaching_proposals_accept_lineage_update;
DROP TRIGGER IF EXISTS coaching_proposals_accept_lineage_insert;
DROP TRIGGER IF EXISTS coaching_proposals_session_must_be_proposed;
DROP TRIGGER IF EXISTS coaching_sessions_lifecycle;

ALTER TABLE coaching_proposals RENAME TO coaching_proposals_0008;
ALTER TABLE coaching_sessions RENAME TO coaching_sessions_0008;
DROP INDEX IF EXISTS idx_coaching_sessions_run;

-- Byte-for-byte 0005's `coaching_sessions` (see 0005_coaching.up.sql for why each
-- constraint reads the way it does).
CREATE TABLE coaching_sessions (
  id                   TEXT PRIMARY KEY NOT NULL,
  backtest_run_id      TEXT NOT NULL REFERENCES backtest_run(id),
  strategy_version_id  TEXT NOT NULL REFERENCES strategy_version(id),
  created_at           TEXT NOT NULL,
  llm_call_id          TEXT REFERENCES llm_call(id),
  outcome              TEXT NOT NULL,
  failure_kind         TEXT,
  failure_detail       TEXT,
  schema_version       INTEGER NOT NULL,

  CHECK (outcome IN ('proposed', 'failed')),
  CHECK ((outcome = 'failed') = (failure_kind IS NOT NULL)),
  CHECK ((outcome = 'failed') = (failure_detail IS NOT NULL)),
  CHECK (failure_kind IS NULL OR failure_kind IN (
    'zero_calls',
    'several_calls',
    'malformed_arguments',
    'inapplicable_mutation',
    'provider_timeout',
    'context_overflow',
    'transport_failure'
  ))
);

INSERT INTO coaching_sessions
  (id, backtest_run_id, strategy_version_id, created_at, llm_call_id, outcome,
   failure_kind, failure_detail, schema_version)
SELECT
   id, backtest_run_id, strategy_version_id, created_at, llm_call_id, outcome,
   failure_kind, failure_detail, schema_version
FROM coaching_sessions_0008;

CREATE TABLE coaching_proposals (
  id                TEXT PRIMARY KEY NOT NULL,
  session_id        TEXT NOT NULL UNIQUE REFERENCES coaching_sessions(id),
  mutation          TEXT NOT NULL,
  hypothesis        TEXT NOT NULL,
  disposition       TEXT NOT NULL,
  child_version_id  TEXT REFERENCES strategy_version(id),

  CHECK (disposition IN ('proposed', 'accepted', 'rejected', 'modified')),
  CHECK (
    length(trim(hypothesis, char(9, 10, 11, 12, 13, 32, 133, 160, 5760,
                                 8192, 8193, 8194, 8195, 8196, 8197, 8198,
                                 8199, 8200, 8201, 8202, 8232, 8233, 8239,
                                 8287, 12288))) > 0
  ),
  CHECK ((disposition = 'accepted') = (child_version_id IS NOT NULL))
);

INSERT INTO coaching_proposals
  (id, session_id, mutation, hypothesis, disposition, child_version_id)
SELECT
   id, session_id, mutation, hypothesis, disposition, child_version_id
FROM coaching_proposals_0008;

DROP TABLE coaching_proposals_0008;
DROP TABLE coaching_sessions_0008;

CREATE INDEX idx_coaching_sessions_run ON coaching_sessions(backtest_run_id, created_at);
