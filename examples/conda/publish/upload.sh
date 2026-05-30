#!/usr/bin/env bash
# Upload a conda package to a BatleHub private conda registry.
#
# Usage:
#   export BATLEHUB_TOKEN=your-token
#   ./upload.sh linux-64 numpy-1.26.0-py311h0_0.tar.bz2
#
# The platform is inferred from the package subdir field if not provided.

set -euo pipefail

REGISTRY_URL="${REGISTRY_URL:-http://localhost:8080/proxy/my-conda}"
PLATFORM="${1:?usage: $0 <platform> <package.tar.bz2|.conda>}"
PACKAGE="${2:?usage: $0 <platform> <package.tar.bz2|.conda>}"
TOKEN="${BATLEHUB_TOKEN:-change-me-user-token}"

echo "Uploading $PACKAGE to $REGISTRY_URL/$PLATFORM/ ..."
curl -fsSL \
  -X POST \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/octet-stream" \
  --data-binary "@$PACKAGE" \
  "$REGISTRY_URL/$PLATFORM/"
echo ""
echo "Done."
