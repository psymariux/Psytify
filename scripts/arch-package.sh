#!/usr/bin/env bash
# Costruisce il pacchetto Arch del client desktop e il database di un repository pacman.
#
# Uso: [CHECK=0] [SIGN=<chiave GPG>] scripts/arch-package.sh [--install]
#   --install   installa il pacchetto sul PC (pacman -U, chiede sudo)
#   CHECK=0     salta `cargo test` (di default `makepkg` lo esegue)
#   SIGN=ID     firma pacchetto e database con la chiave GPG indicata
#
# Esce in dist/arch/:
#   psytify-<versione>-<rel>-x86_64.pkg.tar.zst   il pacchetto
#   psytify.db(.tar.gz) psytify.files             il database del repository (repo-add)
# Per pubblicare il repository, carica TUTTA la cartella dist/arch/ su un server HTTPS e aggiungi
# a /etc/pacman.conf dei client:
#   [psytify]
#   SigLevel = Optional TrustAll     # con SIGN=ID: Required DatabaseOptional
#   Server = https://<il-tuo-host>/arch/$arch
# (ricorda che pacman cerca i file in <Server>/psytify.db: se il server non ha la sottocartella
# `x86_64`, usa `Server = https://<host>/arch`).
#
# Il PKGBUILD sta in client/ ma `makepkg` userebbe `client/src` (il codice) come cartella di lavoro:
# qui BUILDDIR, SRCDEST e PKGDEST puntano altrove.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
OUT=$ROOT/dist/arch
WORK=${TMPDIR:-/tmp}/psytify-makepkg
mkdir -p "$OUT" "$WORK"

# Il backend di distribuzione sta in `.env.build`, fuori dal repository (vedi `.env.build.example`).
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

# `makepkg` rifiuta di girare da root: va lanciato come utente normale.
if [ "$(id -u)" = 0 ]; then
  echo "Lancia lo script come utente normale, non come root." >&2
  exit 1
fi

cd "$ROOT/client"
makepkg "${args[@]}"

pkg=$(ls -t "$OUT"/psytify-*-x86_64.pkg.tar.zst | head -1)
repo_args=(--new --remove)
[ -n "${SIGN:-}" ] && repo_args+=(--sign --key "$SIGN")
(cd "$OUT" && repo-add "${repo_args[@]}" psytify.db.tar.gz "$(basename "$pkg")")

echo
echo "Pacchetto:  $pkg"
echo "Repository: $OUT (psytify.db)"

if [ "${1:-}" = "--install" ]; then
  sudo pacman -U --noconfirm "$pkg"
fi
