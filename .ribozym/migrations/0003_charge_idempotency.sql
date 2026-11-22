-- 0003_charge_idempotency.sql — §8.2/§7.4: an op's cost reaches
-- runs.spent_usd EXACTLY once. The §7.4-B4 kill window between the commit
-- (step 2) and the ledger close (step 4) let the resume path double-charge:
-- the FinalizeResult branch re-books the persisted cost, but the spend was
-- written as an ABSOLUTE value derived from the in-memory mirror — a process
-- that opened AFTER the charge had committed added the cost a second time.
-- charged_at is the durable per-op guard: the spend increment runs only when
-- this NULL→stamp flip wins, inside the SAME BEGIN IMMEDIATE as the
-- increment (a rolled-back tx can never consume the guard).
ALTER TABLE ledger ADD COLUMN charged_at TEXT;
