# Spec: Minimal REST API for a task list

<!-- Golden-run input (§21.3). Deterministic, small, fully testable with a Rust
     toolchain only — no external services. MockLLM replays recorded responses
     for this input; the expected outputs live in examples/expected/. -->

## Summary
A small HTTP service that stores task items and exposes list/create/complete
operations over a REST API.

## Goal & Users
- goal: demonstrate a complete ribozym run end to end on a bounded project
- target_users: developer running the golden test
- platform_region: local machine, no external network at runtime

## Scope
- must_have:
  - REST endpoint returning the list of tasks
  - endpoint to create a task with a title
  - endpoint to mark a task complete
- out_of_scope:
  - authentication
  - persistence beyond process lifetime may use SQLite file, but migrations/backup not required
  - websocket/SSE

## Constraints
- language: Rust (explicit)
- framework: axum
- only crates from crates.io, no private registries
- service must respond on 127.0.0.1:8080

## Budget
- run_budget_usd: 1.50
- hosting_budget_note: none, runs locally

## Delivery Shape
- cargo project, runnable with `cargo run`; Dockerfile optional

## Success Criteria
- cargo test passes with an integration test covering all three endpoints
- curl POST /tasks then GET /tasks returns the created task
