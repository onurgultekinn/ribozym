-- 0001_init.sql — ribozym state.db initial schema (§7.2)
-- Single global store at .ribozym/state.db; single writer = control plane.
-- All timestamps UTC ISO-8601 with Z suffix.
--
-- POPULATION STATUS (honesty note — the schema is §7.2 complete, the CODE is
-- phased). Written today (Faz 4): runs, tasks, ledger, approvals. Created but
-- intentionally EMPTY until the phase that owns them fills them:
--   model_usage     -> Faz 5  (EnvLayer actual_cost per call)
--   task_scores     -> Faz 8  (A10 benchmark tier list)
--   prices          -> Faz 8  (OpenRouter price poll, §5.15 m3)
--   web_cache       -> Faz 7  (A6/A11 research, §13.2)
--   archive_sections-> Faz 6  (A12 archive manifest, §7.6 B2)
-- No read path pretends these are populated; they are forward schema, not dead.

PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;

CREATE TABLE runs(
  spec_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  status TEXT NOT NULL,               -- FSM state; includes Paused(budget) (v2.47 B3)
  autonomy_level INTEGER,
  budget_usd REAL,
  spent_usd REAL DEFAULT 0,
  crisis_count INTEGER DEFAULT 0,     -- FinalReview->Crisis loop counter, cap 2 (v2.47 B1)
  created_at TEXT,
  updated_at TEXT
);

CREATE TABLE tasks(
  spec_id TEXT,
  task_id TEXT,
  status TEXT NOT NULL,               -- pending|in_progress|done|failed|skipped|blocked
  agent TEXT,
  retry_count INTEGER DEFAULT 0,
  escalation_level INTEGER DEFAULT 0, -- 0..4 (§8.1)
  PRIMARY KEY(spec_id, task_id)
);

CREATE TABLE ledger(                  -- operation ledger (§7.4)
  op_id TEXT PRIMARY KEY,
  kind TEXT,                          -- agent_call | shell_exec | git_op | web_fetch
  spec_id TEXT,
  task_id TEXT,
  payload_hash TEXT,
  state TEXT,                         -- started | finished | failed
  result_ref TEXT,
  started_at TEXT,
  finished_at TEXT
);
CREATE INDEX idx_ledger_open ON ledger(state, spec_id) WHERE state = 'started';

CREATE TABLE model_usage(
  id INTEGER PRIMARY KEY,
  spec_id TEXT,
  task_id TEXT,
  agent TEXT,
  provider TEXT,                      -- openrouter | local
  model TEXT,
  tokens_in INTEGER,
  tokens_out INTEGER,
  cached INTEGER,
  cost_usd REAL,
  ts TEXT
);
CREATE INDEX idx_usage_spec ON model_usage(spec_id, ts);

CREATE TABLE task_scores(             -- A10 tier list + live scores (v2.41, §10.3)
  model TEXT,
  task_type TEXT,
  bench_score REAL,
  success_rate REAL,
  tokens_per_success REAL,
  avg_cost REAL,
  tier_rank INTEGER,
  last_used TEXT,
  PRIMARY KEY(model, task_type)
);

CREATE TABLE approvals(
  spec_id TEXT,
  gate TEXT,                          -- spec | task_plan | delivery
  decided_by TEXT,
  decision TEXT,                      -- approved | denied | timeout_auto
  note TEXT,
  ts TEXT,
  PRIMARY KEY(spec_id, gate)
);

CREATE TABLE web_cache(
  key TEXT PRIMARY KEY,               -- sha256(normalized query + provider)
  finding_json TEXT,
  fetched_at TEXT,
  ttl_s INTEGER,
  verified_by TEXT                    -- non-null = verified within TTL; not re-scored
);

CREATE TABLE prices(                  -- background OpenRouter price poll (v2.43, 5 min)
  model TEXT PRIMARY KEY,
  in_price_per_m REAL,
  out_price_per_m REAL,
  fetched_at TEXT,
  stale INTEGER DEFAULT 0             -- 1 when PRICE_STALE fallback applied
);

CREATE TABLE archive_sections(        -- mirror of archive manifest (v2.47 B2)
  spec_id TEXT,
  file TEXT,
  sha256 TEXT,
  size_bytes INTEGER,
  written_at TEXT,
  kind TEXT,
  PRIMARY KEY(spec_id, file)
);
