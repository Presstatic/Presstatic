# Articoli programmati e riavvii: si pubblica ciò che è scaduto mentre il servizio era fermo, e un riavvio
# normale NON ripubblica l'archivio (niente pagine riscritte, niente nuovi avvisi a Google).
BIN="$1"; PORT="$2"; B="http://127.0.0.1:$PORT"
stop() { for p in $(pgrep -x presstatic); do [ "$(readlink /proc/$p/cwd 2>/dev/null)" = /tmp/v25c ] && kill $p; done; sleep 0.6; }
startp() { cd /tmp/v25c && (PRESSTATIC_ADDR=127.0.0.1:$PORT setsid nohup "$BIN" > s.log 2>&1 < /dev/null &); for i in $(seq 1 25); do curl -s -o /dev/null $B/admin/login && break; sleep 0.2; done; }
c() { curl -s -o /dev/null -w "%{redirect_url}" "$@"; }
stop; rm -rf /tmp/v25c && mkdir /tmp/v25c && cd /tmp/v25c && printf 'Admin\nadmin@x.it\npassword-lunga-123\n' | "$BIN" adduser >/dev/null; startp
c -c a.jar -d 'email=admin@x.it&password=password-lunga-123' $B/admin/login >/dev/null
c -b a.jar --form-string title="Già uscito" --form-string slug=gia-uscito --form-string 'body=<p>x</p>' --form-string status=published $B/admin/edit/0 >/dev/null
future=$(date -d '+3 hour' +%Y-%m-%dT%H:%M)
id=$(c -b a.jar --form-string title="Programmato" --form-string slug=programmato --form-string 'body=<p>x</p>' --form-string status=published --form-string "published_at=$future" $B/admin/edit/0 | grep -o 'edit/[0-9]*' | cut -d/ -f2)
before=$(stat -c %Y public/gia-uscito/index.html)
stop; sleep 2
# l'ora dell'articolo programmato arriva mentre il servizio è fermo
rm -rf public/programmato
python3 -c "import sqlite3,time;con=sqlite3.connect('/tmp/v25c/presstatic.db');con.execute('update posts set published_at=? where id=$id',(int(time.time())-1,));con.commit()"
startp; sleep 1.5
echo "articolo scaduto durante il fermo, pubblicato all'avvio: $([ -f public/programmato/index.html ] && echo SI || echo no)"
echo "articolo già uscito NON ripubblicato al riavvio: $([ "$(stat -c %Y public/gia-uscito/index.html)" = "$before" ] && echo SI || echo no)"
stop
