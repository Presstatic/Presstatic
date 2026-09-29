BIN=/home/claude/presstatic/target/release/presstatic; B=http://127.0.0.1:8156
stop() { for p in $(pgrep -x presstatic); do [ "$(readlink /proc/$p/cwd 2>/dev/null)" = /tmp/up2 ] && kill $p; done; for i in $(seq 1 20); do curl -s -o /dev/null $B/admin/login || break; sleep 0.2; done; }
startp() { cd /tmp/up2 && (PRESSTATIC_ADDR=127.0.0.1:8156 setsid nohup $BIN > s.log 2>&1 < /dev/null &); for i in $(seq 1 20); do curl -s -o /dev/null $B/admin/login && break; sleep 0.2; done; }
c() { curl -s -o /dev/null -w "%{redirect_url}" "$@"; }
stop; rm -rf /tmp/up2 && mkdir /tmp/up2 && cd /tmp/up2 && printf 'Admin\nadmin@x.it\npassword-lunga-123\n' | $BIN adduser >/dev/null; startp
c -c a.jar -d 'email=admin@x.it&password=password-lunga-123' $B/admin/login >/dev/null
id=$(c -b a.jar --form-string title="Articolo vecchio" --form-string slug=articolo-vecchio --form-string 'body=<p>x</p>' --form-string status=published $B/admin/edit/0 | grep -o 'edit/[0-9]*' | cut -d/ -f2)
echo "articolo pubblicato (id $id): $(ls public/articolo-vecchio)"
rm -f public/articolo-vecchio/.presstatic-page; stop; startp
echo "dopo il riavvio la cartella è segnata come del CMS: $(ls -A public/articolo-vecchio | grep -c presstatic-page)"
c -c a.jar -d 'email=admin@x.it&password=password-lunga-123' $B/admin/login >/dev/null
c -b a.jar --form-string title="Articolo vecchio" --form-string slug=articolo-vecchio --form-string 'body=<p>modificato</p>' --form-string status=published $B/admin/edit/$id >/dev/null
echo "slug dopo una modifica: $(python3 -c "import sqlite3;print(sqlite3.connect('/tmp/up2/presstatic.db').execute('select slug from posts where id=$id').fetchone()[0])")"
c -b a.jar --form-string title="Articolo vecchio" --form-string slug=articolo-vecchio --form-string 'body=<p>x</p>' --form-string status=draft $B/admin/edit/$id >/dev/null
echo "riportato in bozza, pagina ancora online: $([ -f public/articolo-vecchio/index.html ] && echo SI || echo no)"
stop
