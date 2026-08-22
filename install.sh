#!/usr/bin/env bash
# Install `gate` without a local npm install.
#
# Primary path: fetch the single-file binary for this platform from the
# latest GitHub release and place it on PATH. Binaries have shipped since
# v0.3.0. Falls back to build-from-source instructions when no matching
# binary asset exists for this platform (an older release, or a platform
# outside the build matrix) - gate is not published to npm and, per the
# Milestone 6 owner decision recorded in ROADMAP.md, npm
# will never be the user-facing install path.
#
# Built platforms (keep in sync with .github/workflows/release.yml's
# build-binaries matrix - update both together when adding a platform):
# linux-x64, darwin-arm64.
#
# Usage: curl -fsSL https://raw.githubusercontent.com/brolyssjl/gate/main/install.sh | bash
set -euo pipefail

REPO="brolyssjl/gate"
BIN_NAME="gate"
INSTALL_DIR="${GATE_INSTALL_DIR:-$HOME/.local/bin}"
BUILT_PLATFORMS="linux-x64, darwin-arm64"

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

download_asset() {
  local asset="$1" dest="$2"
  # /releases/latest/download/<asset> redirects straight to the current
  # release's asset - no need to resolve the tag via the (rate-limited)
  # api.github.com first. Works unauthenticated once the repo is public.
  local url="https://github.com/${REPO}/releases/latest/download/${asset}"
  if curl -fsSL "$url" -o "$dest"; then
    return 0
  fi
  # While the repo is private, unauthenticated asset downloads 404 even when
  # the asset exists. gh reuses your existing auth and sees the same assets.
  if command -v gh >/dev/null 2>&1; then
    echo "Direct download failed - retrying via gh (needed while the repo is private)..." >&2
    if gh release download --repo "$REPO" --pattern "$asset" --output "$dest" --clobber; then
      return 0
    fi
  fi
  return 1
}

install_binary() {
  local platform="$1" cpu="$2"
  local asset="${BIN_NAME}-${platform}-${cpu}"
  local tmp
  tmp="$(mktemp "${TMPDIR:-/tmp}/${BIN_NAME}.XXXXXX")"

  echo "Downloading ${asset}..."
  mkdir -p "$INSTALL_DIR"
  # Download to a temp file first: a mid-transfer failure must never leave a
  # truncated (but still +x, still shadowing-the-fallback) binary in place.
  if ! download_asset "$asset" "$tmp"; then
    rm -f "$tmp"
    return 1
  fi
  chmod +x "$tmp"
  mv "$tmp" "$INSTALL_DIR/$BIN_NAME"
  echo "Installed to $INSTALL_DIR/$BIN_NAME"
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Add it to your PATH: export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
  esac
  return 0
}

install_from_source() {
  echo "No matching binary release (built for: ${BUILT_PLATFORMS})." >&2
  echo "gate is not published to npm - build from source instead:" >&2
  echo "  git clone https://github.com/${REPO}.git" >&2
  echo "  cd gate" >&2
  echo "  cargo build --release --manifest-path rust/Cargo.toml" >&2
  echo "Then put rust/target/release/gate on your PATH." >&2
  if ! command -v cargo >/dev/null 2>&1; then
    echo "(cargo not found - install Rust from https://rustup.rs)" >&2
  fi
  exit 1
}

main() {
  local platform cpu
  platform="$(os)"
  cpu="$(arch)"

  if [ "$platform" = "unsupported" ] || [ "$cpu" = "unsupported" ]; then
    echo "Unrecognized platform ($(uname -s) $(uname -m); built for: ${BUILT_PLATFORMS})." >&2
    install_from_source
    return
  fi

  if ! install_binary "$platform" "$cpu"; then
    install_from_source
  fi
}

main "$@"
