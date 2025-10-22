#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE' >&2
Usage: register_repo.sh <owner> <repo> [priority]
Set API_BASE_URL or API_URL to override the default http://localhost:3000 target.
USAGE
  exit 1
}

if [[ ${1:-} == "-h" || ${1:-} == "--help" ]]; then
  usage
fi

if [[ $# -lt 2 || $# -gt 3 ]]; then
  usage
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "Error: jq is required to build the JSON payload." >&2
  exit 2
fi

API_BASE_URL=${API_BASE_URL:-${API_URL:-http://localhost:3000}}
OWNER=$1
REPO=$2
PRIORITY=${3:-0}

if ! [[ $PRIORITY =~ ^-?[0-9]+$ ]]; then
  echo "Priority must be an integer." >&2
  exit 3
fi

payload=$(jq -n --arg owner "$OWNER" --arg name "$REPO" --argjson priority "$PRIORITY" \
  '{owner:$owner,name:$name,priority:$priority}')

curl --fail --show-error --silent \
  -H "Content-Type: application/json" \
  -X POST \
  -d "$payload" \
  "$API_BASE_URL/repos" | jq
