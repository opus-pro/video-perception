#!/usr/bin/env bash
# Eval: Score agent Opinion JSON output quality against criteria.
# Usage: test_eval.sh <agent_slug> <opinion_json_file>
#
# Checks both structural quality (valid JSON, required fields, schema)
# and content quality (coverage, specificity, grounding).
# Outputs a scorecard to stdout.

set -euo pipefail

SLUG="${1:?Usage: test_eval.sh <agent_slug> <opinion_json_file>}"
OPINION_FILE="${2:?Usage: test_eval.sh <agent_slug> <opinion_json_file>}"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

SCORE=0
MAX=0
NOTES=()

score() {
  local points="$1" max="$2" desc="$3"
  SCORE=$((SCORE + points))
  MAX=$((MAX + max))
  if [ "$points" -eq "$max" ]; then
    echo "  [$points/$max] $desc"
  else
    echo "  [$points/$max] $desc"
    [ -n "${4:-}" ] && NOTES+=("  -> $4")
  fi
}

echo "=== Eval: $SLUG — $(basename "$OPINION_FILE") ==="

# ---- STRUCTURAL (40 points) ----
echo ""
echo "--- Structural Quality (40 pts) ---"

# S1: Valid JSON (10 pts)
if python3 -m json.tool "$OPINION_FILE" > /dev/null 2>&1; then
  score 10 10 "Valid JSON"
else
  score 0 10 "Invalid JSON" "File does not parse as valid JSON"
  # Cannot continue structural checks without valid JSON
  echo ""
  echo "--- Structural Score: $SCORE / $MAX ---"
  echo "Cannot continue — file is not valid JSON."
  exit 1
fi

# S2: Required top-level fields (10 pts)
MISSING_FIELDS=$(python3 -c "
import sys, json
d = json.load(open('$OPINION_FILE'))
required = ['version', 'agent', 'video', 'created_at', 'signals_used', 'summary', 'assessment', 'observations']
missing = [k for k in required if k not in d]
print(','.join(missing) if missing else '')
" 2>/dev/null || echo "check-failed")

if [ -z "$MISSING_FIELDS" ]; then
  score 10 10 "All required top-level fields present"
else
  score 0 10 "Missing fields: $MISSING_FIELDS"
fi

# S3: Assessment is a valid enum value (5 pts)
ASSESSMENT_VALID=$(python3 -c "
import sys, json
d = json.load(open('$OPINION_FILE'))
valid = ['excellent', 'good', 'acceptable', 'needs_work', 'problematic']
print('yes' if d.get('assessment') in valid else 'no')
" 2>/dev/null || echo "no")

if [ "$ASSESSMENT_VALID" = "yes" ]; then
  score 5 5 "Assessment is a valid enum value"
else
  score 0 5 "Assessment not in: excellent, good, acceptable, needs_work, problematic"
fi

# S4: Observations have required fields (10 pts)
OBS_CHECK=$(python3 << PYEOF
import sys, json
d = json.load(open('$OPINION_FILE'))
obs = d.get('observations', [])
if not obs:
    print('empty')
    sys.exit()
required_obs = ['scene_idx', 'text', 'severity', 'confidence', 'tags']
bad = []
for i, o in enumerate(obs):
    missing = [k for k in required_obs if k not in o]
    if missing:
        bad.append('obs[%d] missing: %s' % (i, ','.join(missing)))
if bad:
    print('; '.join(bad[:3]))
else:
    print('ok')
PYEOF
)

if [ "$OBS_CHECK" = "ok" ]; then
  score 10 10 "All observations have required fields"
elif [ "$OBS_CHECK" = "empty" ]; then
  score 5 10 "Observations array is empty" "Expected at least one observation"
else
  score 0 10 "Observation field issues: $OBS_CHECK"
fi

# S5: Schema validation via validate.ts (5 pts)
if command -v npx &> /dev/null && [ -f "$REPO_ROOT/schema/validate.ts" ]; then
  if npx tsx "$REPO_ROOT/schema/validate.ts" opinion "$OPINION_FILE" > /dev/null 2>&1; then
    score 5 5 "Passes schema validation (validate.ts)"
  else
    SCHEMA_ERR=$(npx tsx "$REPO_ROOT/schema/validate.ts" opinion "$OPINION_FILE" 2>&1 | head -5 || true)
    score 0 5 "Fails schema validation" "$SCHEMA_ERR"
  fi
else
  echo "  [skip] Schema validation skipped (npx/tsx not available)"
  MAX=$((MAX + 5))
fi

# ---- CONTENT QUALITY (60 points) ----
echo ""
echo "--- Content Quality (60 pts) ---"
echo "  (Requires manual or LLM review — checklist below)"
echo ""

# Print summary stats
python3 << PYEOF || true
import json
d = json.load(open('$OPINION_FILE'))
obs = d.get('observations', [])
print('  Agent: %s' % d.get('agent', '?'))
print('  Assessment: %s' % d.get('assessment', '?'))
print('  Observation count: %d' % len(obs))
print('  Signals used: %s' % ', '.join(d.get('signals_used', [])))
if obs:
    severities = {}
    for o in obs:
        s = o.get('severity', 'unknown')
        severities[s] = severities.get(s, 0) + 1
    print('  Severity breakdown: %s' % severities)
PYEOF

echo ""

# Content checklist (scored by human or LLM evaluator)
cat << 'CHECKLIST'
  C1 [__/10] Coverage: Are observations spread across scenes/timeline?
             (No large gaps; at least 1 observation per meaningful segment)

  C2 [__/10] Specificity: Do observations contain measurable/concrete details?
             (Values like "3800K", "IRE 5", "50mm", severity ratings — not vague impressions)

  C3 [__/10] Scope adherence: Does the agent stay within IDENTITY.md boundaries?
             (Colorist doesn't comment on editing pace; cinematographer ignores sound)

  C4 [__/10] Grounding: Are observations traceable to KNOWLEDGE.md?
             (Uses terminology and standards from the knowledge base)

  C5 [__/10] Taste alignment: Do suggestions reflect TASTE.md preferences?
             (References the agent's aesthetic framework, flags its anti-patterns)

  C6 [__/10] USER.md respect: Does output honor director preferences?
             (If USER.md says "muted earth tones", colorist doesn't suggest saturation boost)
CHECKLIST

echo ""
echo "--- Structural Score: $SCORE / $MAX ---"
for note in "${NOTES[@]+"${NOTES[@]}"}"; do
  echo "$note"
done
echo ""
echo "Fill in C1-C6 manually (or pipe this opinion JSON + agent context to an LLM evaluator)."
echo "Total = Structural ($SCORE/$MAX) + Content (__/60) = __/100"
