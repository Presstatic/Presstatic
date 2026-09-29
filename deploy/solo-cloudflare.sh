#!/usr/bin/env bash
# Presstatic: il sito pubblico (www) accetta connessioni SOLO dalla rete di Cloudflare.
# Così chi scopre l'indirizzo IP del server non può aggirare Cloudflare e colpirlo direttamente.
#
#   sudo bash solo-cloudflare.sh           attiva (o aggiorna l'elenco degli indirizzi di Cloudflare)
#   sudo bash solo-cloudflare.sh --togli   disattiva
#
# Lancialo DOPO aver messo la nuvola arancione sul record www in Cloudflare: con la nuvola grigia il sito
# smetterebbe di rispondere. Il rinnovo del certificato continua a funzionare, perché passa da Cloudflare.
# Il pannello (admin) non viene toccato.
set -euo pipefail
# Il sito è quello della cartella da cui si lancia lo script (l'installazione lo copia in /var/www/<sito>/);
# si può anche indicarlo: sudo bash solo-cloudflare.sh presstatic-miosito-it
SITE="${1:-$(basename "$(dirname "$(readlink -f "$0")")")}"
[ -f "/etc/nginx/sites-available/$SITE" ] || SITE=presstatic
CONF="/etc/nginx/sites-available/$SITE"
SNIP=/etc/nginx/snippets/presstatic-solo-cloudflare.conf
fail() { printf '\n\033[31mErrore: %s\033[0m\n' "$*"; exit 1; }
[ "$(id -u)" = 0 ] || fail "avvialo come amministratore (sudo)"
[ -f "$CONF" ] || fail "configurazione $CONF non trovata: prima installa Presstatic con install.sh"

if [ "${1:-}" = "--togli" ]; then
  sed -i '\#include /etc/nginx/snippets/presstatic-solo-cloudflare.conf;#d' "$CONF"
  rm -f "$SNIP"; nginx -t -q && systemctl reload nginx
  echo "Fatto: il sito pubblico accetta di nuovo connessioni da qualsiasi indirizzo."; exit 0
fi

# Elenco aggiornato dal sito di Cloudflare; se non è raggiungibile, si usa quello incluso qui (settembre 2026).
V4="$(curl -fsS --max-time 10 https://www.cloudflare.com/ips-v4 2>/dev/null || true)"
V6="$(curl -fsS --max-time 10 https://www.cloudflare.com/ips-v6 2>/dev/null || true)"
if ! grep -qE '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+/[0-9]+$' <<< "$V4"; then
  echo "Elenco di Cloudflare non raggiungibile: uso quello incluso nello script."
  V4="173.245.48.0/20 103.21.244.0/22 103.22.200.0/22 103.31.4.0/22 141.101.64.0/18 108.162.192.0/18 190.93.240.0/20 188.114.96.0/20 197.234.240.0/22 198.41.128.0/17 162.158.0.0/15 104.16.0.0/13 104.24.0.0/14 172.64.0.0/13 131.0.72.0/22"
  V6="2400:cb00::/32 2606:4700::/32 2803:f800::/32 2405:b500::/32 2405:8100::/32 2a06:98c0::/29 2c0f:f248::/32"
fi
mkdir -p "$(dirname "$SNIP")"
{
  echo "# Creato da solo-cloudflare.sh: solo la rete di Cloudflare può collegarsi al sito pubblico."
  for r in $V4 $V6; do
    [[ "$r" =~ ^[0-9a-fA-F:.]+/[0-9]+$ ]] && echo "allow $r;"
  done
  echo "allow 127.0.0.1;"
  echo "deny all;"
} > "$SNIP"
# include solo nel blocco del sito pubblico (www), una volta sola
if ! grep -q "presstatic-solo-cloudflare.conf" "$CONF"; then
  sed -i '0,/^\(\s*\)server_name www\.\(.*\);/s##&\n\1include /etc/nginx/snippets/presstatic-solo-cloudflare.conf;#' "$CONF"
fi
nginx -t -q || { sed -i '\#include /etc/nginx/snippets/presstatic-solo-cloudflare.conf;#d' "$CONF"; fail "configurazione non valida: modifica annullata"; }
systemctl reload nginx
echo "Fatto: il sito pubblico ora risponde solo attraverso Cloudflare ($(grep -c '^allow' "$SNIP") reti ammesse)."
echo "Rilancia questo script ogni tanto (per esempio una volta al mese) per aggiornare l'elenco degli indirizzi di Cloudflare."
