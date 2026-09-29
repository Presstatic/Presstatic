BIN="$1"; PORT="$2"; B="http://127.0.0.1:$PORT"
for p in $(pgrep -x presstatic); do [ "$(readlink /proc/$p/cwd 2>/dev/null)" = /tmp/v27 ] && kill $p; done; sleep 0.5
rm -rf /tmp/v27 && mkdir /tmp/v27 && cd /tmp/v27 && (PRESSTATIC_ADDR=127.0.0.1:$PORT setsid nohup "$BIN" > s.log 2>&1 < /dev/null &); for i in $(seq 1 25); do curl -s -o /dev/null $B/admin/login && break; sleep 0.2; done
T=$(cat setup-token.txt)
for i in 1 2 3 4 5; do curl -s -o /dev/null --form-string token=$T --form-string site_name=Prova --form-string base_url=http://www.prova.it --form-string "name=Admin $i" --form-string email=admin$i@x.it --form-string password=password-lunga-123 --form-string theme=classico --form-string lang=it --form-string timezone=Europe/Rome $B/admin/setup & done; wait
echo "amministratori creati da 5 invii simultanei: $(python3 -c "import sqlite3;print(sqlite3.connect('/tmp/v27/presstatic.db').execute('select count(*) from users').fetchone()[0])")"
