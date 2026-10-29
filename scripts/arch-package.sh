#!/usr/bin/env bash
# Builds the Arch package of the desktop client and the database of a pacman repository.
#
# Usage: [CHECK=0] [SIGN=<GPG key>] scripts/arch-package.sh [--install]
#   --install   installs the package on this machine (pacman -U, asks for sudo)
#   CHECK=0     skips `cargo test` (`makepkg` runs it by default)
#   SIGN=ID     signs the package and the database with the given GPG key
#
# Output in dist/arch/:
#   psytify-<version>-<rel>-x86_64.pkg.tar.zst    the package
#   psytify.db(.tar.gz) psytify.files             the repository database (repo-add)
# To publish the repository, upload the WHOLE dist/arch/ folder to an HTTPS server and add
# to the clients' /etc/pacman.conf:
#   [psytify]
#   SigLevel = Optional TrustAll     # with SIGN=ID: Required DatabaseOptional
#   Server = https://<your-host>/arch/$arch
# (remember that pacman looks for the files in <Server>/psytify.db: if the server has no
# `x86_64` subfolder, use `Server = https://<host>/arch`).
#
# The PKGBUILD lives in client/ but `makepkg` would use `client/src` (the code) as its working folder:
# here BUILDDIR, SRCDEST and PKGDEST point elsewhere.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=$ROOT/dist/arch
WORK=${TMPDIR:-/tmp}/psytify-makepkg
mkdir -p "$OUT" "$WORK"

# The distribution backend lives in `.env.build`, outside the repository (see `.env.build.example`).
if [ -f "$ROOT/.env.build" ]; then
  set -a
  # shellcheck disable=SC1091
  . "$ROOT/.env.build"
  set +a
fi

export BUILDDIR=$WORK SRCDEST=$WORK PKGDEST=$OUT
args=(--force --clean --syncdeps --noconfirm)
[ "${CHECK:-1}" = 0 ] && args+=(--nocheck)
[ -n "${SIGN:-}" ] && args+=(--sign --key "$SIGN")

# `makepkg` refuses to run as root: run it as a normal user.
if [ "$(id -u)" = 0 ]; then
  echo "Run this script as a normal user, not as root." >&2
  exit 1
fi

cd "$ROOT/client"
makepkg "${args[@]}"

pkg=$(ls -t "$OUT"/psytify-*-x86_64.pkg.tar.zst | head -1)
repo_args=(--new --remove)
[ -n "${SIGN:-}" ] && repo_args+=(--sign --key "$SIGN")
(cd "$OUT" && repo-add "${repo_args[@]}" psytify.db.tar.gz "$(basename "$pkg")")

echo
echo "Package:    $pkg"
echo "Repository: $OUT (psytify.db)"

if [ "${1:-}" = "--install" ]; then
  sudo pacman -U --noconfirm "$pkg"
fi
