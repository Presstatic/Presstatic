import subprocess, json, os, re, urllib.parse, sqlite3, time, threading
B = "http://127.0.0.1:8086"; D = "/tmp/sec"; os.chdir(D)
def curl(jar, *a, out="/dev/null", w="%{http_code} %{redirect_url}"):
    return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-o", out, "-w", w, *a], capture_output=True, text=True).stdout
def msg(r): return urllib.parse.unquote(r.split("msg=", 1)[-1]) if "msg=" in r else r
fails = 0
def check(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l)
def rss(): return int(open(f"/proc/{pid}/status").read().split("VmRSS:")[1].split()[0]) // 1024
pid = [p for p in os.popen("pgrep -x presstatic").read().split() if os.readlink(f"/proc/{p}/cwd") == D][0]

curl("a.jar", "-d", "email=admin@x.it&password=password-lunga-123", B + "/admin/login")
curl("a.jar", "--form-string", "name=Autore", "--form-string", "email=autore@x.it", "--form-string", "role=author", "--form-string", "password=password-lunga-123", B + "/admin/users/0")
curl("s.jar", "-d", "email=autore@x.it&password=password-lunga-123", B + "/admin/login")

print("1) Link pericolosi nel video")
video = ["--form-string", "title=Video innocuo", "--form-string", "body=<p>Guarda il video.</p>", "--form-string", "schema_type=VideoObject", "--form-string", "sd_embedUrl=javascript:alert(document.domain)", "--form-string", "sd_contentUrl=data:text/html,<script>alert(1)</script>"]
r = curl("s.jar", *video, "--form-string", "status=published", B + "/admin/edit/0")
vid = re.search(r"edit/(\d+)", r).group(1)
curl("a.jar", *video, "--form-string", "slug=video-innocuo", "--form-string", "status=published", B + f"/admin/edit/{vid}")
page = open(f"{D}/public/video-innocuo/index.html").read()
check("javascript:" not in page and "data:text/html" not in page, "il link javascript: e quello data: non arrivano nella pagina pubblicata")
db = sqlite3.connect(f"{D}/presstatic.db")
db.execute("UPDATE posts SET schema_data = ? WHERE id = ?", (json.dumps({"embedUrl": "javascript:alert(2)", "duration": "3"}), vid)); db.commit()
curl("a.jar", "-X", "POST", B + "/admin/rebuild")
page = open(f"{D}/public/video-innocuo/index.html").read()
check("javascript:" not in page, "anche un valore pericoloso già salvato nel database viene scartato")
curl("a.jar", *video[:-4], "--form-string", "sd_embedUrl=https://www.youtube-nocookie.com/embed/abcdefghijk", "--form-string", "slug=video-innocuo", "--form-string", "status=published", B + f"/admin/edit/{vid}")
check('src="https://www.youtube-nocookie.com/embed/abcdefghijk"' in open(f"{D}/public/video-innocuo/index.html").read(), "un link https normale funziona ancora")

print("2) Richieste da altri siti (CSRF)")
evil = ["--form-string", "name=Intruso", "--form-string", "email=intruso@evil.example", "--form-string", "role=admin", "--form-string", "password=password-lunga-123"]
for label, hdr in [("Origin di un altro sito", ["-H", "Origin: https://evil.example"]),
                   ("sito pubblico www sullo stesso dominio (Sec-Fetch-Site: same-site)", ["-H", "Sec-Fetch-Site: same-site", "-H", "Origin: https://www.miosito.it", "-H", "Host: admin.miosito.it"]),
                   ("pagina in sandbox (Origin: null)", ["-H", "Origin: null"])]:
    check(curl("a.jar", *hdr, *evil, B + "/admin/users/0").startswith("403"), f"rifiutata: {label}")
check("intruso@" not in curl("a.jar", B + "/admin/users", out="/tmp/u.html", w="") + open("/tmp/u.html").read(), "nessun utente creato dall'esterno")
check(curl("a.jar", "-H", "Origin: http://127.0.0.1:8086", "-H", "Sec-Fetch-Site: same-origin", "--form-string", "name=Collega", "--form-string", "email=collega@x.it", "--form-string", "role=editor", "--form-string", "password=password-lunga-123", B + "/admin/users/0").startswith("303"), "le richieste dal pannello stesso funzionano")
check(curl("x.jar", "-H", "Origin: https://evil.example", "-d", "email=admin@x.it&password=password-lunga-123", B + "/admin/login").startswith("403"), "anche l'accesso da un altro sito è rifiutato")

print("3) Pagine pubbliche sul dominio del pannello")
h = subprocess.run(["curl", "-s", "-D", "-", "-o", "/dev/null", B + "/video-innocuo/"], capture_output=True, text=True).stdout.lower()
check("content-security-policy: sandbox" in h, "le pagine del sito si aprono in sandbox")
h = subprocess.run(["curl", "-s", "-D", "-", "-o", "/dev/null", B + "/non-esiste/"], capture_output=True, text=True).stdout.lower()
check("404" in h.split("\n")[0] and "content-security-policy: sandbox" in h, "anche la pagina 404 (quella con gli script configurati)")
h = subprocess.run(["curl", "-s", "-D", "-", "-o", "/dev/null", B + "/assets/fonts/newsreader-5.3.woff2"], capture_output=True, text=True).stdout.lower()
check("200" in h.split("\n")[0] and "nosniff" in h, "font e immagini restano disponibili al pannello")

print("4) Fonti dell'IA verso indirizzi interni (SSRF)")
for u in ["http://127.0.0.1/", "http://localhost:8086/admin", "http://169.254.169.254/latest/meta-data/", "http://[::1]/", "http://10.0.0.1/", "http://192.168.1.1/", "http://127.0.0.1:8197/fonte"]:
    h = subprocess.run(["curl", "-s", "-b", "a.jar", "--data-urlencode", f"sources={u}", "-d", "model=anthropic:claude-sonnet-5", B + "/admin/ai"], capture_output=True, text=True).stdout
    check("solo indirizzi pubblici" in h, f"bloccato {u}")
h = subprocess.run(["curl", "-s", "-b", "a.jar", "--data-urlencode", "sources=https://github.com/Pagefind/pagefind", "-d", "model=anthropic:claude-sonnet-5", B + "/admin/ai", "-o", "/dev/null", "-w", "%{redirect_url}"], capture_output=True, text=True).stdout
print("     fonte pubblica (github.com):", msg(h)[:110] or "risposta senza reindirizzamento")

print("5) Tentativi di accesso")
def attempt(ip, pw="sbagliata-123456"):
    return subprocess.run(["curl", "-s", "-o", "/dev/null", "-w", "%{redirect_url}", "-H", f"X-Real-IP: {ip}", "-d", f"email=admin@x.it&password={pw}", B + "/admin/login"], capture_output=True, text=True).stdout
ts = [threading.Thread(target=attempt, args=("203.0.113.5",)) for _ in range(10)]
t0 = time.time(); [t.start() for t in ts]; [t.join() for t in ts]
check("Troppi tentativi" in msg(attempt("203.0.113.5", "password-lunga-123")), "dopo 10 tentativi sbagliati quell'indirizzo è bloccato, anche con la password giusta")
check(attempt("203.0.113.9", "password-lunga-123").endswith("/admin"), "da un altro indirizzo l'amministratore entra normalmente")
# Un X-Forwarded-For falso non deve far sembrare un altro indirizzo (vulnerabilità corretta nella revisione).
fake = subprocess.run(["curl", "-s", "-o", "/dev/null", "-w", "%{redirect_url}", "-H", "X-Real-IP: 203.0.113.5", "-H", "X-Forwarded-For: 8.8.8.8", "-d", "email=admin@x.it&password=password-lunga-123", B + "/admin/login"], capture_output=True, text=True).stdout
check("Troppi tentativi" in msg(fake), "un X-Forwarded-For falso non aggira il blocco dell'indirizzo")
print(f"     10 tentativi in parallelo gestiti in {time.time()-t0:.1f} s")

print("6) Immagine «bomba»")
before = rss(); t0 = time.time()
out = subprocess.run(["curl", "-s", "-b", "a.jar", "-F", "file=@/tmp/bomba.png", B + "/admin/upload"], capture_output=True, text=True).stdout
check("error" in out and time.time() - t0 < 5, f"rifiutata in {time.time()-t0:.2f} s: {json.loads(out).get('error','')[:70]}")
check(rss() - before < 50, f"memoria del processo: {before} MB prima, {rss()} MB dopo")

print("7) Testo scritto dall'IA")
r = curl("a.jar", "--data-urlencode", "notes=Il ponte riapre il 25 settembre.", "-d", "model=anthropic:claude-sonnet-5", B + "/admin/ai")
aid = re.search(r"edit/(\d+)", r).group(1)
body = db.execute("SELECT body FROM posts WHERE id = ?", (aid,)).fetchone()[0]
check("<script" not in body and "onclick" not in body and "Il ponte sul canale di Latina" in body, "script e attributi pericolosi tolti anche per le bozze di un amministratore; il testo resta")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
