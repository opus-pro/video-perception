#!/usr/bin/env bash
# Publish a GitHub Release from the assets the Release workflow built.
#
# The Release workflow uploads everything it assembled (binaries, wheels, the
# plugin bundle, checksums.txt) as the `release-assets` artifact before it tries
# to create the GitHub Release itself. When that last step cannot run on the
# hosted runner, for example because the repository's organization has an IP
# allow list that rejects GitHub API calls from GitHub-hosted runners, a
# maintainer publishes the same files from their own machine with this script.
#
# Usage: scripts/publish-release.sh [--replace] <tag> [run-id]
#   --replace  allow replacing the assets of a release that already exists
#   <tag>      the release tag, e.g. v0.3.0 (must already be pushed)
#   [run-id]   a Release workflow run id; defaults to the latest run for <tag>
#
# The run must be a push-triggered run of .github/workflows/release.yml whose
# head commit is the commit <tag> points to; anything else is refused. Only the
# files listed in checksums.txt are uploaded, after their hashes are verified.
#
# Requires the GitHub CLI (gh) authenticated with write access to the repo.
set -euo pipefail

REPLACE=0
if [ "${1:-}" = "--replace" ]; then
  REPLACE=1
  shift
fi
TAG="${1:?usage: scripts/publish-release.sh [--replace] <tag> [run-id]}"
RUN_ID="${2:-}"
REPO="${GH_REPO:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"
WORKFLOW_PATH=".github/workflows/release.yml"

# The commits endpoint dereferences annotated tags to the tagged commit.
TAG_SHA="$(gh api "repos/$REPO/commits/$TAG" --jq .sha)"

if [ -z "$RUN_ID" ]; then
  RUN_ID="$(gh run list --repo "$REPO" --workflow release.yml --branch "$TAG" \
    --event push --limit 1 --json databaseId --jq '.[0].databaseId')"
fi
if [ -z "$RUN_ID" ]; then
  echo "publish-release: no Release workflow run found for $TAG in $REPO" >&2
  exit 1
fi

read -r RUN_EVENT RUN_PATH RUN_SHA < <(
  gh api "repos/$REPO/actions/runs/$RUN_ID" --jq '[.event, .path, .head_sha] | @tsv'
)
if [ "$RUN_EVENT" != "push" ] || [ "$RUN_PATH" != "$WORKFLOW_PATH" ] || [ "$RUN_SHA" != "$TAG_SHA" ]; then
  echo "publish-release: run $RUN_ID is not a Release workflow run for $TAG" >&2
  echo "  run: event=$RUN_EVENT path=$RUN_PATH head_sha=$RUN_SHA" >&2
  echo "  tag: commit=$TAG_SHA" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "publish-release: downloading release-assets from run $RUN_ID ($RUN_SHA)" >&2
gh run download "$RUN_ID" --repo "$REPO" --name release-assets --dir "$WORK"
cd "$WORK"
shasum -a 256 -c checksums.txt
mapfile -t ASSETS < <(awk '{print $2}' checksums.txt)

if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  if [ "$REPLACE" -ne 1 ]; then
    echo "publish-release: $TAG already exists; pass --replace to overwrite its assets" >&2
    exit 1
  fi
  echo "publish-release: replacing the assets of $TAG" >&2
  gh release upload "$TAG" --repo "$REPO" --clobber "${ASSETS[@]}" checksums.txt
else
  gh release create "$TAG" --repo "$REPO" \
    --title "video-perception $TAG" \
    --generate-notes \
    --verify-tag \
    "${ASSETS[@]}" checksums.txt
fi
