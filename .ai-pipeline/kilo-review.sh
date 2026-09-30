#!/usr/bin/env bash
set -euo pipefail

base_ref="${GITHUB_BASE_REF:-main}"
# Text model for the review; override with the KILO_MODEL repository variable.
# Left unset, the CLI picked the account default, which drifted to an image model.
model="${KILO_MODEL:-kilo/google/gemini-3.1-pro-preview}"
work_dir="${RUNNER_TEMP:-$(mktemp -d)}/kilo-review"
evidence=".ai-pipeline/kilo-review.txt"
mkdir -p "$work_dir"

git diff --check "origin/$base_ref...HEAD"
git diff --stat "origin/$base_ref...HEAD" >"$work_dir/pr.stat"
git diff "origin/$base_ref...HEAD" >"$work_dir/pr.diff"

# Read-only reviewer: the ask agent cannot edit or write, and shell and web
# access are denied on top. The CI job already built and tested this commit
# (kilo-review needs ci); a reviewer that re-ran the build blew the job timeout.
export KILO_PERMISSION='{"bash":"deny","webfetch":"deny","websearch":"deny"}'

prompt="Review this pull request: the diff of origin/$base_ref...HEAD is attached (pr.diff, with pr.stat as the file summary). You may read files in the repository for context. You cannot run commands; the CI job already built and tested this commit and passed. Look for correctness, regressions, security issues, missing tests, and CI bypasses. Include concise evidence. The very last line of your answer must be exactly KILO_GATE: PASS, or exactly KILO_GATE: FAIL if any blocking issue exists."

status=0
timeout 14m kilo run "$prompt" -f "$work_dir/pr.stat" -f "$work_dir/pr.diff" \
  --auto --agent ask --model "$model" >"$work_dir/review.txt" || status=$?

# Written only after the agent has exited, so nothing it does in the checkout can
# remove the evidence the upload step needs.
sed 's/\x1b\[[0-9;]*[A-Za-z]//g' "$work_dir/review.txt" >"$evidence"
cat "$evidence"

if [ "$status" -ne 0 ]; then
  echo "kilo run failed with exit code $status (124 = timed out)" >&2
  exit "$status"
fi

last_line=$(grep -v '^[[:space:]]*$' "$evidence" | tail -n 1 | tr -d '\r' | sed 's/[[:space:]]*$//')
if [ "$last_line" != "KILO_GATE: PASS" ]; then
  echo "Kilo gate did not pass (last line: '$last_line')" >&2
  exit 1
fi
