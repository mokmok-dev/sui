#!/usr/bin/env bash
#
# Cloud Agent install for the `sui` Rust workspace.
#
# The base image already ships the pinned Rust toolchain (rust-toolchain.toml),
# so this script only prepares what is missing:
#   1. single-user Nix (no daemon) so agents can run the exact CI gate,
#      `nix flake check`, and the flake dev shell (.envrc = `use flake`);
#   2. flakes enabled;
#   3. nix on PATH for login and non-login agent shells;
#   4. a warm cargo dependency cache.
#
# Design constraints:
#   * Idempotent: safe to run repeatedly and against a warm snapshot.
#   * Non-gating: it MUST NOT run lints/tests or compile the crates, because a
#     future agent needs a working environment even when the code it checked
#     out is mid-change. The warm Nix store is provided by the snapshot, not by
#     building here.
set -euo pipefail

NIX_PROFILE_SCRIPT="$HOME/.nix-profile/etc/profile.d/nix.sh"

# 1. Install single-user Nix if it is not already present.
if [ ! -e "$NIX_PROFILE_SCRIPT" ]; then
  echo "==> Installing single-user Nix"
  curl -fsSL https://nixos.org/nix/install | sh -s -- --no-daemon --yes
fi

# 2. Enable flakes (required by this repo's flake.nix).
mkdir -p "$HOME/.config/nix"
if ! grep -qs 'experimental-features.*flakes' "$HOME/.config/nix/nix.conf" 2>/dev/null; then
  echo 'experimental-features = nix-command flakes' >>"$HOME/.config/nix/nix.conf"
fi

# 3. Ensure nix is on PATH for both login and non-login agent shells.
SOURCE_LINE='[ -e "$HOME/.nix-profile/etc/profile.d/nix.sh" ] && . "$HOME/.nix-profile/etc/profile.d/nix.sh"'
if ! grep -qsF "$SOURCE_LINE" "$HOME/.bashrc" 2>/dev/null; then
  printf '\n# Load single-user Nix (added by sui Cloud Agent install)\n%s\n' "$SOURCE_LINE" >>"$HOME/.bashrc"
fi

# Make nix usable within the rest of this script.
# shellcheck disable=SC1090
[ -e "$NIX_PROFILE_SCRIPT" ] && . "$NIX_PROFILE_SCRIPT"

# 4. Warm the cargo dependency cache (download only; never compiles).
echo "==> Fetching cargo dependencies"
cargo fetch --locked

echo "==> Install complete."
echo "    $(cargo --version)"
echo "    $(nix --version 2>/dev/null || echo 'nix unavailable')"
