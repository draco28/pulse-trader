-- r1.s2.w2 — 0005: the coaching session + proposal tables (ADR-0021 decision 5,
-- grill L2, audit C3/C4).
--
-- Migration number `0005` was RESERVED for this spine at release planning:
-- `r1.s1` already shipped `0007` and `r1.s3` holds `0006`, so this migration lands
-- BELOW the database's current maximum. That is supported on purpose —
-- `src/adapters/db/migrate.rs` compares applied version SETS rather than maxima
-- precisely so filling a reserved gap cannot be mistaken for "already current"
-- (PR #115). `tests/migration_0005.rs` proves it against a real database at 0007.
--
-- THE FULL SCHEMA LANDS NOW, INCLUDING THE COLUMNS NOTHING READS YET (grill L2).
-- `disposition`, `child_version_id` and the accept idempotency guarantee are
-- dormant until `r1.s4`'s accept/reject/modify rail, which then exercises them
-- WITHOUT a second migration. `w2` writes only the `proposed` state. The dormant
-- columns are schema stability for a consumer committed in this same release (the
-- `SweepableValue::Sweep` precedent), not a shell.
--
-- NO `validated` COLUMN EXISTS HERE, and adding one later would be a mistake
-- rather than an improvement (audit C4 / ADR-0021 decision 3): a mutation's
-- validity is established by `apply()` at the moment of use and is never a stored
-- property, because the inputs it depends on — the DSL schema, the version tree,
-- the validation rules — can all change between proposal and accept. `r1.s4`
-- re-runs `apply()` at accept.
--
-- Conventions inherited from `0003_backtest_runs` / `0004_llm_call` (the single
-- source of truth for these shapes): `TEXT` primary keys; RFC3339 UTC `created_at`
-- from the injected `Clock`; a `schema_version` row tag the adapter ASSERTS on
-- read (the #68 read-reject); JSON payload columns for typed domain values. There
-- is no `f64`-typed column anywhere in this migration (NFR-2).
--
-- NO immutability triggers, unlike `llm_call` and `backtest_run`: a proposal's
-- `disposition` is UPDATEd by `r1.s4`'s rail, so this pair is deliberately not
-- append-only. The audit guarantee here is that a session row EXISTS for every
-- turn (below), not that it can never be touched.

-- The audit trail (audit C3). One row per coach turn, success or failure — the
-- never-silence guarantee is that this row exists either way, so a failed turn is
-- a record rather than an absence.
--
-- `llm_call_id` is NULLABLE and is NULL PRECISELY WHEN NO PROVIDER CALL WAS MADE:
-- a pre-call failure (an oversized context) records the session with no ledger
-- row, while every turn that reached the provider names its `llm_call`. The
-- precedent is `strategy_version.creating_llm_call_ids`.
CREATE TABLE coaching_sessions (
  id                   TEXT PRIMARY KEY NOT NULL,
  backtest_run_id      TEXT NOT NULL REFERENCES backtest_run(id),      -- the run the coach READ (and never recomputed)
  strategy_version_id  TEXT NOT NULL REFERENCES strategy_version(id),  -- whose DSL the proposal mutates
  created_at           TEXT NOT NULL,                                  -- injected Clock (RFC3339 UTC)
  llm_call_id          TEXT REFERENCES llm_call(id),                   -- NULL iff no provider call was made (audit C3)
  outcome              TEXT NOT NULL,                                  -- 'proposed' | 'failed'
  failure_kind         TEXT,                                           -- the CoachFailure tag; NULL iff outcome='proposed'
  failure_detail       TEXT,                                           -- serde JSON of the whole typed CoachFailure
  schema_version       INTEGER NOT NULL,                               -- row-schema tag, asserted on read (#68)

  -- The outcome vocabulary, enumerated in-schema so a typo cannot become a state.
  CHECK (outcome IN ('proposed', 'failed')),

  -- Never silence, at the SQL layer: a failed turn carries BOTH its kind and its
  -- detail, and a proposed turn carries NEITHER. `(a) = (b)` compares SQLite's 0/1
  -- booleans, so this is an iff, not an implication.
  CHECK ((outcome = 'failed') = (failure_kind IS NOT NULL)),
  CHECK ((outcome = 'failed') = (failure_detail IS NOT NULL)),

  -- The L3 failure taxonomy, enumerated. A new variant is a schema change on
  -- purpose: the taxonomy is the contract `w3` records against.
  CHECK (failure_kind IS NULL OR failure_kind IN (
    'zero_calls',
    'several_calls',
    'malformed_arguments',
    'inapplicable_mutation',
    'provider_timeout',
    'context_overflow',
    -- r1.s2.w4 (operator ruling 2026-08-29): a provider transport fault is a
    -- recorded outcome too. Added by editing 0005 IN PLACE rather than shipping a
    -- second migration, because 0005 has never left this spine branch — ADR-0018's
    -- forward-only rule binds migrations that have been applied somewhere, and this
    -- one has not.
    'transport_failure'
  ))
);

-- AT MOST ONE PROPOSAL PER SESSION, and the session id IS the accept idempotency
-- key (`r1.s4`'s consistency model keys one child version per proposal by session
-- id). `session_id UNIQUE` is that guarantee in the schema rather than in a
-- comment: "exactly one mutation per turn" cannot be violated by a retry.
CREATE TABLE coaching_proposals (
  id                TEXT PRIMARY KEY NOT NULL,
  session_id        TEXT NOT NULL UNIQUE REFERENCES coaching_sessions(id),
  mutation          TEXT NOT NULL,                                     -- serde JSON of the typed w1 `Mutation`
  hypothesis        TEXT NOT NULL,                                     -- the stated reason; never empty
  disposition       TEXT NOT NULL,                                     -- 'proposed' | 'accepted' | 'rejected' | 'modified'
  child_version_id  TEXT REFERENCES strategy_version(id),              -- non-NULL iff disposition='accepted' (dormant until r1.s4)

  -- The disposition vocabulary, enumerated in-schema (grill L2).
  CHECK (disposition IN ('proposed', 'accepted', 'rejected', 'modified')),

  -- A stated hypothesis is part of the capability sentence; an empty string is
  -- silence wearing a proposal's clothes. `Hypothesis` enforces this in the domain
  -- and the column enforces it against anything that writes around the domain.
  --
  -- The trim CHAR-SET is explicit because SQLite's one-argument `trim()` strips
  -- SPACES ONLY. A hypothesis of a single tab or newline would pass `trim(x)` and
  -- then be refused by `Hypothesis::new` (Rust's `str::trim` is whitespace-wide) at
  -- READ time — so the row would insert and `list_sessions_for_run` would
  -- fail-close on it forever after.
  --
  -- The set is therefore every scalar Rust's `char::is_whitespace` accepts (the
  -- Unicode `White_Space` property), NOT the ASCII subset: ASCII HT/LF/VT/FF/CR/SP,
  -- NEL, NBSP, OGHAM SPACE MARK, EN QUAD..HAIR SPACE, LINE/PARAGRAPH SEPARATOR,
  -- NARROW NBSP, MEDIUM MATHEMATICAL SPACE, IDEOGRAPHIC SPACE. Parity with the
  -- domain rule is what matters, so it is test-enforced rather than trusted:
  -- `migration_0005::the_hypothesis_check_rejects_every_scalar_rust_calls_whitespace`
  -- derives the set from the toolchain and asserts each one is refused. If a future
  -- Rust picks up a Unicode revision that adds a scalar, that test goes red and this
  -- list is what gets updated. Non-whitespace Unicode stays acceptable — U+200B
  -- ZERO WIDTH SPACE is not `White_Space`, and is deliberately storable.
  CHECK (
    length(trim(hypothesis, char(9, 10, 11, 12, 13, 32, 133, 160, 5760,
                                 8192, 8193, 8194, 8195, 8196, 8197, 8198,
                                 8199, 8200, 8201, 8202, 8232, 8233, 8239,
                                 8287, 12288))) > 0
  ),

  -- `r1.s4`'s consistency model: no accepted proposal without its child version,
  -- and no child version on a proposal that was not accepted.
  CHECK ((disposition = 'accepted') = (child_version_id IS NOT NULL))
);

-- The coach's read pattern: sessions for one run, most recent last. `(run, created_at)`
-- makes that scan index-served.
CREATE INDEX idx_coaching_sessions_run ON coaching_sessions(backtest_run_id, created_at);

-- The coach-prompt version on the ledger row (grill L2 / audit C2). NULLABLE, and
-- `llm_call.schema_version` deliberately STAYS at 1 — the same reasoning `0007`
-- recorded for `key_source`: `llm_call_repo::get_call` fail-closes on any stored
-- `schema_version` that is not the current constant, so bumping the tag would
-- strand every row written before this migration. An ADDED nullable column is
-- backward-compatible by construction: a pre-0005 row reads back `None`, meaning
-- "no prompt version recorded", which is true rather than an error.
--
-- The VALUE is the content hash of the RESOLVED prompt — whichever of the
-- compiled-in default or the `$PULSE_PROMPT_DIR` overlay actually won, hashed per
-- call (audit C2). Computing it is `w3`'s; this item ships the column and the
-- field. Composer rows stay NULL.
ALTER TABLE llm_call ADD COLUMN prompt_version TEXT;
