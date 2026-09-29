# Più letti da Google Analytics 4: un finto Google (token + Data API) risponde come quello vero.
# Uso: python3 tests/piu-letti.py http://127.0.0.1:8209 /percorso/del/sito   (programma avviato con PRESSTATIC_GOOGLE_API e PRESSTATIC_GA_API = http://127.0.0.1:8310)
import subprocess, sys, re, json, sqlite3, os, time, threading, http.server, base64, urllib.parse
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives import serialization
B, DIR = sys.argv[1], sys.argv[2]
PUB = f"{DIR}/public"
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
def curl(jar, *a): return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def home(): return open(f"{PUB}/index.html", encoding="utf-8").read()
def setting(k, v):
    with db() as c: c.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (k, v))
seen = {"scope": "", "auth": "", "body": {}}
rows = {"data": [("/articolo-b/", 900), ("/", 5000), ("/articolo-a/", 500), ("/category/cronaca/", 400), ("/articolo-c/", 300), ("/non-esiste/", 200), ("/articolo-d/", 100)]}
class G(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        if self.path == "/token":
            jwt = urllib.parse.parse_qs(body.decode())["assertion"][0]
            seen["scope"] = json.loads(base64.urlsafe_b64decode(jwt.split(".")[1] + "=="))["scope"]
            out, code = {"access_token": "tok-ga", "expires_in": 3600}, 200
        elif self.path.startswith("/v1beta/properties/123456789:runReport"):
            seen["auth"], seen["body"] = self.headers.get("Authorization"), json.loads(body)
            out, code = {"rows": [{"dimensionValues": [{"value": p}], "metricValues": [{"value": str(v)}]} for p, v in rows["data"]]}, 200
        else:
            out, code = {"error": {"message": "The caller does not have permission"}}, 403
        self.send_response(code); self.send_header("Content-Type", "application/json"); self.end_headers(); self.wfile.write(json.dumps(out).encode())
    def log_message(self, *a): pass
threading.Thread(target=http.server.HTTPServer(("127.0.0.1", 8310), G).serve_forever, daemon=True).start()
key = rsa.generate_private_key(public_exponent=65537, key_size=2048).private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()).decode()
sa = {"type": "service_account", "client_email": "presstatic@progetto.iam.gserviceaccount.com", "private_key": key, "token_uri": "http://127.0.0.1:8310/token"}
A = "/tmp/pl-a.jar"; open(A, "w").close()
curl(A, "-o", "/dev/null", "--data-urlencode", "email=andrea@example.com", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
for x in "abcd":
    curl(A, "-o", "/dev/null", "--form-string", f"title=Articolo {x.upper()}", "--form-string", f"slug=articolo-{x}", "--form-string", "body=<p>Testo.</p>", "--form-string", "status=published", B + "/admin/edit/0")
off_home = home()
ok('<section class="mostread"' not in off_home, "spento: la home non ha il blocco")  # lo stile dei temi può nominarlo, il blocco no
art_before = os.stat(f"{PUB}/articolo-a/index.html").st_mtime
setting("google_sa", json.dumps(sa)); setting("ga_property", "123456789"); setting("most_read_on", "on"); setting("most_read_count", "3"); setting("most_read_at", str(int(time.time())))
r = curl(A, "-o", "/dev/null", "-w", "%{redirect_url}", "-X", "POST", B + "/admin/integrations/piu-letti")
m = urllib.parse.unquote_plus(r.split("msg=", 1)[-1])
ok("«Articolo B» (900 visite), «Articolo A» (500 visite), «Articolo C» (300 visite)" in m, "il pulsante mostra i 3 più letti con le visite, nell'ordine giusto")
ok(seen["scope"].endswith("analytics.readonly") and seen["auth"] == "Bearer tok-ga", "chiede a Google il permesso di sola lettura di Analytics, con il suo token")
ok(seen["body"]["metrics"][0]["name"] == "screenPageViews" and seen["body"]["dateRanges"][0]["startDate"] == "1daysAgo", "chiede le visualizzazioni delle ultime 24 ore")
h = home(); sec = h.split('<section class="mostread"', 1)[-1].split("</section>", 1)[0]
order = re.findall(r'<li><a href="[^"]+">([^<]+)</a></li>', sec)
ok(order == ["Articolo B", "Articolo A", "Articolo C"], f"la home ha il blocco «I più letti» nell'ordine delle visite ({order})")
ok("<script" not in sec and h.count("<script") == off_home.count("<script"), "niente JavaScript in più nella home")
ok(os.stat(f"{PUB}/articolo-a/index.html").st_mtime == art_before, "gli articoli non vengono rigenerati: cambia solo la home")
print(f"   peso aggiunto alla home: {len(h.encode()) - len(off_home.encode())} byte, prima della compressione")
rows["data"] = [("/articolo-d/", 2000), ("/articolo-c/", 1500), ("/articolo-b/", 10)]
setting("most_read_at", "0")
for _ in range(80):
    if "Articolo D" in home().split('<section class="mostread"', 1)[-1].split("</section>", 1)[0]: break
    time.sleep(1)
order = re.findall(r'<li><a href="[^"]+">([^<]+)</a></li>', home().split('<section class="mostread"', 1)[-1].split("</section>", 1)[0])
ok(order[:2] == ["Articolo D", "Articolo C"], f"si aggiorna da solo quando è il momento ({order})")
setting("ga_property", "999")
m = urllib.parse.unquote_plus(curl(A, "-o", "/dev/null", "-w", "%{redirect_url}", "-X", "POST", B + "/admin/integrations/piu-letti").split("msg=", 1)[-1])
ok("Visualizzatore" in m, "se Google nega l'accesso, il messaggio spiega cosa fare")
setting("most_read_on", ""); curl(A, "-o", "/dev/null", "-X", "POST", B + "/admin/rebuild")
ok('<section class="mostread"' not in home(), "rispento: la home torna senza il blocco")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
