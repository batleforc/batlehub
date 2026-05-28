#!/usr/bin/env bash
# Download GitHub release assets through the batlehub proxy.
#
# The proxy mirrors:
#   https://github.com/{owner}/{repo}/releases/download/{tag}/{file}
#                         ↓
#   http://localhost:8080/proxy/my-github/{owner}/{repo}/releases/download/{tag}/{file}
#
# Benefits: artifact is cached after the first download; no GitHub rate-limit
# exposure on repeated CI pulls; bandwidth stays on-prem.

set -euo pipefail

PROXY="http://localhost:8080/proxy/my-github"
TOKEN="change-me-user-token"

# ── helpers ───────────────────────────────────────────────────────────────────

proxy_download() {
  local owner="$1" repo="$2" tag="$3" file="$4" dest="$5"
  local url="${PROXY}/${owner}/${repo}/releases/download/${tag}/${file}"
  echo "Downloading ${url}"
  curl -fsSL \
    -H "Authorization: Bearer ${TOKEN}" \
    -o "${dest}" \
    "${url}"
}

proxy_latest_release() {
  local owner="$1" repo="$2"
  curl -fsSL \
    -H "Authorization: Bearer ${TOKEN}" \
    "${PROXY}/${owner}/${repo}/releases" \
    | jq -r '.[0].tag_name'
}

# ── example: download the latest kubectl binary ───────────────────────────────

OWNER="kubernetes"
REPO="kubernetes"
TAG=$(proxy_latest_release "${OWNER}" "${REPO}")
FILE="kubectl-linux-amd64"

proxy_download "${OWNER}" "${REPO}" "${TAG}" "${FILE}" "./kubectl"
chmod +x ./kubectl
echo "kubectl ${TAG} downloaded."
