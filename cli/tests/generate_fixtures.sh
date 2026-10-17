#!/bin/bash
# Regenerate test fixtures from demo/morning_run_30s.mp4
#
# Run manually: cd cli && bash tests/generate_fixtures.sh
#
# Not run in CI — too slow. Run when sensor output changes
# to refresh the committed fixture files.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OBS=$(mktemp -d)

echo "Building video-perception..."
cd "$REPO_ROOT/cli"
cargo build --release

BIN="$REPO_ROOT/cli/target/release/video-perception"

echo "Generating Observation..."
"$BIN" observe "$REPO_ROOT/demo/morning_run_30s.mp4" \
  --capabilities stats,signalstats,layout,track \
  --out "$OBS"

# Snapshot JSON files into fixtures (no binary assets)
FIXTURES="$REPO_ROOT/cli/tests/fixtures/obs"
mkdir -p "$FIXTURES"

cp "$OBS/dashboard.json" "$FIXTURES/"
cp "$OBS/scenes.jsonl" "$FIXTURES/"
cp "$OBS/manifest.json" "$FIXTURES/"
[ -f "$OBS/transcript.jsonl" ] && cp "$OBS/transcript.jsonl" "$FIXTURES/"

echo "Fixtures written to $FIXTURES"
echo "Files:"
ls -la "$FIXTURES"

# Cleanup temp dirs
rm -rf "$OBS"
