#!/usr/bin/env bash
set -euo pipefail

base_ref="${GITHUB_BASE_REF:-main}"
git diff --check "origin/$base_ref...HEAD"

prompt="Review the changes in origin/$base_ref...HEAD. Do not edit files. Look for correctness, regressions, security issues, missing tests, and CI bypasses. If any blocking issue exists, end with exactly KILO_GATE: FAIL. Otherwise end with exactly KILO_GATE: PASS. Include concise evidence before the final line."

kilo run --auto --agent orchestrator "$prompt" | tee .ai-pipeline/kilo-review.txt
grep -qx 'KILO_GATE: PASS' .ai-pipeline/kilo-review.txt

