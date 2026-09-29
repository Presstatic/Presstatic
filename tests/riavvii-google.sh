# Un riavvio normale non deve riavvisare Google per gli articoli già usciti.
BIN="$1"; PORT="$2"; B="http://127.0.0.1:$PORT"
stop() { for p in $(pgrep -x presstatic); do [ "$(readlink /proc/$p/cwd 2>/dev/null)" = /tmp/v25d ] && kill $p; done; sleep 0.6; }
startp() { cd /tmp/v25d && (PRESSTATIC_ADDR=127.0.0.1:$PORT PRESSTATIC_GOOGLE_API=http://127.0.0.1:8197 setsid nohup "$BIN" > s.log 2>&1 < /dev/null &); for i in $(seq 1 25); do curl -s -o /dev/null $B/admin/login && break; sleep 0.2; done; }
c() { curl -s -o /dev/null -w "%{redirect_url}" "$@"; }
stop; rm -rf /tmp/v25d && mkdir /tmp/v25d && cd /tmp/v25d && printf 'Admin\nadmin@x.it\npassword-lunga-123\n' | "$BIN" adduser >/dev/null; startp
c -c a.jar -d 'email=admin@x.it&password=password-lunga-123' $B/admin/login >/dev/null
c -b a.jar --data-urlencode "google_sa@/tmp/mock/sa.json" -d google_on=on $B/admin/indicizzazione >/dev/null
for i in 1 2 3; do c -b a.jar --form-string title="Articolo $i" --form-string "slug=articolo-$i" --form-string 'body=<p>x</p>' --form-string status=published $B/admin/edit/0 >/dev/null; done
sent=$(grep -c "\"google\": \"publish\"" /tmp/mock/services.log 2>/dev/null || echo 0)
stop; sleep 1; : > /tmp/mock/services.log; startp; sleep 2
echo "avvisi a Google alla pubblicazione: $sent; dopo il riavvio: $(grep -c "\"google\": \"publish\"" /tmp/mock/services.log 2>/dev/null || echo 0) (deve essere 0)"
stop
