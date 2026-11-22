#!/usr/bin/env bash
# The four standing gates of this repo (PROGRESS.md header). CI = this script.
set -euo pipefail
cd "$(dirname "$0")/.."
echo "== fmt =="       && cargo fmt --check
echo "== clippy =="    && cargo clippy --all-targets -- -D warnings
echo "== tests =="     && cargo test
echo "== skeleton =="  && python scripts/validate_skeleton.py
echo "ALL GATES GREEN"
