#!/usr/bin/env bash
# Install `gate` without a local npm install.
#
# Primary path: fetch the single-file binary for this platform from the
# latest GitHub release, verify it against the release's SHA256SUMS, and
# place it on PATH. Binaries have shipped since v0.3.0. Falls back to
# build-from-source instructions when no matching binary asset exists for
# this platform (an older release, or a platform outside the build matrix)
# - gate is not published to npm and, per the Milestone 6 owner decision
# recorded in ROADMAP.md, npm will never be the user-facing install path.
#
# Built platforms (keep in sync with .github/workflows/release.yml's
# build-binaries matrix - update both together when adding a platform):
# linux-x64, darwin-arm64.
#
# Env vars:
#   GATE_INSTALL_DIR  where to put the binary (default: $HOME/.local/bin)
#   GATE_VERSION      pin an exact release, e.g. GATE_VERSION=0.4.0
#                     (default: the latest release)
#
# Usage: curl -fsSL https://raw.githubusercontent.com/brolyssjl/gate/main/install.sh | bash
set -euo pipefail

REPO="brolyssjl/gate"
BIN_NAME="gate"
SUMS_NAME="SHA256SUMS"
INSTALL_DIR="${GATE_INSTALL_DIR:-$HOME/.local/bin}"
BUILT_PLATFORMS="linux-x64, darwin-arm64"
VERSION="${GATE_VERSION:-}"

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

# https://github.com/OWNER/REPO/releases/latest/download/<asset> redirects
# straight to the current release's asset - no need to resolve the tag via
# the (rate-limited) api.github.com first. Pinned via $GATE_VERSION, it
# targets that tag's own asset path instead. Works unauthenticated once the
# repo is public.
release_url() {
  local asset="$1"
  if [ -n "$VERSION" ]; then
    echo "https://github.com/${REPO}/releases/download/v${VERSION}/${asset}"
  else
    echo "https://github.com/${REPO}/releases/latest/download/${asset}"
  fi
}

download_asset() {
  local asset="$1" dest="$2"
  local url
  url="$(release_url "$asset")"
  if curl -fsSL "$url" -o "$dest"; then
    return 0
  fi
  # While the repo is private, unauthenticated asset downloads 404 even when
  # the asset exists. gh reuses your existing auth and sees the same assets.
  if command -v gh >/dev/null 2>&1; then
    echo "Direct download failed - retrying via gh (needed while the repo is private)..." >&2
    local gh_args=(--repo "$REPO" --pattern "$asset" --output "$dest" --clobber)
    if [ -n "$VERSION" ]; then
      gh_args=("v${VERSION}" "${gh_args[@]}")
    fi
    if gh release download "${gh_args[@]}"; then
      return 0
    fi
  fi
  return 1
}

# Verify `file` (downloaded as release asset `asset`) against the one line
# naming it in `sums_file` (a SHA256SUMS in the standard `sha256sum`/`shasum`
# format: "<hex digest>  <filename>" per line). Checks just that one line,
# via each tool's own `-c` mode fed on stdin, run from the file's own
# directory so the checksum line's bare filename resolves without requiring
# every other release asset to be present alongside it.
verify_checksum() {
  local file="$1" asset="$2" sums_file="$3"

  local checker
  if command -v sha256sum >/dev/null 2>&1; then
    checker="sha256sum"
  elif command -v shasum >/dev/null 2>&1; then
    checker="shasum -a 256"
  else
    echo "Neither sha256sum nor shasum is available - cannot verify the download." >&2
    return 1
  fi

  local hash
  hash="$(awk -v f="$asset" '$2 == f { print $1; exit }' "$sums_file")"
  if [ -z "$hash" ]; then
    echo "SHA256SUMS has no entry for ${asset} - refusing to install." >&2
    return 1
  fi

  local dir base
  dir="$(dirname "$file")"
  base="$(basename "$file")"
  if ! (cd "$dir" && printf '%s  %s\n' "$hash" "$base" | $checker -c - >/dev/null 2>&1); then
    echo "Checksum verification FAILED for ${asset} - the download may be corrupted or tampered with. Nothing installed." >&2
    return 1
  fi
  return 0
}

install_binary() {
  local platform="$1" cpu="$2"
  local asset="${BIN_NAME}-${platform}-${cpu}"
  local tmp tmp_sums
  tmp="$(mktemp "${TMPDIR:-/tmp}/${BIN_NAME}.XXXXXX")"
  tmp_sums="$(mktemp "${TMPDIR:-/tmp}/${BIN_NAME}.XXXXXX.sums")"
  # A mid-transfer failure, or a failed checksum, must never leave a
  # truncated or tampered (but still +x, still shadowing-the-fallback)
  # binary in place - everything below downloads to temp files first and
  # only chmod+x/mv's the real destination once verification passed.
  # Cleanup is explicit per path: a RETURN trap set here stays armed for
  # every later function return, where these locals no longer exist and
  # `set -u` turns the dangling expansion into an error.

  echo "Downloading ${asset}$( [ -n "$VERSION" ] && echo " (v${VERSION})" )..."
  mkdir -p "$INSTALL_DIR"
  if ! download_asset "$asset" "$tmp"; then
    rm -f "$tmp" "$tmp_sums"
    return 1
  fi
  if ! download_asset "$SUMS_NAME" "$tmp_sums"; then
    echo "Could not download ${SUMS_NAME} - refusing to install an unverified binary." >&2
    rm -f "$tmp" "$tmp_sums"
    return 1
  fi
  if ! verify_checksum "$tmp" "$asset" "$tmp_sums"; then
    rm -f "$tmp" "$tmp_sums"
    return 1
  fi

  chmod +x "$tmp"
  mv "$tmp" "$INSTALL_DIR/$BIN_NAME"
  rm -f "$tmp_sums"
  local installed_version
  installed_version="$("$INSTALL_DIR/$BIN_NAME" --version 2>/dev/null || echo "unknown")"
  echo "Installed gate v${installed_version} to $INSTALL_DIR/$BIN_NAME (checksum verified)"
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

# Only run on direct execution, not when sourced (e.g. to unit-test
# verify_checksum in isolation without a network call - see
# CONTRIBUTING.md's "Release checksums" section).
if [ "${BASH_SOURCE[0]}" = "${0}" ]; then
  main "$@"
fi
