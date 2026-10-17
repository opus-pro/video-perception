#!/usr/bin/env bash
# Test: schema validation tooling and JSON Schema export
# Requires: npx, tsx

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PASS=0
FAIL=0

pass() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# Check prerequisites
if ! command -v npx &> /dev/null; then
  echo "SKIP: npx not installed"
  exit 0
fi

echo "=== Schema Tests ==="

# --- Test 1: export-json-schema.ts produces JSON Schema files ---
echo "Test 1: export-json-schema.ts generates schema files"
if npx tsx "$REPO_ROOT/schema/export-json-schema.ts" > /dev/null 2>&1; then
  pass "export-json-schema.ts runs successfully"
else
  fail "export-json-schema.ts failed"
fi

# --- Test 2: Required JSON Schema files exist ---
echo "Test 2: Required JSON Schema files exist"
SCHEMA_DIR="$REPO_ROOT/schema/json"
for f in signal_deck.schema.json; do
  if [ -f "$SCHEMA_DIR/$f" ]; then
    pass "$f exists"
  else
    fail "$f missing in schema/json/"
  fi
done

# --- Test 3: JSON Schema files are valid JSON ---
echo "Test 3: JSON Schema files are valid JSON"
SCHEMA_VALID=true
for f in "$SCHEMA_DIR"/*.schema.json; do
  if [ -f "$f" ]; then
    if ! python3 -m json.tool "$f" > /dev/null 2>&1; then
      fail "$(basename "$f") is not valid JSON"
      SCHEMA_VALID=false
    fi
  fi
done
if [ "$SCHEMA_VALID" = true ]; then
  pass "all .schema.json files are valid JSON"
fi

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
