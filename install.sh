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
#   GATE_KEEP_OLD_INSTALLS  set to 1 to keep obsolete pre-Rust install dirs
#                     (~/.local/share/brainstorm-tools/gate-v*) instead of
#                     removing them after a verified install
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
# targets that tag's own asset path instead. Works unauthenticated.
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
  # Fallback for a network hiccup or GitHub API rate limiting on an
  # unauthenticated request. gh reuses your existing auth and sees the same
  # assets.
  if command -v gh >/dev/null 2>&1; then
    echo "Direct download failed - retrying via gh (uses your existing GitHub auth)..." >&2
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

# Best-effort build-provenance check on top of the checksum above. The
# checksum only proves the binary matches SHA256SUMS from the same release -
# it says nothing if both were replaced together (a compromised GitHub
# account or Actions token can do that). `gh attestation verify` checks the
# binary against the SLSA provenance attestation release.yml records via
# `actions/attest-build-provenance`, which is signed through GitHub's OIDC
# issuer and Sigstore, not just committed alongside the asset.
#
# Skipped (with a note, not a failure) when `gh` isn't installed, since it's
# the only tool that can check this. Treated as a pass when the release
# predates attestations (no attestations found - older releases have none).
# Any other failure aborts the install and removes the temp files, same as a
# checksum mismatch.
verify_provenance() {
  local file="$1" asset="$2"

  if ! command -v gh >/dev/null 2>&1; then
    echo "Note: gh not found - build provenance not checked (checksum verified above). Install gh and re-run to also verify: gh attestation verify <file> --repo ${REPO}" >&2
    return 0
  fi

  local out
  if out="$(gh attestation verify "$file" --repo "$REPO" 2>&1)"; then
    echo "$out"
    echo "provenance verified"
    return 0
  fi

  if echo "$out" | grep -qi "no attestations found"; then
    echo "Note: no build attestations found for ${asset} (older releases predate provenance) - continuing on checksum verification alone." >&2
    return 0
  fi

  echo "$out" >&2
  echo "provenance verification FAILED for ${asset} - the release may have been tampered with. Nothing installed." >&2
  return 1
}

install_binary() {
  local platform="$1" cpu="$2"
  local asset="${BIN_NAME}-${platform}-${cpu}"
  local tmp tmp_sums
  tmp="$(mktemp "${TMPDIR:-/tmp}/${BIN_NAME}.XXXXXX")"
  # BSD mktemp (macOS) only substitutes a trailing run of X's, so a suffix
  # after them (".sums") used to make the whole template literal - two
  # concurrent installs collided on the exact same path. Keep the X's last.
  tmp_sums="$(mktemp "${TMPDIR:-/tmp}/${BIN_NAME}-sums.XXXXXX")"
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
  if ! verify_provenance "$tmp" "$asset"; then
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
  clean_legacy_install_dirs
  return 0
}

# gate predates this binary-release install path: an npm-era prototype
# installed itself under ~/.local/share/brainstorm-tools/gate-v*. That path
# is dead (Milestone 6's TypeScript retirement, see ROADMAP.md) - a stale
# copy could quietly shadow the binary this script just installed. The dirs
# match a pattern only this installer ever created, so they are ours to
# remove. Called only after a verified install succeeded, never as a
# pre-step; set GATE_KEEP_OLD_INSTALLS=1 to keep them.
clean_legacy_install_dirs() {
  local legacy_root="$HOME/.local/share/brainstorm-tools"
  local d
  for d in "$legacy_root"/gate-v*; do
    [ -d "$d" ] || continue
    if [ "${GATE_KEEP_OLD_INSTALLS:-}" = "1" ]; then
      echo "Note: keeping obsolete pre-Rust install dir (GATE_KEEP_OLD_INSTALLS=1): ${d}"
      continue
    fi
    if rm -rf "$d" 2>/dev/null; then
      echo "Removed obsolete pre-Rust gate install dir: ${d}"
    else
      echo "Note: could not remove obsolete install dir ${d} - safe to delete manually." >&2
    fi
  done
  # Drop the shared root once the last tool's dir is gone; rmdir refuses a
  # non-empty dir, so a sibling tool's leftovers keep it alive.
  rmdir "$legacy_root" 2>/dev/null || true
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
