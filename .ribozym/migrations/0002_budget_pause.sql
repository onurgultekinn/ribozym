-- 0002_budget_pause.sql — Faz 4 accounting columns (§8.2, v2.47 B3/B4).
-- Additive only: 0001 already shipped runs.budget_usd / runs.spent_usd.
-- paused_from records the phase to resume INTO when a Paused(budget) run's
-- budget is raised (§6.1 "kullanıcı bütçe yükseltip resume eder"); without a
-- durable anchor the resume would have to re-derive it from the ledger.
-- budget_warned latches the ONE-TIME soft-limit BudgetWarning event (§8.2) so
-- repeated polls do not spam the log.

ALTER TABLE runs ADD COLUMN paused_from TEXT;        -- Phase.as_str() or NULL
ALTER TABLE runs ADD COLUMN paused_reason TEXT;      -- budget | checkpoint, or NULL
ALTER TABLE runs ADD COLUMN budget_warned INTEGER NOT NULL DEFAULT 0;
