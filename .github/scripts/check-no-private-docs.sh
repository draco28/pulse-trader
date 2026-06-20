#!/usr/bin/env bash
set -euo pipefail
# Fail if any private-class doc filename appears tracked in the public repo.
FORBIDDEN=(PRD.md SRS.md BACKLOG.md PROJECT_PLAN.md MODEL_CARD.md RISK_REGISTER.md \
  THREAT_MODEL.md EVALS_PLAN.md CUTOVER_PLAN.md TEST_STRATEGY.md ROADMAP.md)
hits=0
for f in "${FORBIDDEN[@]}"; do
  if git ls-files "**/$f" "$f" | grep -q .; then
    echo "::error::private-class doc present in public repo: $f"; hits=1
  fi
done
[ "$hits" -eq 0 ] && echo "OK: no private-class docs in public tree"
exit "$hits"
