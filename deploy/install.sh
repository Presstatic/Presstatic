#!/usr/bin/env bash
# Presstatic: installazione o aggiornamento su un server Ubuntu o Debian, in un solo comando:
#
#   curl -fsSL https://raw.githubusercontent.com/Presstatic/Presstatic/main/deploy/install.sh | sudo bash -s -- miosito.it tua@email.it
#
# oppure, con i file già sul server (presstatic e install.sh nella stessa cartella):  sudo bash install.sh miosito.it tua@email.it
#
# Prima di avviarlo, nel DNS del dominio i record A di miosito.it, www.miosito.it e admin.miosito.it devono puntare
# all'IP del server. Alla fine lo script stampa l'indirizzo dell'installazione guidata. Si può rilanciare quando si vuole:
# aggiorna il programma e lascia intatti articoli, utenti e impostazioni.
set -euo pipefail

# ---- da personalizzare una volta sola, prima di pubblicare (vedi README, "Pubblicare Presstatic") ----
REPO="${PRESSTATIC_REPO:-Presstatic/Presstatic}"   # repository GitHub con le release
# Sicurezza dell'origine (V6): dopo l'installazione, proteggi il server dietro Cloudflare —
# accetta traffico su www solo dagli IP di Cloudflare (Authenticated Origin Pulls o firewall) e
# tieni il pannello (admin) accessibile solo a te (Cloudflare Access / Tunnel / IP whitelisting).
PUBKEY_PEM='-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEAFHvBTkqsfmHG+DInqCJ7/kgfNWiIrZdEgZK+bM1sYHc=
-----END PUBLIC KEY-----'
# -------------------------------------------------------------------------------------------------------

DOMAIN="${1:-}"; EMAIL="${2:-}"
# Cartella dello script SOLO se è un file vero (sudo bash install.sh dallo zip scompattato). Con «curl … | sudo bash»
# non c'è nessun file: non si guarda la cartella corrente, dove chiunque potrebbe aver lasciato un falso «presstatic»
# che verrebbe installato ed eseguito da root. In quel caso si scarica sempre il programma e se ne verifica la firma.
HERE=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"; fi
step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
fail() { printf '\n\033[31mErrore: %s\033[0m\n' "$*"; exit 1; }
ask() { local v; read -rp "$1" v < /dev/tty 2>/dev/null || read -rp "$1" v; echo "$v"; }

[ "$(id -u)" = 0 ] || fail "avvialo come amministratore (sudo)"
[ -n "$DOMAIN" ] || DOMAIN="$(ask 'Dominio del sito (per esempio miosito.it): ')"
[ -n "$EMAIL" ] || EMAIL="$(ask 'Email per il certificato HTTPS (avvisi di scadenza): ')"
DOMAIN="${DOMAIN#http://}"; DOMAIN="${DOMAIN#https://}"; DOMAIN="${DOMAIN#www.}"; DOMAIN="${DOMAIN%%/*}"

# Più siti sulla stessa VPS. Il primo sito usa i nomi di sempre (cartella /var/www/presstatic, servizio e utente
# «presstatic», porta 8080), così le installazioni esistenti si aggiornano come prima. Ogni altro dominio ha nomi propri
# (per esempio presstatic-miosito-it) e la prima porta libera dalla 8081. Il sito si riconosce dal suo «admin.dominio».
SITE=""
for f in /etc/nginx/sites-available/presstatic /etc/nginx/sites-available/presstatic-*; do
  [ -f "$f" ] || continue
  if grep -qE "server_name[^;]*[[:space:]]admin\.${DOMAIN//./\\.}[[:space:];]" "$f"; then SITE="$(basename "$f")"; break; fi
done
if [ -z "$SITE" ]; then
  if [ -f /etc/nginx/sites-available/presstatic ] || [ -f /etc/systemd/system/presstatic.service ]; then
    SITE="presstatic-$(printf '%s' "$DOMAIN" | tr 'A-Z' 'a-z' | tr -c 'a-z0-9' '-' | sed 's/-\+/-/g; s/-$//' | cut -c1-21)"
  else
    SITE=presstatic
  fi
fi
DIR="/var/www/$SITE"
BIN_DIR="$DIR/bin"
if [ -f "/etc/systemd/system/$SITE.service" ]; then
  PORT="$(grep -oE 'PRESSTATIC_ADDR=127\.0\.0\.1:[0-9]+' "/etc/systemd/system/$SITE.service" | grep -oE '[0-9]+$' | head -1)"
fi
if [ -z "${PORT:-}" ]; then
  if [ "$SITE" = presstatic ]; then PORT=8080
  else
    PORT=8081
    while grep -qs "PRESSTATIC_ADDR=127.0.0.1:$PORT\b" /etc/systemd/system/presstatic*.service || { command -v ss >/dev/null && ss -ltn | grep -q ":$PORT "; }; do PORT=$((PORT + 1)); done
  fi
fi
[[ "$DOMAIN" =~ ^[a-z0-9.-]+\.[a-z]{2,}$ ]] || fail "«$DOMAIN» non sembra un dominio valido"
[[ "$EMAIL" =~ ^[^[:space:]@]+@[^[:space:]@]+$ ]] || fail "«$EMAIL» non sembra un'email valida"

step "1/5 Programma"
# Processore del server: x86_64 (Intel, AMD) o aarch64 (ARM64: Ampere, Graviton, Hetzner CAX, Raspberry Pi 64 bit…).
case "$(uname -m)" in x86_64|amd64) ARCH=x86_64 ;; aarch64|arm64) ARCH=aarch64 ;; *) ARCH="$(uname -m)" ;; esac
echo "Processore del server: $ARCH"
# Architettura di un programma, letta dalla sua intestazione ELF (62 = x86_64, 183 = aarch64): il nome del file non basta.
elf_arch() { case "$(od -An -tu2 -j18 -N2 "$1" 2>/dev/null | tr -d ' ')" in 62) echo x86_64 ;; 183) echo aarch64 ;; *) echo altro ;; esac; }
BIN=""
if [ -n "$HERE" ]; then
  for f in "$HERE/presstatic-linux-$ARCH" "$HERE/presstatic" "$HERE/presstatic-linux-x86_64" "$HERE/presstatic-linux-aarch64" "$HERE/../target/release/presstatic"; do
    [ -f "$f" ] || continue
    if [ "$(elf_arch "$f")" = "$ARCH" ]; then BIN="$f"; break; fi
    echo "Salto $f: è per un altro processore ($(elf_arch "$f"))."
  done
  [ -n "$BIN" ] && echo "Uso il programma che sta accanto allo script ($BIN): installalo così solo se viene da una fonte di cui ti fidi."
fi
# Ultima risorsa: nessun programma pronto per questo processore, ma c'è il codice sorgente (accanto allo script o
# scaricato e verificato con la firma): si compila qui, una volta sola. Servono qualche minuto e circa 2 GB di memoria.
build_here() {
  local src="$1"
  echo "Compilo Presstatic per $ARCH dal codice sorgente: ci vogliono da 5 a 20 minuti, secondo il server."
  apt-get install -y -q build-essential pkg-config curl ca-certificates >/dev/null
  if [ "$(awk '/MemTotal/ {print int($2/1024)}' /proc/meminfo)" -lt 1900 ] && ! swapon --show | grep -q presstatic-build; then
    echo "Poca memoria: aggiungo 2 GB di swap temporaneo per la compilazione."
    fallocate -l 2G /var/tmp/presstatic-build.swap 2>/dev/null || dd if=/dev/zero of=/var/tmp/presstatic-build.swap bs=1M count=2048 status=none
    chmod 600 /var/tmp/presstatic-build.swap && mkswap -q /var/tmp/presstatic-build.swap && swapon /var/tmp/presstatic-build.swap
  fi
  if ! command -v cargo >/dev/null && [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null || fail "installazione di Rust non riuscita"
  fi
  export PATH="$HOME/.cargo/bin:$PATH"
  (cd "$src" && cargo build --release --locked) || fail "compilazione non riuscita"
  swapoff /var/tmp/presstatic-build.swap 2>/dev/null && rm -f /var/tmp/presstatic-build.swap
  BIN="$src/target/release/presstatic"
}
if [ -z "$BIN" ] && [ -n "$HERE" ]; then
  for d in "$HERE" "$HERE/.."; do [ -f "$d/Cargo.toml" ] && grep -q 'name = "presstatic"' "$d/Cargo.toml" && { build_here "$(cd "$d" && pwd)"; break; }; done
fi
if [ -z "$BIN" ]; then
  [[ "$REPO" == *UTENTE* ]] && fail "nessun repository impostato in REPO da cui scaricare il programma. (Un programma presstatic accanto allo script si usa solo lanciando install.sh come file, non con curl | bash.)"
  [ -n "$PUBKEY_PEM" ] || fail "PUBKEY_PEM non impostata nello script: non scarico un programma che non posso verificare. Imposta la chiave pubblica (vedi README, «Pubblicare Presstatic») oppure metti presstatic e install.sh sul server e rilancia."
  TMP="$(mktemp -d)"
  trap 'rm -rf "$TMP"' EXIT
  # La versione si ricava dalla release (l'indirizzo /releases/latest porta a /releases/tag/vX.Y.Z),
  # MAI eseguendo il programma scaricato: finché la firma non è verificata, il file non viene avviato.
  TAG="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" | sed -n 's#.*/tag/v\{0,1\}##p')"
  [[ "$TAG" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "non trovo l'ultima versione pubblicata su github.com/$REPO"
  echo "Scarico Presstatic $TAG da github.com/$REPO"
  # Il programma per questo processore; se la release non ce l'ha, il codice sorgente firmato, da compilare qui.
  if curl -fsSL -o "$TMP/presstatic" "https://github.com/$REPO/releases/download/v$TAG/presstatic-linux-$ARCH" 2>/dev/null; then
    curl -fsSL -o "$TMP/presstatic.sig" "https://github.com/$REPO/releases/download/v$TAG/presstatic-linux-$ARCH.sig" || fail "firma non trovata nella release"
    FROM_SRC=""
  else
    echo "La release non ha un programma pronto per $ARCH: scarico il codice sorgente firmato."
    curl -fsSL -o "$TMP/presstatic" "https://github.com/$REPO/releases/download/v$TAG/presstatic-src.tar.gz" || fail "né programma per $ARCH né codice sorgente nella release"
    curl -fsSL -o "$TMP/presstatic.sig" "https://github.com/$REPO/releases/download/v$TAG/presstatic-src.tar.gz.sig" || fail "firma del codice sorgente non trovata"
    FROM_SRC=1
  fi
  printf '%s\n' "$PUBKEY_PEM" > "$TMP/pub.pem"
  # Impronta della chiave: la stessa che mostra la pagina Aggiornamenti del pannello e «presstatic impronta».
  # Va confrontata con quella pubblicata sul sito di Presstatic (fuori da GitHub, da cui arriva questo script).
  FP="$(openssl pkey -pubin -in "$TMP/pub.pem" -outform DER 2>/dev/null | tail -c 32 | sha256sum | awk '{print $1}' | sed 's/.\{4\}/& /g; s/ $//')"
  [ -n "$FP" ] || fail "chiave pubblica PUBKEY_PEM non valida"
  echo "Impronta della chiave di firma: $FP"
  if [ -n "${PRESSTATIC_IMPRONTA:-}" ]; then
    [ "$(printf '%s' "$PRESSTATIC_IMPRONTA" | tr -d ' ')" = "$(printf '%s' "$FP" | tr -d ' ')" ] || fail "l'impronta della chiave NON coincide con quella attesa: installazione interrotta, non installare questo programma"
    echo "Coincide con quella attesa."
  else
    echo "Confrontala con quella pubblicata sul sito di Presstatic: se è diversa, interrompi ora con Ctrl+C (continuo tra 8 secondi)."
    sleep 8
  fi
  # La firma copre "versione\nsha256(programma)": vale solo per questo file e per questa versione.
  printf '%s\n%s' "$TAG" "$(sha256sum "$TMP/presstatic" | awk '{print $1}')" > "$TMP/msg"
  openssl pkeyutl -verify -pubin -inkey "$TMP/pub.pem" -rawin -in "$TMP/msg" -sigfile "$TMP/presstatic.sig" >/dev/null 2>&1 \
    || fail "la firma del programma scaricato non è valida: non lo installo"
  if [ -n "${FROM_SRC:-}" ]; then mkdir -p "$TMP/src" && tar -xzf "$TMP/presstatic" -C "$TMP/src" --strip-components=1 && build_here "$TMP/src"; else BIN="$TMP/presstatic"; fi
  echo "Firma verificata (versione $TAG)."
  BIN="$TMP/presstatic"
fi
chmod +x "$BIN"
"$BIN" --version >/dev/null 2>&1 || fail "il programma non parte su questo server (serve Linux x86_64 con glibc 2.34 o successiva: Ubuntu 22.04+, Debian 12+)"

step "2/5 Controllo del DNS"
# Aggiornamento di un sito che ha già il certificato: il DNS non serve (con Cloudflare acceso punterebbe comunque altrove).
if [ -f "/etc/letsencrypt/live/$DOMAIN/fullchain.pem" ]; then
  echo "Il certificato HTTPS di $DOMAIN c'è già: è un aggiornamento, controllo del DNS non necessario."
else
  IP="$(curl -fsS --max-time 5 https://api.ipify.org 2>/dev/null || true)"
  MISSING=""
  for h in "$DOMAIN" "www.$DOMAIN" "admin.$DOMAIN"; do
    R="$(getent ahostsv4 "$h" | awk '{print $1; exit}' || true)"
    if [ -z "$R" ]; then MISSING="$MISSING $h (non trovato)"; elif [ -n "$IP" ] && [ "$R" != "$IP" ]; then MISSING="$MISSING $h ($R)"; fi
  done
  if [ -n "$MISSING" ]; then
    echo "Questi nomi non puntano ancora a questo server${IP:+ ($IP)}:$MISSING"
    echo "Se usi Cloudflare, durante l'installazione metti la nuvola grigia (solo DNS) sui tre record."
    OK="$(ask 'Continuare comunque? Il certificato HTTPS potrebbe non essere rilasciato. [s/N] ')"
    [[ "${OK:-n}" =~ ^[sSyY] ]] || exit 1
  fi
fi

step "3/5 Nginx, Certbot, utente e servizio"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq nginx certbot python3-certbot-nginx openssl >/dev/null
# brotli_static: Nginx invia le pagine già compresse in brotli (Ubuntu 22.04+ lo offre; se manca, si usa solo gzip)
apt-get install -y -qq libnginx-mod-http-brotli-static >/dev/null 2>&1 || true
id -u "$SITE" >/dev/null 2>&1 || useradd --system --home-dir "$DIR" --shell /usr/sbin/nologin "$SITE"
mkdir -p "$DIR/public" "$BIN_DIR"
# Il programma sta nella cartella del sito, di proprietà del servizio: così il pannello può aggiornarlo da solo.
[ -f "$BIN_DIR/presstatic" ] && mv -f "$BIN_DIR/presstatic" "$BIN_DIR/presstatic.old"
install -m 755 "$BIN" "$BIN_DIR/presstatic"
# Lo script che fa accettare al sito solo il traffico di Cloudflare: si lancia dopo aver acceso la nuvola arancione.
if [ -n "$HERE" ] && [ -f "$HERE/solo-cloudflare.sh" ]; then
  install -m 755 "$HERE/solo-cloudflare.sh" "$DIR/solo-cloudflare.sh"
elif [[ "$REPO" != *UTENTE* ]]; then
  curl -fsSL -o "$DIR/solo-cloudflare.sh" "https://raw.githubusercontent.com/$REPO/main/deploy/solo-cloudflare.sh" && chmod 755 "$DIR/solo-cloudflare.sh" \
    || echo "(solo-cloudflare.sh non scaricato: lo trovi nel repository, cartella deploy)"
fi
# Il vecchio programma unico (/usr/local/bin/presstatic, installazioni fatte con la prima versione dello script) si toglie
# solo se nessun servizio lo usa più: su una VPS con più siti, un sito vecchio può ancora avviarsi da lì.
if [ -e /usr/local/bin/presstatic ] && ! grep -qs "ExecStart=/usr/local/bin/presstatic" /etc/systemd/system/*.service; then rm -f /usr/local/bin/presstatic; fi
chown -R "$SITE": "$DIR"
cat > "/etc/systemd/system/$SITE.service" << EOF
[Unit]
Description=Presstatic
After=network.target

[Service]
User=$SITE
Group=$SITE
WorkingDirectory=$DIR
Environment=PRESSTATIC_ADDR=127.0.0.1:$PORT
ExecStart=$BIN_DIR/presstatic
Restart=always
RestartSec=2
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=$DIR
UMask=0022

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable "$SITE" >/dev/null 2>&1
systemctl reset-failed "$SITE" 2>/dev/null || true   # un servizio fermo dopo troppi tentativi falliti (per esempio senza programma) si riavvia lo stesso
systemctl restart "$SITE"

step "4/5 Sito e pannello su Nginx, con HTTPS"
# Sito pubblico regolato per servire più richieste al secondo: Presstatic salva accanto a ogni pagina la versione
# gzip e brotli, e Nginx le invia così come sono invece di comprimere a ogni richiesta.
mkdir -p /etc/nginx/snippets
{
  echo "# Creato da install.sh di Presstatic: sito pubblico regolato per servire più richieste al secondo."
  echo "# Pagine già compresse: Nginx invia index.html.br o index.html.gz invece di comprimere a ogni visita."
  echo "gzip_static on;"
  echo "gzip_vary on;"
  if ls /etc/nginx/modules-enabled/ 2>/dev/null | grep -q brotli-static; then echo "brotli_static on;"; fi
  echo "# Registro delle visite spento: dietro Cloudflare vede solo le pagine non ancora in cache, e nei picchi riempirebbe il disco."
  echo "access_log off;"
  echo "keepalive_requests 100000;"
  echo "open_file_cache_valid 60s;"
  echo "open_file_cache_min_uses 1;"
  echo "open_file_cache_errors on;"
  echo "server_tokens off;"
} > /etc/nginx/snippets/presstatic-statico.conf
# Più connessioni contemporanee e più file aperti (con i valori di Ubuntu Nginx rifiuta le connessioni oltre circa 1.500)
sed -i 's/^\(\s*\)worker_connections [0-9]*;/\1worker_connections 8192;/; s/^\(\s*\)# *multi_accept on;/\1multi_accept on;/' /etc/nginx/nginx.conf
grep -q "^worker_rlimit_nofile" /etc/nginx/nginx.conf || sed -i 's/^worker_processes \(.*\);/worker_processes \1;\nworker_rlimit_nofile 65535;/' /etc/nginx/nginx.conf
# Code più lunghe per le nuove connessioni nei picchi di traffico
cat > /etc/sysctl.d/99-presstatic.conf << SYS
net.core.somaxconn = 65535
net.ipv4.tcp_max_syn_backlog = 65535
fs.file-max = 1048576
SYS
sysctl -q --system >/dev/null 2>&1 || true
if [ -f /etc/nginx/sites-available/$SITE ]; then
  # siti installati prima dei moduli: anche /modulo/ va al programma, come /commenti/
  sed -i 's#\^/(commenti|newsletter|push)/#^/(commenti|newsletter|push|modulo)/#' "/etc/nginx/sites-available/$SITE"
  echo "Configurazione di Nginx già presente: la lascio com'è (eventuali modifiche fatte a mano restano)."
  sed -i 's#try_files \$uri \$uri/ =404; add_header Cache-Control "public, max-age=60"#try_files \$uri/index.html \$uri \$uri/ =404; add_header Cache-Control "public, max-age=60"#; s#=404; sendfile off; add_header#=404; add_header#' /etc/nginx/sites-available/$SITE
  if ! grep -q "presstatic-statico.conf" /etc/nginx/sites-available/$SITE; then
    sed -i "0,/^\(\s*\)server_name www\.\(.*\);/s##&\n\1include /etc/nginx/snippets/presstatic-statico.conf;#" /etc/nginx/sites-available/$SITE
    nginx -t -q 2>/dev/null && echo "Aggiunte alla configurazione esistente: pagine già compresse e regolazioni per il carico." \
      || { sed -i '\#include /etc/nginx/snippets/presstatic-statico.conf;#d' /etc/nginx/sites-available/$SITE; echo "Non ho potuto aggiungere le regolazioni alla configurazione esistente: la lascio com'era."; }
  fi
else
# Limiti di frequenza (contesto http): accesso/recupero e moduli pubblici, per indirizzo reale del visitatore.
cat > /etc/nginx/conf.d/presstatic-limiti.conf <<'EOF'
limit_req_zone $binary_remote_addr zone=presstatic_accesso:10m rate=10r/m;
limit_req_zone $binary_remote_addr zone=presstatic_moduli:10m rate=30r/m;
limit_req_status 429;
EOF
cat > /etc/nginx/sites-available/$SITE << EOF
# Creato da install.sh di Presstatic. Il sito pubblico sono file statici; il pannello è su admin.$DOMAIN.
server {
    listen 80 backlog=4096;
    server_name www.$DOMAIN;
    include /etc/nginx/snippets/presstatic-statico.conf;
    set_real_ip_from 173.245.48.0/20; set_real_ip_from 103.21.244.0/22; set_real_ip_from 103.22.200.0/22; set_real_ip_from 103.31.4.0/22;
    set_real_ip_from 141.101.64.0/18; set_real_ip_from 108.162.192.0/18; set_real_ip_from 190.93.240.0/20; set_real_ip_from 188.114.96.0/20;
    set_real_ip_from 197.234.240.0/22; set_real_ip_from 198.41.128.0/17; set_real_ip_from 162.158.0.0/15; set_real_ip_from 104.16.0.0/13;
    set_real_ip_from 104.24.0.0/14; set_real_ip_from 172.64.0.0/13; set_real_ip_from 131.0.72.0/22;
    set_real_ip_from 2400:cb00::/32; set_real_ip_from 2606:4700::/32; set_real_ip_from 2803:f800::/32; set_real_ip_from 2405:b500::/32;
    set_real_ip_from 2405:8100::/32; set_real_ip_from 2a06:98c0::/29; set_real_ip_from 2c0f:f248::/32;
    real_ip_header CF-Connecting-IP;
    root $DIR/public;
    index index.html;
    absolute_redirect off;
    sendfile on;
    tcp_nopush on;
    open_file_cache max=20000 inactive=60s;
    gzip on;
    gzip_types application/xml application/rss+xml text/plain text/css text/javascript application/json;
    error_page 404 /404.html;
    add_header X-Content-Type-Options nosniff always;
    location / { try_files \$uri/index.html \$uri \$uri/ =404; add_header Cache-Control "public, max-age=60" always; add_header X-Content-Type-Options nosniff always; }
    location /media/ { add_header Cache-Control "public, max-age=31536000, immutable" always; add_header X-Content-Type-Options nosniff always; }
    location /assets/ { add_header Cache-Control "public, max-age=31536000, immutable" always; }
    location ~ \.(xml|txt)\$ { add_header Cache-Control "public, max-age=300" always; }
    # Commenti, newsletter e notifiche partono dal sito e arrivano al programma da qui: il pannello (admin.) può restare
    # chiuso alla redazione e dietro Cloudflare, senza mostrare l'IP del server.
    location ~ ^/(commenti|newsletter|push|modulo)/ {
        limit_req zone=presstatic_moduli burst=20 nodelay;
        proxy_pass http://127.0.0.1:$PORT;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-Proto \$scheme;
        add_header Cache-Control "no-store" always;
    }
    location ^~ /admin { return 404; }
}
server {
    listen 80;
    server_name $DOMAIN;
    return 301 https://www.$DOMAIN\$request_uri;
}
server {
    listen 80;
    server_name admin.$DOMAIN;
    client_max_body_size 25m;
    add_header X-Frame-Options SAMEORIGIN always;
    add_header X-Content-Type-Options nosniff always;
    add_header Referrer-Policy same-origin always;
    add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;
    # Dietro Cloudflare: l'indirizzo reale del visitatore, accettato solo dagli indirizzi di Cloudflare.
    set_real_ip_from 173.245.48.0/20; set_real_ip_from 103.21.244.0/22; set_real_ip_from 103.22.200.0/22; set_real_ip_from 103.31.4.0/22;
    set_real_ip_from 141.101.64.0/18; set_real_ip_from 108.162.192.0/18; set_real_ip_from 190.93.240.0/20; set_real_ip_from 188.114.96.0/20;
    set_real_ip_from 197.234.240.0/22; set_real_ip_from 198.41.128.0/17; set_real_ip_from 162.158.0.0/15; set_real_ip_from 104.16.0.0/13;
    set_real_ip_from 104.24.0.0/14; set_real_ip_from 172.64.0.0/13; set_real_ip_from 131.0.72.0/22;
    set_real_ip_from 2400:cb00::/32; set_real_ip_from 2606:4700::/32; set_real_ip_from 2803:f800::/32; set_real_ip_from 2405:b500::/32;
    set_real_ip_from 2405:8100::/32; set_real_ip_from 2a06:98c0::/29; set_real_ip_from 2c0f:f248::/32;
    real_ip_header CF-Connecting-IP;
    # Accesso e recupero password: pochi tentativi al minuto per indirizzo, prima ancora di arrivare al programma.
    location ~ ^/admin/(login|recupero) {
        limit_req zone=presstatic_accesso burst=10 nodelay;
        proxy_pass http://127.0.0.1:$PORT;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-Proto \$scheme;
    }
    location / {
        proxy_pass http://127.0.0.1:$PORT;
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-Proto \$scheme;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        proxy_read_timeout 300s;
    }
}
EOF
fi
ln -sf "/etc/nginx/sites-available/$SITE" "/etc/nginx/sites-enabled/$SITE"
rm -f /etc/nginx/sites-enabled/default
nginx -t -q
systemctl reload nginx
certbot --nginx -n --agree-tos -m "$EMAIL" --redirect -d "$DOMAIN" -d "www.$DOMAIN" -d "admin.$DOMAIN" \
  || fail "certificato HTTPS non rilasciato. Il pannello non va usato senza HTTPS: controlla che i tre record DNS puntino a questo server (con Cloudflare, nuvola grigia) e rilancia lo script."

step "5/5 Fatto"
# Non eseguiamo mai da root il binario nella cartella del servizio (scrivibile dall'utente presstatic).
for _ in $(seq 1 20); do [ -f "$DIR/setup-token.txt" ] && break; sleep 0.5; done
if [ -f "$DIR/setup-token.txt" ]; then
  echo "Apri questo indirizzo e segui l'installazione guidata:"
  echo
  echo "    https://admin.$DOMAIN/admin/setup?token=$(cat "$DIR/setup-token.txt")"
else
  echo "Presstatic installato e già configurato. Pannello: https://admin.$DOMAIN/admin"
fi
echo
echo "Sito: https://www.$DOMAIN"
if [ -f "$DIR/solo-cloudflare.sh" ]; then
  echo
  echo "Usi Cloudflare? Dopo l'installazione guidata: nuvola arancione su www e admin, SSL/TLS «Full (strict)», poi:"
  echo "    sudo bash $DIR/solo-cloudflare.sh"
fi
echo "Riga di comando: sudo -u $SITE $BIN_DIR/presstatic adduser"
