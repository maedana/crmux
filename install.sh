#!/bin/sh
set -eu

REPO="maedana/crmux"
INSTALL_DIR="${HOME}/.local/bin"

# Detect OS and architecture
OS=$(uname -s)
ARCH=$(uname -m)

case "${OS}" in
  Linux)
    case "${ARCH}" in
      x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
      *) echo "Error: unsupported architecture: ${ARCH}" >&2; exit 1 ;;
    esac
    ;;
  Darwin)
    case "${ARCH}" in
      x86_64) TARGET="x86_64-apple-darwin" ;;
      arm64)  TARGET="aarch64-apple-darwin" ;;
      *) echo "Error: unsupported architecture: ${ARCH}" >&2; exit 1 ;;
    esac
    ;;
  *)
    echo "Error: unsupported OS: ${OS}" >&2
    exit 1
    ;;
esac

echo "Detected platform: ${TARGET}"

# Get latest release tag
TAG=$(curl -sSL "https://api.github.com/repos/${REPO}/releases/latest" | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p')
if [ -z "${TAG}" ]; then
  echo "Error: failed to fetch latest release tag" >&2
  exit 1
fi

echo "Latest release: ${TAG}"

# Download and extract
URL="https://github.com/${REPO}/releases/download/${TAG}/crmux-${TARGET}.tar.gz"
echo "Downloading ${URL}..."

TMPDIR=$(mktemp -d)
trap 'rm -rf "${TMPDIR}"' EXIT

curl -sSL "${URL}" -o "${TMPDIR}/crmux.tar.gz"
tar xzf "${TMPDIR}/crmux.tar.gz" -C "${TMPDIR}"

# Install
mkdir -p "${INSTALL_DIR}"
mv "${TMPDIR}/crmux" "${INSTALL_DIR}/crmux"
chmod +x "${INSTALL_DIR}/crmux"

echo "Installed crmux to ${INSTALL_DIR}/crmux"

# Check PATH
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo ""
    echo "WARNING: ${INSTALL_DIR} is not in your PATH."
    echo "Add the following to your shell profile:"
    echo "  export PATH=\"${INSTALL_DIR}:\${PATH}\""
    ;;
esac
