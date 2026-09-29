#!/usr/bin/env bash
# Pubblica una nuova versione di Presstatic su GitHub: compila, firma, crea tag e release.
#
#   deploy/release.sh 1.0.1 "Cosa cambia in questa versione"
#
# Serve: Rust, la CLI di GitHub (gh, già autenticata con "gh auth login"), la chiave privata creata con
# "presstatic keygen" (predefinita: ~/.presstatic-signing.key, oppure la variabile PRESSTATIC_SIGNING_KEY),
# il file PUBLIC_KEY con la chiave pubblica corrispondente e il file REPO con "utente/repository".
set -euo pipefail
cd "$(dirname "$0")/.."
V="${1:-}"; NOTES="${2:-Aggiornamento}"
[[ "$V" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Uso: deploy/release.sh 1.2.3 \"note di rilascio\""; exit 1; }
KEY="${PRESSTATIC_SIGNING_KEY:-$HOME/.presstatic-signing.key}"
[ -f "$KEY" ] || { echo "Chiave privata non trovata in $KEY (creala con: presstatic keygen $KEY)"; exit 1; }
[ -s PUBLIC_KEY ] || { echo "Il file PUBLIC_KEY è vuoto: incolla la chiave pubblica stampata da keygen"; exit 1; }
grep -q UTENTE REPO && { echo "Il file REPO contiene ancora il segnaposto: scrivi utente/repository"; exit 1; }
command -v gh >/dev/null || { echo "Installa la CLI di GitHub: https://cli.github.com"; exit 1; }
REPO="$(tr -d '[:space:]' < REPO)"

sed -i "s/^version = \".*\"/version = \"$V\"/" Cargo.toml
cargo build --release
BUILT="$(target/release/presstatic --version | awk '{print $2}')"
[ "$BUILT" = "$V" ] || { echo "Il binario compilato dichiara la versione $BUILT, non $V: controlla Cargo.toml"; exit 1; }
OUT="$(mktemp -d)"
cp target/release/presstatic "$OUT/presstatic-linux-x86_64"
# sign firma "versione\nsha256(file)" leggendo la versione dal binario stesso: la firma è legata alla versione.
target/release/presstatic sign "$OUT/presstatic-linux-x86_64" "$KEY"
git add Cargo.toml Cargo.lock
git commit -qm "v$V" || true
git tag -a "v$V" -m "v$V"
git push && git push --tags
gh release create "v$V" "$OUT/presstatic-linux-x86_64" "$OUT/presstatic-linux-x86_64.sig" --repo "$REPO" --title "v$V" --notes "$NOTES"
echo
echo "Release v$V pubblicata. Entro 12 ore tutti i siti mostreranno «Aggiornamento disponibile» nel pannello."
