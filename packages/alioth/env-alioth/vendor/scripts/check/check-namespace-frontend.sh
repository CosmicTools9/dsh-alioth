#!/usr/bin/env bash
# =============================================================================
# check-namespace-frontend.sh
#
# Shell wrapper for the Bun-based namespace module frontend consistency check.
# See scripts/check/check-namespace-frontend.ts for implementation details.
#
# Usage:
#   bash scripts/check/check-namespace-frontend.sh
#
# Exit codes:
#   0  all non-baseline module frontends satisfy the minimum engineering skeleton
#   1  at least one non-baseline module frontend fails a MUST-level check
# =============================================================================
set -euo pipefail

exec bun run "$(dirname "$0")/check-namespace-frontend.ts" "$@"
