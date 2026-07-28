#!/usr/bin/env bash
# Install `gate` without a local npm install.
#
# Primary path: fetch the single-file binary for this platform from the
# latest GitHub release and place it on PATH. Falls back to `npm install -g`
# when Node is available and no matching binary asset exists yet (binaries
# ship once the owner wires scripts/buildBinary.mjs into a release job -
# mechanical prep only, not yet part of any published release).
#
# Usage: curl -fsSL https://raw.githubusercontent.com/brolyssjl/gate/main/install.sh | bash
set -euo pipefail

REPO="brolyssjl/gate"
BIN_NAME="gate"
INSTALL_DIR="${GATE_INSTALL_DIR:-$HOME/.local/bin}"

os() {
  case "$(uname -s)" in
    Linux) echo "linux" ;;
    Darwin) echo "darwin" ;;
    *) echo "unsupported" ;;
  esac
}

arch() {
  case "$(uname -m)" in
    x86_64|amd64) echo "x64" ;;
    arm64|aarch64) echo "arm64" ;;
    *) echo "unsupported" ;;
  esac
}

latest_tag() {
  curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/'
}

install_binary() {
  local platform="$1" cpu="$2" tag="$3"
  local asset="${BIN_NAME}-${platform}-${cpu}"
  local url="https://github.com/${REPO}/releases/download/${tag}/${asset}"

  echo "Downloading ${asset} (${tag})..."
  mkdir -p "$INSTALL_DIR"
  if ! curl -fsSL "$url" -o "$INSTALL_DIR/$BIN_NAME"; then
    return 1
  fi
  chmod +x "$INSTALL_DIR/$BIN_NAME"
  echo "Installed to $INSTALL_DIR/$BIN_NAME"
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Add it to your PATH: export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
  esac
  return 0
}

install_via_npm() {
  if ! command -v npm >/dev/null 2>&1; then
    echo "No matching binary release and no npm found. Install Node >= 20, then:" >&2
    echo "  npm install -g gate-cli" >&2
    exit 1
  fi
  echo "No matching binary release yet - installing via npm instead."
  npm install -g gate-cli
}

main() {
  local platform cpu
  platform="$(os)"
  cpu="$(arch)"

  if [ "$platform" = "unsupported" ] || [ "$cpu" = "unsupported" ]; then
    echo "Unrecognized platform ($(uname -s) $(uname -m)) - falling back to npm." >&2
    install_via_npm
    return
  fi

  local tag
  if ! tag="$(latest_tag)" || [ -z "$tag" ]; then
    echo "Could not determine the latest release - falling back to npm." >&2
    install_via_npm
    return
  fi

  if ! install_binary "$platform" "$cpu" "$tag"; then
    install_via_npm
  fi
}

main "$@"
