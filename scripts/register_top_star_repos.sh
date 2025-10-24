#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE' >&2
Usage: register_top_star_repos.sh [--count N] [--priority P]

Registers the top-N starred public GitHub repositories into the collector queue
via the API's /repos endpoint (default N=50, priority=0).

Environment:
  - API_BASE_URL or API_URL: Base URL for the API (default http://localhost:3000)
  - GITHUB_API_BASE_URL: Base URL for GitHub API (default https://api.github.com)
  - GITHUB_TOKEN: Optional token to increase rate limits for GitHub API

Requires: jq, curl
USAGE
}

COUNT=50
PRIORITY=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --count)
      COUNT=${2:-}
      shift 2
      ;;
    --priority)
      PRIORITY=${2:-}
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if ! command -v jq >/dev/null 2>&1; then
  echo "Error: jq is required." >&2
  exit 2
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "Error: curl is required." >&2
  exit 2
fi

API_BASE_URL=${API_BASE_URL:-${API_URL:-http://localhost:3000}}
GITHUB_API_BASE_URL=${GITHUB_API_BASE_URL:-https://api.github.com}

if ! [[ $COUNT =~ ^[0-9]+$ ]]; then
  echo "--count must be a positive integer" >&2
  exit 3
fi
if ! [[ $PRIORITY =~ ^-?[0-9]+$ ]]; then
  echo "--priority must be an integer" >&2
  exit 3
fi

# Build GitHub API headers
AUTH_HEADER=()
if [[ -n "${GITHUB_TOKEN:-}" ]]; then
  AUTH_HEADER=( -H "Authorization: token ${GITHUB_TOKEN}" )
fi

echo "Fetching top ${COUNT} starred repositories from GitHub…" >&2
resp=$(curl --fail --show-error --silent \
  -H "Accept: application/vnd.github+json" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  -H "User-Agent: github-spam-lab-register-script" \
  "${GITHUB_API_BASE_URL}/search/repositories?q=stars:%3E0&sort=stars&order=desc&per_page=${COUNT}&page=1" \
  "${AUTH_HEADER[@]}" )

owners=( $(jq -r '.items[] | .owner.login' <<<"$resp") )
names=( $(jq -r '.items[] | .name' <<<"$resp") )

if [[ ${#owners[@]} -eq 0 || ${#owners[@]} -ne ${#names[@]} ]]; then
  echo "No repositories parsed from GitHub response; aborting." >&2
  exit 4
fi

echo "Registering ${#owners[@]} repositories to ${API_BASE_URL} with priority ${PRIORITY}…" >&2

registered=0
for i in "${!owners[@]}"; do
  owner=${owners[$i]}
  repo=${names[$i]}
  payload=$(jq -n --arg owner "$owner" --arg name "$repo" --argjson priority "$PRIORITY" '{owner:$owner,name:$name,priority:$priority}')
  # Use API directly to avoid subshells; prints minimal output
  curl --fail --show-error --silent \
    -H "Content-Type: application/json" \
    -X POST \
    -d "$payload" \
    "$API_BASE_URL/repos" >/dev/null && registered=$((registered+1)) || {
      echo "Failed to register ${owner}/${repo}" >&2
    }
done

echo "Done. Registered ${registered}/${#owners[@]} repositories." >&2

