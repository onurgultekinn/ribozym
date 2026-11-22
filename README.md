# ribozym

**An autonomous multi-agent R&D ecosystem that turns a one-line idea into working software.**

A spec goes in, reviewed and tested software comes out. Seven specialised agents
collaborate under hard gates; nothing is delivered unless it traces back to the
spec and passes verification.

```
idea → A1 Architect (spec + ADRs)      → [gate: your approval]
     → A2 CEO (task DAG)               → [gate]
     → A5 Lead Coder  →  A7 Security  →  A4 Test Architect
     → A3 Reporter (numbers from code) → A1 Reviewer (spec_match must be 100%)
     → [delivery gate]                 → Delivered
```

## Why

LLM agents are good at generating code and bad at *finishing* things. ribozym's
answer is not a bigger model — it is **discipline encoded in the orchestrator**:

| Principle | Implementation |
|---|---|
| **No silent success** | A missing check is a violation, never a green light |
| **Money safety** | Pre-clamp budget: a task whose estimate would cross the hard wall is never started |
| **Traceability** | Every requirement must be cited by ≥1 task; tasks cannot cite unknown requirements |
| **Crash safety** | Append-only event log with monotonic `seq`; run lock with heartbeat; resume replays |
| **Deterministic core** | Clock, SHA-256, FSM and budget are pure functions with golden tests |

## Status

| Metric | Value |
|---|---|
| Rust | ~20k lines, 57 files |
| Tests | **204 passing, 0 failing** |
| Clippy (`-D warnings`) | 0 |
| Skeleton validator | 52/52 |
| Phases complete | 0 – 6a |

Implemented: schemas, SQLite state (WAL + migrations), event log, run lock,
FSM, budget clamps, tool gateway (exact-argv allowlist + env scrubbing),
schema repair loop, local/cloud/mock inference routing with VRAM awareness,
agents A1–A7, spec-match gate, delivery pipeline.

Not yet: interactive CLI interview mode, real VRAM probe (nvml),
per-provider `model_usage` accounting, web UI.

## Install

```bash
git clone <repo> && cd ribozym
cargo build --release            # single binary: target/release/ribozym
export OPENROUTER_API_KEY=...    # cloud primary engine
```

Optional local GPU engine (llama-server):

```bash
export LLAMA_SERVER_URL=http://127.0.0.1:8080
# config/models.toml [local].eligible_task_types decides which grunt work stays local
```

## Usage

```bash
ribozym run --tasks tasks.json --spec spec.json   # create or resume
ribozym status                                    # derived, never self-reported
ribozym approve <spec_id>                         # pass a gate
ribozym config validate                           # config + schema checks
```

Exit codes are meaningful: `0` delivered · `1` user error · `2` failed ·
`3` paused (budget) · `4` waiting (gate).

## Architecture

```
src/
├─ orchestrator.rs   phase driver, gates, budget clamps
├─ fsm.rs            run state machine (no unanchored transitions)
├─ events.rs         append-only log, seq monotonic, crash-tolerant replay
├─ lock.rs           run lock (pid + heartbeat, stale reclaim)
├─ gateway.rs        ToolGateway: exact-argv allowlist, env scrub, timeout, output cap
├─ repair.rs         SchemaGuard: one repair round, byte-stable prefix for caching
├─ select.rs         grunt-work routing (local vs cloud, VRAM aware)
├─ inference/        providers (cloud, local, mock) + EnvLayer budget/backoff
├─ agents/           A1–A7 implementations
├─ validate.rs       structural + full JSON-schema + cross-document rules
├─ hash.rs           dependency-free SHA-256 + canonical JSON
├─ clock.rs          dependency-free UTC ISO-8601
└─ state.rs          SQLite (WAL, migrations, test parity)
```

The full design is [`docs/SPEC.md`](docs/SPEC.md) (1180 lines, v2.47).
Turkish documentation: [`README.tr.md`](README.tr.md).

## Design notes

- **No decorative code.** A dedicated audit removed every constant that faked a
  result; unpopulated tables are explicitly marked as future-phase work.
- **Dependency minimalism.** `tracing-subscriber` was unavailable offline, so a
  minimal subscriber was written; SHA-256 and the UTC clock are dependency-free
  and cross-checked against Python.
- **Numbers come from code.** The reporter agent derives figures from the run
  ledger; a self-reported `total` that contradicts the task count is rejected.

## License

MIT — see [LICENSE](LICENSE).
