#!/usr/bin/env bash
# Fails if tracked docs contain business details that belong in the private repo.
# AGENTS.md is skipped because it names these terms to forbid them.
# A false positive means the pattern needs narrowing here, not the doc rewording.
set -euo pipefail

pattern='\$[0-9]+|\b(ARR|MRR|revenue|churn|CAC|LTV|runway|valuation|fundrais\w*|investors?|term sheet|margin|conversion rate|go-to-market|GTM)\b'

if git ls-files -z -- '*.md' ':!AGENTS.md' | xargs -0 grep -nIiE "$pattern"; then
  echo >&2 "Business content found above. It belongs in the private repo, not here."
  exit 1
fi
echo "No business content found."
