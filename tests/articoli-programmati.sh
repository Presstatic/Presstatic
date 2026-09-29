BIN="$1"; PORT="$2"; B="http://127.0.0.1:$PORT"
stop() { for p in $(pgrep -x presstatic); do [ "$(readlink /proc/$p/cwd 2>/dev/null)" = /tmp/v25b ] && kill $p; done; sleep 0.5; }
startp() { cd /tmp/v25b && (PRESSTATIC_ADDR=127.0.0.1:$PORT setsid nohup "$BIN" > s.log 2>&1 < /dev/null &); for i in $(seq 1 25); do curl -s -o /dev/null $B/admin/login && break; sleep 0.2; done; }
c() { curl -s -o /dev/null -w "%{redirect_url}" "$@"; }
stop; rm -rf /tmp/v25b && mkdir /tmp/v25b && cd /tmp/v25b && printf 'Admin\nadmin@x.it\npassword-lunga-123\n' | "$BIN" adduser >/dev/null; startp
c -c a.jar -d 'email=admin@x.it&password=password-lunga-123' $B/admin/login >/dev/null
future=$(date -d '+3 hour' +%Y-%m-%dT%H:%M)
id=$(c -b a.jar --form-string title="Programmato" --form-string slug=programmato --form-string 'body=<p>x</p>' --form-string status=published --form-string "published_at=$future" $B/admin/edit/0 | grep -o 'edit/[0-9]*' | cut -d/ -f2)
stop
# cancello la pagina generata e sposto l'ora nel passato: simulo "pubblicazione dovuta mentre era spento"
rm -rf public/programmato
python3 -c "import sqlite3,time;con=sqlite3.connect('/tmp/v25b/presstatic.db');con.execute('update posts set published_at=? where id=$id',(int(time.time())-60,));con.commit()"
echo "prima del riavvio: pagina presente? $([ -f public/programmato/index.html ] && echo SI || echo no)"
startp; sleep 1.5
echo "dopo il riavvio: pagina pubblicata? $([ -f public/programmato/index.html ] && echo SI || echo no)"
stop
