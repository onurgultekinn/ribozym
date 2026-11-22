# Changelog

All notable changes to ribozym. Phases track `agent-sistem-planlama-v2.md` (v2.47);
`PROGRESS.md` is the working ledger, this file the released summary.

## [0.1.0] — 2026-09-13 — first publishable build

### Added
- Faz 0–4: schemas (11 contracts), config registry (A1–A12), SQLite state.db
  (WAL, migrations 0001–0003), append-only fsynced event log, §7.4 resume
  ledger (exactly-once ops + charges), DAG scheduler with single-writer file
  ownership, §9 approval gates (durable requests, Q4 timeout policy), run FSM,
  golden kill→resume hashes stable across every B4 write window.
- Faz 5: agent line — A5 LeadCoder (schema-forced turns, fix-loop ≤3, isolated
  workdir), A7 security (secret scan of the WHOLE tree, gateway-rescope,
  capability-probed cargo audit), A4 Test Architect (tests/ ownership, AC-cite
  fairness rule), AgentBridge (sync control plane ↔ async agents, per-task
  runtime), honest per-turn cost accounting.
- Faz 5-c/5-d: management line — A1 (spec draft with code-computed provenance
  hash; referee with code-recomputed spec_match_pct, fabricated rows dropped,
  unaudited assumptions counted falsified), A2 (plan generation + in-scope E2
  replan: completed tasks immutable, requirement set pinned), A3 (report
  numbers strictly from the system of record; LLM merges narrative only;
  sealed compliance block), `run_op`/`plan_turn` ledger-guarded management
  turns, §6.1 full lifecycle `idle→…→scheduling` with the REAL spec gate,
  structural form check (`check_form`) parking incomplete forms before any
  model call, crisis→Replanned→re-scheduling loop (capped), read-only CLI
  surfaces (`status`, `report`, `budget show`, `new-form`), `ribozym run
  --mode interactive`.
- Faz 6 (partial): §13 web door — pluggable `WebFetcher` (command backend,
  argv-only ask, SSRF guard), sha256 snippet-only findings (P7: content is
  data), rules-layer blind verification (`rules_only: true`, injection
  demotion), TTL `web_cache` with verified-reuse (no re-score inside TTL),
  A11 as a real DAG task through the bridge.
- Safety: EnvLayer on the production call path (pre-clamp + reconcile + E0
  full-jitter backoff), budget wall pauses instead of failing tasks (B3),
  `charged_at` exactly-once spend guard (0003), sticky soft-limit latch,
  ToolGateway full-argv allowlist + env scrub + timeout/cap, RunLock
  single-copy discipline, local `logging.rs` WARN+ sink.

### Fixed (real bugs found by adversarial self-review; each locked by a test)
- double-charge of `runs.spent_usd` when killed in the §7.4 commit→close
  window (invisible while every test executor billed 0.0);
- soft-limit warning latch erased by the next non-crossing commit (could
  re-warn after a restart);
- wall-clock event `ts` leaking into report durations → ledger payload hashes
  → two identical runs diverging on final digest (durations now read from the
  injected-clock ledger stamps);
- planning-phase scheduler/by_id left empty after the spec gate (runs would
  "fail" with no schedulable tasks despite persisted rows);
- byte-arithmetic markdown section slicing (UTF-8 panic risk) in the form
  check; cache hits returning stale `query_id` envelopes.

### Deviations (honest, tracked)
- archive sections stored uncompressed (no zstd in the offline dependency
  set); manifest hash protocol identical — a storage-format change later.
- A11 verification is the rule layer only per the Q2/MVP decision; LLM blind
  scoring is v1.0.
- model selection is "first non-blacklisted registry entry" until A10
  calibration (Faz 8) lands.
