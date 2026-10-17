#!/bin/sh
set -eu

REPO="opus-pro/video-perception"
BINARY="video-perception"
REQUESTED_VERSION="${VIDEO_PERCEPTION_VERSION:-}"
DEFAULT_INSTALL_DIR="/usr/local/bin"
FALLBACK_INSTALL_DIR="${HOME}/.local/bin"
INSTALL_DIR="${VIDEO_PERCEPTION_INSTALL_DIR:-}"

for command in curl tar; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "Error: $command is required"
    exit 1
  fi
done

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)
case "$ARCH" in
  x86_64) ARCH="amd64" ;;
  arm64|aarch64) ARCH="arm64" ;;
  *) echo "Error: unsupported architecture: $ARCH"; exit 1 ;;
esac
case "$OS" in
  darwin|linux) ;;
  *) echo "Error: unsupported OS: $OS"; exit 1 ;;
esac
if [ "$OS" = "linux" ] && [ "$ARCH" != "amd64" ]; then
  echo "Error: published Linux builds currently require x86_64"
  exit 1
fi

if [ -z "$INSTALL_DIR" ]; then
  if [ -w "$DEFAULT_INSTALL_DIR" ]; then
    INSTALL_DIR="$DEFAULT_INSTALL_DIR"
  else
    INSTALL_DIR="$FALLBACK_INSTALL_DIR"
  fi
fi
mkdir -p "$INSTALL_DIR"

if [ -n "$REQUESTED_VERSION" ]; then
  VERSION="$REQUESTED_VERSION"
else
  VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    | head -n 1)
fi
case "$VERSION" in
  v*) ;;
  *) VERSION="v${VERSION}" ;;
esac
if [ "$VERSION" = "v" ]; then
  echo "Error: could not resolve the latest release"
  exit 1
fi

ASSET_NAME="${BINARY}_${VERSION#v}_${OS}_${ARCH}.tar.gz"
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM
BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

echo "==> Downloading ${ASSET_NAME}"
curl -fL --retry 3 -o "${TMP_DIR}/${ASSET_NAME}" "${BASE_URL}/${ASSET_NAME}"
curl -fL --retry 3 -o "${TMP_DIR}/checksums.txt" "${BASE_URL}/checksums.txt"

EXPECTED_SUM=$(awk -v name="$ASSET_NAME" '$2 == name || $2 == "*" name { print $1; exit }' "${TMP_DIR}/checksums.txt")
if [ -z "$EXPECTED_SUM" ]; then
  echo "Error: checksum for ${ASSET_NAME} not found"
  exit 1
fi
if command -v shasum >/dev/null 2>&1; then
  ACTUAL_SUM=$(shasum -a 256 "${TMP_DIR}/${ASSET_NAME}" | awk '{print $1}')
elif command -v sha256sum >/dev/null 2>&1; then
  ACTUAL_SUM=$(sha256sum "${TMP_DIR}/${ASSET_NAME}" | awk '{print $1}')
else
  echo "Error: shasum or sha256sum is required"
  exit 1
fi
if [ "$EXPECTED_SUM" != "$ACTUAL_SUM" ]; then
  echo "Error: checksum mismatch for ${ASSET_NAME}"
  exit 1
fi

tar -xzf "${TMP_DIR}/${ASSET_NAME}" -C "$TMP_DIR" "$BINARY"
if [ -w "$INSTALL_DIR" ]; then
  install -m 0755 "${TMP_DIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
else
  sudo install -m 0755 "${TMP_DIR}/${BINARY}" "${INSTALL_DIR}/${BINARY}"
fi

echo "Installed ${BINARY} ${VERSION} to ${INSTALL_DIR}/${BINARY}"
echo "Next: video-perception capabilities && video-perception doctor"
"${INSTALL_DIR}/${BINARY}" --version
