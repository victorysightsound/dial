#!/bin/sh
set -e

# DIAL installer - downloads the correct prebuilt binary for your platform.
# Usage: curl -fsSL https://raw.githubusercontent.com/victorysightsound/dial/main/install.sh | sh

REPO="victorysightsound/dial"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

# Detect OS
case "$(uname -s)" in
    Linux)  OS="unknown-linux-gnu" ;;
    Darwin) OS="apple-darwin" ;;
    *)      echo "Error: Unsupported OS: $(uname -s)"; exit 1 ;;
esac

# Detect architecture
case "$(uname -m)" in
    x86_64|amd64)  ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *)             echo "Error: Unsupported architecture: $(uname -m)"; exit 1 ;;
esac

TARGET="${ARCH}-${OS}"

# Get latest release tag
echo "Finding latest release..."
LATEST=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"([^"]+)".*/\1/')

if [ -z "$LATEST" ]; then
    echo "Error: Could not determine latest release"
    exit 1
fi

echo "Latest release: ${LATEST}"
echo "Installing dial globally for the current user into ${INSTALL_DIR}"

# Download
URL="https://github.com/${REPO}/releases/download/${LATEST}/dial-${TARGET}.tar.gz"
echo "Downloading dial for ${TARGET}..."

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

curl -fsSL "$URL" -o "$TMPDIR/dial.tar.gz"
tar xzf "$TMPDIR/dial.tar.gz" -C "$TMPDIR"

# Install
mkdir -p "$INSTALL_DIR"
mv "$TMPDIR/dial" "$INSTALL_DIR/dial"
chmod +x "$INSTALL_DIR/dial"

echo ""
echo "Installed dial ${LATEST} to ${INSTALL_DIR}/dial"

# Check PATH
case ":$PATH:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        echo ""
        echo "Note: ${INSTALL_DIR} is not in your PATH. Add it once for your user:"
        echo "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.bashrc"
        echo ""
        echo "Then restart your shell or run: source ~/.bashrc"
        ;;
esac

echo ""
echo "Run 'dial --version' to verify."
