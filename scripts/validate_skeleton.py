#!/usr/bin/env python3
"""ribozym pre-code skeleton validator (Phase 0 gate).

Common tests for the coding-ready skeleton:
  1. Every schema in schemas/ is a valid JSON Schema draft 2020-12 document.
  2. Positive fixtures (examples/expected/*_valid.*) validate against their schema.
  3. Negative fixtures (examples/expected/negative/*) are REJECTED by their schema.
  4. events_valid.jsonl lines validate; seq strictly increases.
  5. All TOML config files parse.
  6. Cross-file consistency:
     - agents.toml IDs are exactly A1..A12 (contiguous, v2.46);
     - every agents.toml prompt_file exists and its first line carries model_tier;
     - budget_share_pct sums to 100;
     - every agent escalation_tier is a valid tier;
     - ribozym.toml referenced enums are consistent with the doc rules.
  7. SQLite migration 0001_init.sql executes cleanly; §7.2 tables exist;
     PK constraint on tasks rejects duplicates.
Exit code 0 = skeleton is coding-ready; nonzero = fix before Phase 0.
"""

import json
import re
import sqlite3
import sys
import tempfile
import tomllib
from pathlib import Path

try:
    from jsonschema import Draft202012Validator
except ImportError:
    print("FATAL: jsonschema not installed (pip install jsonschema)")
    sys.exit(2)

ROOT = Path(__file__).resolve().parents[1]
SCHEMAS = ROOT / "schemas"
EXPECTED = ROOT / "examples" / "expected"
NEGATIVE = EXPECTED / "negative"

failures: list[str] = []
passed = 0


def check(name: str, cond: bool, detail: str = "") -> None:
    global passed
    if cond:
        passed += 1
        print(f"  ok   {name}")
    else:
        failures.append(f"{name}: {detail}")
        print(f"  FAIL {name} — {detail}")


def schema_map() -> dict[str, dict]:
    m = {}
    for p in sorted(SCHEMAS.glob("*.schema.json")):
        m[p.name] = json.loads(p.read_text(encoding="utf-8"))
    return m


def main() -> int:
    schemas = schema_map()

    print("[1] schema documents are valid draft 2020-12")
    for name, sch in schemas.items():
        try:
            Draft202012Validator.check_schema(sch)
            check(f"check_schema {name}", True)
        except Exception as exc:  # noqa: BLE001
            check(f"check_schema {name}", False, str(exc)[:120])

    v = {n: Draft202012Validator(s) for n, s in schemas.items()}

    print("[2] positive fixtures validate")
    positives = [
        ("spec_valid.json", "spec.schema.json"),
        ("tasks_valid.json", "tasks.schema.json"),
        ("execution_result_valid.json", "execution_result.schema.json"),
        ("report_valid.json", "department_report.schema.json"),
        ("coding_task_valid.json", "coding_task.schema.json"),
    ]
    for fix, sch in positives:
        errs = sorted(v[sch].iter_errors(json.loads((EXPECTED / fix).read_text(encoding="utf-8"))),
                      key=lambda e: e.json_path)
        check(f"{fix} -> {sch}", not errs,
              "; ".join(f"{e.json_path}: {e.message[:100]}" for e in errs[:3]))

    events = [json.loads(line) for line in
              (EXPECTED / "events_valid.jsonl").read_text(encoding="utf-8").splitlines() if line.strip()]
    seqs = [e["seq"] for e in events]
    for i, ev in enumerate(events):
        errs = list(v["event.schema.json"].iter_errors(ev))
        check(f"events_valid.jsonl line {i + 1} -> event.schema.json", not errs,
              "; ".join(e.message[:100] for e in errs[:2]))
    check("events seq strictly increasing", all(b > a for a, b in zip(seqs, seqs[1:])), str(seqs))

    print("[3] negative fixtures are rejected")
    negatives = [
        ("spec_decision_no_rationale.json", "spec.schema.json"),
        ("spec_bad_requirement_id.json", "spec.schema.json"),
        ("tasks_unknown_agent.json", "tasks.schema.json"),
        ("tasks_uncovered_requirement.json", "tasks.schema.json"),
        ("event_missing_seq.json", "event.schema.json"),
        ("event_unknown_name.json", "event.schema.json"),
        ("result_bad_hash.json", "execution_result.schema.json"),
        ("report_wrong_generator.json", "department_report.schema.json"),
        ("coding_task_path_traversal.json", "coding_task.schema.json"),
    ]
    for fix, sch in negatives:
        doc = json.loads((NEGATIVE / fix).read_text(encoding="utf-8"))
        errs = list(v[sch].iter_errors(doc))
        check(f"reject {fix}", bool(errs), "schema ACCEPTED an invalid document")

    print("[4] TOML configs parse")
    tomls = [ROOT / "ribozym.toml", ROOT / "config" / "agents.toml", ROOT / "config" / "models.toml"]
    parsed = {}
    for path in tomls:
        try:
            parsed[path.name] = tomllib.loads(path.read_text(encoding="utf-8"))
            check(f"parse {path.name}", True)
        except Exception as exc:  # noqa: BLE001
            check(f"parse {path.name}", False, str(exc)[:120])

    print("[5] cross-file consistency")
    agents = parsed.get("agents.toml", {}).get("agent", [])
    ids = [a["id"] for a in agents]
    check("agents.toml has exactly A1..A12", ids == [f"A{i}" for i in range(1, 13)], str(ids))
    total_share = sum(a.get("budget_share_pct", 0) for a in agents)
    check("budget_share_pct sums to 100", total_share == 100, str(total_share))
    valid_tiers = {"low", "medium", "high", "none"}
    check("all model_tier values valid", all(a.get("model_tier") in valid_tiers for a in agents),
          str([a["id"] for a in agents if a.get("model_tier") not in valid_tiers]))
    check("all escalation_tier values valid", all(a.get("escalation_tier") in valid_tiers for a in agents),
          str([a["id"] for a in agents if a.get("escalation_tier") not in valid_tiers]))

    for a in agents:
        pf = ROOT / a["prompt_file"]
        if not pf.exists():
            check(f"prompt exists {a['id']}", False, str(a["prompt_file"]))
            continue
        first = pf.read_text(encoding="utf-8").splitlines()[0]
        m = re.search(r"model_tier:\s*(\w+)", first)
        ok = bool(m) and m.group(1) == a["model_tier"]
        check(f"prompt {a['id']} first-line model_tier matches registry", ok, first[:80])

    azym = parsed.get("ribozym.toml", {}).get("errors", {})
    check("ribozym.toml final_review_crisis_max == 2 (v2.47 B1)", azym.get("final_review_crisis_max") == 2,
          str(azym.get("final_review_crisis_max")))
    check("ribozym.toml archive integrity sha256-manifest (v2.47 B2)",
          parsed.get("ribozym.toml", {}).get("archive", {}).get("integrity") == "sha256-manifest")

    print("[6] SQLite migration 0001_init.sql")
    sql = (ROOT / ".ribozym" / "migrations" / "0001_init.sql").read_text(encoding="utf-8")
    with tempfile.TemporaryDirectory() as td:
        con = sqlite3.connect(str(Path(td) / "test.db"))
        try:
            con.executescript(sql)
            check("migration executes", True)
        except Exception as exc:  # noqa: BLE001
            check("migration executes", False, str(exc)[:150])
        have = {r[0] for r in con.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        need = {"runs", "tasks", "ledger", "model_usage", "task_scores", "approvals", "web_cache", "prices", "archive_sections"}
        check("all §7.2 tables present", need <= have, str(sorted(need - have)))
        con.execute("INSERT INTO tasks(spec_id, task_id, status, agent) VALUES ('spec-20260912-001','T1','pending','A5')")
        try:
            con.execute("INSERT INTO tasks(spec_id, task_id, status, agent) VALUES ('spec-20260912-001','T1','done','A5')")
            check("tasks PK rejects duplicates", False, "duplicate insert succeeded")
        except sqlite3.IntegrityError:
            check("tasks PK rejects duplicates", True)
        con.commit()
        con.close()

    print(f"\n{passed} passed, {len(failures)} failed")
    if failures:
        print("FAILURES:")
        for f in failures:
            print(f"  - {f}")
        return 1
    print("SKELETON IS CODING-READY")
    return 0


if __name__ == "__main__":
    sys.exit(main())
