#!/bin/sh
# Render the Homebrew formula from the release checksums.
#
#   sh packaging/homebrew/render.sh <version> <checksums.txt> <template> > video-perception.rb
#
# Every asset the template names must appear in checksums.txt; a missing
# checksum is an error so a formula never ships with an empty sha256.
set -eu

VERSION="$1"
CHECKSUMS="$2"
TEMPLATE="$3"

sum_for() {
  name="$1"
  sum=$(awk -v name="$name" '$2 == name || $2 == "*" name { print $1; exit }' "$CHECKSUMS")
  if [ -z "$sum" ]; then
    echo "render.sh: no checksum for $name in $CHECKSUMS" >&2
    exit 1
  fi
  printf '%s' "$sum"
}

SHA_DARWIN_ARM64=$(sum_for "video-perception_${VERSION}_darwin_arm64.tar.gz")
SHA_DARWIN_AMD64=$(sum_for "video-perception_${VERSION}_darwin_amd64.tar.gz")
SHA_LINUX_AMD64=$(sum_for "video-perception_${VERSION}_linux_amd64.tar.gz")

sed \
  -e "s/@VERSION@/${VERSION}/g" \
  -e "s/@SHA_DARWIN_ARM64@/${SHA_DARWIN_ARM64}/g" \
  -e "s/@SHA_DARWIN_AMD64@/${SHA_DARWIN_AMD64}/g" \
  -e "s/@SHA_LINUX_AMD64@/${SHA_LINUX_AMD64}/g" \
  "$TEMPLATE"
