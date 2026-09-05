-- r1.s4.w4 — 0008: the coach LIFECYCLE the merged 0005 schema cannot represent
-- (ADR-0010 / ADR-0018 / ADR-0019 / ADR-0021 as amended by this item).
--
-- WHY A SECOND MIGRATION, WHEN 0005 SAID THERE WOULD NOT BE ONE.
-- `0005` shipped the disposition columns dormant on the stated bet that `r1.s4`
-- would exercise them "without a second migration". Planning `r1.s4` proved the
-- bet wrong in four specific, verified places, and each one is a state the coach
-- rail must be able to STORE rather than merely intend:
--
--   1. A SESSION ID CLAIMED BEFORE THE PROVIDER CALL. `0005`'s
--      `outcome IN ('proposed','failed')` has no pre-call state at all, so the
--      only way to make the turn idempotent was to write the row AFTER the call —
--      which is exactly the window in which a crash leaves the silent turn release
--      exit criterion 4 forbids.
--   2. TWO NEW HONEST FAILURES. `inapplicable_advice` (the coach answered with
--      structural advice the `r1` parameter-only vocabulary cannot express, #131)
--      and `missing_backtest_inputs` (the parent run's exact snapshot selection is
--      gone) are neither of the seven enumerated tags, and recording either as one
--      of them would put a false reason in the audit trail.
--   3. AN ACCEPTED PROPOSAL'S RUN. `0005` stores `child_version_id` and has no
--      column for the re-backtest of that child, so "no child lacks its run" was
--      unrepresentable, not merely unenforced.
--   4. A FAILED ACCEPT. An accept that dies at apply/compile/backtest has to be a
--      recorded, typed outcome on the proposal — not an absence a reader has to
--      infer from a missing child.
--
-- ADR-0018 IS HONOURED, NOT SIDESTEPPED. `0005` is applied history and is NOT
-- edited: this migration rebuilds its two tables forward. `0008` is the next free
-- number after `0007_llm_call_key_source`, and the shipped set stays contiguous.
--
-- THE REBUILD IS FK-SAFE WITHOUT TOUCHING `PRAGMA foreign_keys`. That pragma is a
-- NO-OP inside a transaction, and sqlx runs this file in one, so the usual
-- "12-step ALTER TABLE" recipe is not available. The rename ordering below does the
-- same job with the pragma left alone: renaming `coaching_sessions` while
-- `coaching_proposals` has ALREADY been renamed makes SQLite rewrite the child's
-- `REFERENCES` clause to the archived parent (the post-3.26 behaviour), so the two
-- archived tables reference each other, the two new tables reference each other,
-- and neither DROP ever severs a live edge. A failure anywhere rolls the whole file
-- back.
--
-- CONVENTIONS ARE 0005's, deliberately: `TEXT` keys, RFC3339 UTC `created_at`,
-- serde-JSON payload columns for typed domain values, an asserted `schema_version`
-- row tag, and no `f64`-typed column anywhere (NFR-2). The two tables remain
-- non-append-only — a proposal's disposition is UPDATEd by the rail — so the
-- guarantee here is that every legal state is representable and every illegal one
-- is refused, not that a row can never be touched.

-- ---------------------------------------------------------------------------
-- 0. Pre-flight: VERIFY the dormant-row claim rather than trusting it.
-- ---------------------------------------------------------------------------
-- Under `0005` an accepted proposal necessarily names a child and CANNOT name a
-- run — the column does not exist. `r1.s2` wrote only the `proposed` state, so no
-- such row should exist; if one does, this migration has no truthful value to put
-- in `accepted_run_id` and refuses rather than inventing a run link or silently
-- demoting the disposition. `RAISE(ABORT, ...)` is only legal inside a trigger,
-- hence the scratch table: the conditional INSERT fires it exactly when the
-- offending row exists.
CREATE TABLE _0008_preflight (reason TEXT NOT NULL);

CREATE TRIGGER _0008_preflight_refuse BEFORE INSERT ON _0008_preflight
BEGIN
  SELECT RAISE(
    ABORT,
    'migration 0008: an accepted coaching proposal already carries a child version with no run; 0008 refuses to invent a run link'
  );
END;

INSERT INTO _0008_preflight (reason)
SELECT 'accepted_without_run' FROM coaching_proposals WHERE disposition = 'accepted' LIMIT 1;

DROP TRIGGER _0008_preflight_refuse;
DROP TABLE _0008_preflight;

-- ---------------------------------------------------------------------------
-- 1. Archive the 0005 tables (child first — see the rename note in the header).
-- ---------------------------------------------------------------------------
ALTER TABLE coaching_proposals RENAME TO coaching_proposals_0005;
ALTER TABLE coaching_sessions RENAME TO coaching_sessions_0005;

-- The index followed its table through the rename and still owns the name the
-- rebuilt table needs.
DROP INDEX IF EXISTS idx_coaching_sessions_run;

-- ---------------------------------------------------------------------------
-- 2. `coaching_sessions`, rebuilt with the claim state.
-- ---------------------------------------------------------------------------
CREATE TABLE coaching_sessions (
  id                   TEXT PRIMARY KEY NOT NULL,
  backtest_run_id      TEXT NOT NULL REFERENCES backtest_run(id),      -- the run the coach READ
  strategy_version_id  TEXT NOT NULL REFERENCES strategy_version(id),  -- whose DSL the proposal mutates
  created_at           TEXT NOT NULL,                                  -- injected Clock (RFC3339 UTC)
  llm_call_id          TEXT REFERENCES llm_call(id),                   -- NULL when no ledger row was correlated
  outcome              TEXT NOT NULL,                                  -- 'pending' | 'proposed' | 'failed'
  failure_kind         TEXT,
  failure_detail       TEXT,
  schema_version       INTEGER NOT NULL,
  -- The single-flight key: an opaque lowercase SHA-256 over the turn's request
  -- inputs, computed by w1 and only STORED and COMPARED here. NULLABLE, and the
  -- CHECK below is what makes that safe: it is REQUIRED for every `pending` row
  -- (there is no claim without one) and absent on the two terminal shapes that
  -- legitimately have none — the `0005` rows copied in below, and the direct
  -- terminal writes `save_session` still makes during Round 1, before w1 retires
  -- that production bypass. A NOT NULL column would have made those rows
  -- unmigratable and this migration a data-loss event.
  request_fingerprint  TEXT,

  -- The outcome vocabulary, enumerated in-schema so a typo cannot become a state.
  -- `pending` is the pre-call claim 0005 could not express.
  CHECK (outcome IN ('pending', 'proposed', 'failed')),

  -- A claim without a fingerprint is not a claim: it is a row nothing can ever
  -- match, so the single-flight guarantee it exists to provide would silently not
  -- hold. The trim char-set is 0005's `hypothesis` set — every scalar Rust's
  -- `char::is_whitespace` accepts — for the same reason it was spelled out there:
  -- SQLite's one-argument `trim()` strips SPACES ONLY, so a fingerprint of a
  -- single tab would pass a naive check and then be refused by the domain
  -- constructor at read time, stranding the row forever.
  CHECK (
    outcome <> 'pending'
    OR (request_fingerprint IS NOT NULL
        AND length(trim(request_fingerprint, char(9, 10, 11, 12, 13, 32, 133, 160, 5760,
                                                   8192, 8193, 8194, 8195, 8196, 8197, 8198,
                                                   8199, 8200, 8201, 8202, 8232, 8233, 8239,
                                                   8287, 12288))) > 0)
  ),

  -- A claim PRECEDES the provider call, so it names no ledger row. (It carries no
  -- failure either; the iff pair below already says so.)
  CHECK (outcome <> 'pending' OR llm_call_id IS NULL),

  -- Never silence, at the SQL layer (0005's rule, unchanged): a failed turn
  -- carries BOTH its kind and its detail; `proposed` and `pending` carry NEITHER.
  -- `(a) = (b)` compares SQLite's 0/1 booleans, so this is an iff.
  CHECK ((outcome = 'failed') = (failure_kind IS NOT NULL)),
  CHECK ((outcome = 'failed') = (failure_detail IS NOT NULL)),

  -- The failure taxonomy: 0005's seven, plus the three this item makes honest.
  -- `interrupted` is the finalization of a claim left by an earlier process
  -- lifetime — recorded WITHOUT a second provider call, because the turn really
  -- did end without an answer and pretending otherwise would bill a lie.
  CHECK (failure_kind IS NULL OR failure_kind IN (
    'zero_calls',
    'several_calls',
    'malformed_arguments',
    'inapplicable_mutation',
    'provider_timeout',
    'context_overflow',
    'transport_failure',
    'inapplicable_advice',
    'missing_backtest_inputs',
    'interrupted'
  ))
);

INSERT INTO coaching_sessions
  (id, backtest_run_id, strategy_version_id, created_at, llm_call_id, outcome,
   failure_kind, failure_detail, schema_version, request_fingerprint)
SELECT
   id, backtest_run_id, strategy_version_id, created_at, llm_call_id, outcome,
   failure_kind, failure_detail, schema_version, NULL
FROM coaching_sessions_0005;

-- ---------------------------------------------------------------------------
-- 3. `coaching_proposals`, rebuilt with the accepted run link and typed failure.
-- ---------------------------------------------------------------------------
CREATE TABLE coaching_proposals (
  id                    TEXT PRIMARY KEY NOT NULL,
  session_id            TEXT NOT NULL UNIQUE REFERENCES coaching_sessions(id),
  mutation              TEXT NOT NULL,
  hypothesis            TEXT NOT NULL,
  disposition           TEXT NOT NULL,
  child_version_id      TEXT REFERENCES strategy_version(id),
  -- The re-backtest OF the accepted child. Together with `child_version_id` this
  -- is the release's "no accepted proposal lacks its child and no child lacks its
  -- run" written as schema instead of as intent.
  accepted_run_id       TEXT REFERENCES backtest_run(id),
  -- The LATEST accept outcome, on the existing mutable proposal projection — not a
  -- new append-only decision-attempt entity. A later valid modify clears it; a
  -- successful accept clears it inside the same transaction that writes the child.
  accept_failure_stage  TEXT,
  accept_failure_detail TEXT,

  CHECK (disposition IN ('proposed', 'accepted', 'rejected', 'modified')),

  -- 0005's hypothesis rule, carried across verbatim (see 0005 for the char-set
  -- derivation and the `migration_0005` test that keeps it in parity with Rust).
  CHECK (
    length(trim(hypothesis, char(9, 10, 11, 12, 13, 32, 133, 160, 5760,
                                 8192, 8193, 8194, 8195, 8196, 8197, 8198,
                                 8199, 8200, 8201, 8202, 8232, 8233, 8239,
                                 8287, 12288))) > 0
  ),

  -- Both links exist exactly when the proposal is accepted, and nothing else may
  -- name either. This is 0005's child rule PLUS the run half it could not state.
  CHECK ((disposition = 'accepted') = (child_version_id IS NOT NULL)),
  CHECK ((disposition = 'accepted') = (accepted_run_id IS NOT NULL)),

  -- The accept progression w2/w3 report, enumerated. There is deliberately NO
  -- `read_back` stage: once the child and the run are committed the accept
  -- SUCCEEDED, and a read-back failure is a saved-but-unreadable accepted outcome
  -- carrying both ids (the r1.s3 precedent) — a shape the accepted row below
  -- forbids from carrying failure fields at all.
  CHECK (accept_failure_stage IS NULL OR accept_failure_stage IN (
    'apply', 'load_inputs', 'load_snapshots', 'compile', 'backtest', 'persist'
  )),
  -- Half a failure is silence wearing a record's clothes.
  CHECK ((accept_failure_stage IS NULL) = (accept_failure_detail IS NULL)),
  -- A failed accept leaves the proposal OPEN. An accepted row therefore has no
  -- accept-failure fields, and a rejected row has no child, no run and no failure.
  CHECK (accept_failure_stage IS NULL OR disposition IN ('proposed', 'modified'))
);

INSERT INTO coaching_proposals
  (id, session_id, mutation, hypothesis, disposition, child_version_id,
   accepted_run_id, accept_failure_stage, accept_failure_detail)
SELECT
   id, session_id, mutation, hypothesis, disposition, child_version_id,
   NULL, NULL, NULL
FROM coaching_proposals_0005;

-- ---------------------------------------------------------------------------
-- 4. Retire the archived tables (child first, so no DROP severs a live edge).
-- ---------------------------------------------------------------------------
DROP TABLE coaching_proposals_0005;
DROP TABLE coaching_sessions_0005;

-- The coach's read pattern, unchanged from 0005: sessions for one run, most
-- recent last.
CREATE INDEX idx_coaching_sessions_run ON coaching_sessions(backtest_run_id, created_at);

-- ---------------------------------------------------------------------------
-- 5. The rules a CHECK cannot express, because they read another row.
-- ---------------------------------------------------------------------------

-- A session's identity and its single settling move.
--
-- `pending -> proposed | failed` happens exactly once. After that the row is a
-- record: its identity, its fingerprint and its outcome are the audit trail, and
-- an UPDATE that edits any of them is rewriting history rather than continuing it.
-- `IS NOT` (not `<>`) on the nullable fingerprint, so NULL-to-value is caught too.
CREATE TRIGGER coaching_sessions_lifecycle BEFORE UPDATE ON coaching_sessions
BEGIN
  SELECT CASE WHEN NEW.id <> OLD.id
              OR NEW.backtest_run_id <> OLD.backtest_run_id
              OR NEW.strategy_version_id <> OLD.strategy_version_id
              OR NEW.created_at <> OLD.created_at
    THEN RAISE(ABORT, 'coaching_sessions: a recorded session''s identity is immutable')
  END;
  SELECT CASE WHEN NEW.request_fingerprint IS NOT OLD.request_fingerprint
    THEN RAISE(ABORT, 'coaching_sessions: a session''s request fingerprint is immutable')
  END;
  SELECT CASE WHEN OLD.outcome <> 'pending' AND NEW.outcome <> OLD.outcome
    THEN RAISE(ABORT, 'coaching_sessions: a settled outcome is terminal')
  END;
  SELECT CASE WHEN OLD.outcome = 'pending' AND NEW.outcome NOT IN ('proposed', 'failed')
    THEN RAISE(ABORT, 'coaching_sessions: a claim settles once, to proposed or failed')
  END;
  -- A turn that produced a proposal did not fail. Without this the disposition
  -- rail could be handed a proposal whose own session says the turn never happened.
  SELECT CASE WHEN NEW.outcome = 'failed'
                AND EXISTS (SELECT 1 FROM coaching_proposals WHERE session_id = OLD.id)
    THEN RAISE(ABORT, 'coaching_sessions: a session carrying a proposal cannot be recorded as failed')
  END;
END;

-- A proposal belongs to a turn that PRODUCED one. Attaching it to a pending claim
-- would assert an outcome the session does not have; attaching it to a failed turn
-- would contradict the one the session does have.
CREATE TRIGGER coaching_proposals_session_must_be_proposed BEFORE INSERT ON coaching_proposals
BEGIN
  SELECT CASE
    WHEN (SELECT outcome FROM coaching_sessions WHERE id = NEW.session_id) IS NOT 'proposed'
    THEN RAISE(ABORT, 'coaching_proposals: a proposal may be attached only to a proposed session')
  END;
END;

-- The accepted lineage, on insert and on update.
--
-- The FKs can say "some version exists" and "some run exists"; they cannot say
-- that THIS run is the re-backtest of THIS child, or that THIS child descends from
-- the version the session coached. `r1.s4` reads that lineage AS the version tree,
-- so a false edge is not recoverable from the row afterwards.
CREATE TRIGGER coaching_proposals_accept_lineage_insert BEFORE INSERT ON coaching_proposals
WHEN NEW.disposition = 'accepted'
BEGIN
  SELECT CASE
    WHEN (SELECT strategy_version_id FROM backtest_run WHERE id = NEW.accepted_run_id)
         IS NOT NEW.child_version_id
    THEN RAISE(ABORT, 'coaching_proposals: the accepted run is not a run of the accepted child version')
  END;
  SELECT CASE
    WHEN (SELECT parent_version_id FROM strategy_version WHERE id = NEW.child_version_id)
         IS NOT (SELECT strategy_version_id FROM coaching_sessions WHERE id = NEW.session_id)
    THEN RAISE(ABORT, 'coaching_proposals: the accepted child is not a child of the coached version')
  END;
  SELECT CASE
    WHEN (SELECT strategy_id FROM strategy_version WHERE id = NEW.child_version_id)
         IS NOT (SELECT p.strategy_id FROM strategy_version p
                 JOIN coaching_sessions s ON s.strategy_version_id = p.id
                 WHERE s.id = NEW.session_id)
    THEN RAISE(ABORT, 'coaching_proposals: the accepted child belongs to another strategy')
  END;
END;

CREATE TRIGGER coaching_proposals_accept_lineage_update BEFORE UPDATE ON coaching_proposals
WHEN NEW.disposition = 'accepted'
BEGIN
  SELECT CASE
    WHEN (SELECT strategy_version_id FROM backtest_run WHERE id = NEW.accepted_run_id)
         IS NOT NEW.child_version_id
    THEN RAISE(ABORT, 'coaching_proposals: the accepted run is not a run of the accepted child version')
  END;
  SELECT CASE
    WHEN (SELECT parent_version_id FROM strategy_version WHERE id = NEW.child_version_id)
         IS NOT (SELECT strategy_version_id FROM coaching_sessions WHERE id = NEW.session_id)
    THEN RAISE(ABORT, 'coaching_proposals: the accepted child is not a child of the coached version')
  END;
  SELECT CASE
    WHEN (SELECT strategy_id FROM strategy_version WHERE id = NEW.child_version_id)
         IS NOT (SELECT p.strategy_id FROM strategy_version p
                 JOIN coaching_sessions s ON s.strategy_version_id = p.id
                 WHERE s.id = NEW.session_id)
    THEN RAISE(ABORT, 'coaching_proposals: the accepted child belongs to another strategy')
  END;
END;

-- The disposition transition matrix, mirroring the domain's `Proposal::transition`:
--
--     proposed -> modified | rejected | accepted
--     modified -> modified | rejected | accepted
--     accepted, rejected    (terminal)
--
-- Column-presence checks alone would admit a BACKWARD transition — an accepted row
-- rewritten to `modified` with both links cleared satisfies every CHECK above and
-- is exactly the un-settling the session-id accept key exists to prevent. Rewriting
-- a terminal row with the IDENTICAL disposition and links is the idempotent no-op a
-- retrying client lands on; anything else is refused. An update that leaves the
-- disposition alone is not a transition at all — that is how a failed accept
-- records itself on a still-open proposal.
CREATE TRIGGER coaching_proposals_transition BEFORE UPDATE ON coaching_proposals
BEGIN
  SELECT CASE WHEN NEW.session_id <> OLD.session_id
    THEN RAISE(ABORT, 'coaching_proposals: a proposal cannot change session')
  END;
  SELECT CASE
    WHEN OLD.disposition IN ('accepted', 'rejected')
     AND NOT (NEW.disposition = OLD.disposition
              AND NEW.child_version_id IS OLD.child_version_id
              AND NEW.accepted_run_id IS OLD.accepted_run_id)
    THEN RAISE(ABORT, 'coaching_proposals: accepted and rejected are terminal')
  END;
  SELECT CASE
    WHEN OLD.disposition IN ('proposed', 'modified')
     AND NEW.disposition <> OLD.disposition
     AND NEW.disposition NOT IN ('modified', 'rejected', 'accepted')
    THEN RAISE(ABORT, 'coaching_proposals: nothing returns to `proposed`')
  END;
END;
