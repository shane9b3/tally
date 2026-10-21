#!/usr/bin/env sh
# Verifies that the production web bundle (`pnpm --filter tally-desktop
# build:web`) ships no dev mock engine and no e2e hook (security review 3,
# MEDIUM-7). Run through `pnpm --filter tally-desktop check:bundle`.
set -eu
here="$(cd "$(dirname "$0")/.." && pwd)"
dir="$here/dist/assets"
if [ ! -d "$dir" ]; then
  echo "check-bundle: $dir is missing; run build:web first" >&2
  exit 2
fi
status=0
for needle in "webMock" "mockV3" "__tally" "MockClient" \
  "That column is not in the AutoFilter." \
  "a conditional format needs a range and a rule" \
  "block too large for MockClient"; do
  if grep -l -F -- "$needle" "$dir"/*.js >/dev/null 2>&1; then
    echo "check-bundle: '$needle' found in $(grep -l -F -- "$needle" "$dir"/*.js | tr '\n' ' ')" >&2
    status=1
  fi
done
if [ "$status" -eq 0 ]; then
  echo "check-bundle: ok ($(ls "$dir"/*.js | wc -l | tr -d ' ') scripts, no dev mock strings)"
fi
exit "$status"
