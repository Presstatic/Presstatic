# Concorrenza nel pannello, oltre alla redazione sugli articoli (vedi redazione-contemporanea.py).
#  1. Due immagini con lo stesso nome caricate nello stesso istante non si sovrascrivono.
#  2. Impostazioni con un campo sbagliato: non si salva niente (prima restavano salvate a metà).
#  3. Due amministratori che si tolgono il ruolo a vicenda nello stesso istante: resta almeno un amministratore.
#  4. Articolo programmato con la pubblicazione che fallisce: si riprova, invece di perderlo.
#  5. Programma fermato mentre scriveva pagine: al riavvio il sito si rigenera da solo.
#  6. Impostazioni, integrazioni e profili: chi salva sopra il lavoro di un altro amministratore viene fermato.
# Uso: python3 tests/concorrenza-pannello.py http://127.0.0.1:8199 /percorso/del/sito /percorso/del/programma
import subprocess, sys, re, sqlite3, threading, time, os, io, shutil, urllib.parse
from PIL import Image
B, DIR, BIN = sys.argv[1], sys.argv[2], sys.argv[3]
PORT = B.rsplit(":", 1)[1]
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
def curl(jar, *a): return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
def post(jar, path, fields=None):
    args = []
    for k, v in (fields or {}).items(): args += ["--form-string", f"{k}={v}"]
    return curl(jar, "-o", "/dev/null", "-w", "%{redirect_url}", *args, "-X", "POST", B + path)
def post_url(jar, path, fields):  # moduli inviati come application/x-www-form-urlencoded (per esempio le integrazioni)
    args = []
    for k, v in fields.items(): args += ["--data-urlencode", f"{k}={v}"]
    return curl(jar, "-o", "/dev/null", "-w", "%{redirect_url}", *args, B + path)
def msg(r): return urllib.parse.unquote_plus(r.split("msg=", 1)[-1]) if "msg=" in r else r
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def login(jar, email):
    open(jar, "w").close()
    curl(jar, "-o", "/dev/null", "--data-urlencode", f"email={email}", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
def start():
    subprocess.Popen(f"cd {DIR} && PRESSTATIC_ADDR=127.0.0.1:{PORT} exec {BIN} >> s.log 2>&1", shell=True, start_new_session=True)
    for _ in range(50):
        if subprocess.run(["curl", "-s", "-o", "/dev/null", B + "/admin/login"]).returncode == 0: return
        time.sleep(0.2)
def stop():
    for p in subprocess.run(["pgrep", "-x", "presstatic"], capture_output=True, text=True).stdout.split():
        if os.path.realpath(f"/proc/{p}/cwd") == os.path.realpath(DIR): subprocess.run(["kill", "-9", p])
    time.sleep(0.5)

A, Bj = "/tmp/cp-a.jar", "/tmp/cp-b.jar"
login(A, "andrea@example.com")
post(A, "/admin/users/0", {"name": "Bruno Admin", "email": "bruno@example.com", "password": "password-lunga-123", "role": "admin"})
login(Bj, "bruno@example.com")

print("== 1. Immagini con lo stesso nome nello stesso istante")
for i, color in enumerate(((200, 30, 30), (30, 30, 200))):
    Image.new("RGB", (1300, 800), color).save(f"/tmp/cp-foto{i}.jpg", quality=90)
urls = [None, None]
def up(i): urls[i] = re.search(r'"url":"([^"]+)"', curl(A, "-F", f"file=@/tmp/cp-foto{i}.jpg;filename=foto.jpg", B + "/admin/upload")).group(1)
ts = [threading.Thread(target=up, args=(i,)) for i in range(2)]
for t in ts: t.start()
for t in ts: t.join()
ok(urls[0] != urls[1], f"due indirizzi diversi ({urls[0].rsplit('/', 1)[1]} e {urls[1].rsplit('/', 1)[1]})")
pix = [Image.open(DIR + "/public" + u).convert("RGB").getpixel((10, 10)) for u in urls]
ok(pix[0][0] > 150 and pix[1][2] > 150, "ognuna ha la sua immagine: la rossa è rossa, la blu è blu")

print("== 2. Impostazioni con un campo sbagliato")
with db() as c: before = c.execute("SELECT value FROM settings WHERE key = 'site_name'").fetchone()
m = msg(post(A, "/admin/settings", {"site_name": "Nome cambiato", "base_url": "http://127.0.0.1:8080", "cf_zone": "sbagliato"}))
with db() as c: after = c.execute("SELECT value FROM settings WHERE key = 'site_name'").fetchone()
ok("Zone ID" in m, "il campo sbagliato viene segnalato")
ok(before == after, "e niente viene salvato: il nome del sito è rimasto quello di prima")

print("== 3. Due amministratori si tolgono il ruolo a vicenda")
with db() as c: ida, idb = [r[0] for r in c.execute("SELECT id FROM users WHERE email IN ('andrea@example.com', 'bruno@example.com') ORDER BY email")]
zero = 0
for _ in range(8):
    with db() as c: c.execute("UPDATE users SET role = 'admin' WHERE id IN (?, ?)", (ida, idb))
    ts = [threading.Thread(target=post, args=(A, f"/admin/users/{idb}", {"name": "Bruno Admin", "email": "bruno@example.com", "role": "editor"})),
          threading.Thread(target=post, args=(Bj, f"/admin/users/{ida}", {"name": "Andrea Admin", "email": "andrea@example.com", "role": "editor"}))]
    for t in ts: t.start()
    for t in ts: t.join()
    with db() as c: zero += c.execute("SELECT COUNT(*) FROM users WHERE role = 'admin'").fetchone()[0] == 0
ok(zero == 0, "8 prove nello stesso istante: resta sempre almeno un amministratore")
with db() as c: c.execute("UPDATE users SET role = 'admin' WHERE id = ?", (ida,)); c.execute("UPDATE users SET role = 'editor' WHERE id = ?", (idb,))
m = msg(post(A, f"/admin/users/{ida}", {"name": "Andrea Admin", "email": "andrea@example.com", "role": "editor"}))
ok("non puoi togliere a te stesso" in m, "l'ultimo amministratore non può togliersi il ruolo")

print("== 4. Articolo programmato con la pubblicazione che fallisce")
# programmato per domani, poi spostato a tra 35 secondi direttamente nel database (il campo del modulo ha i minuti)
r = post(A, "/admin/edit/0", {"title": "Programmato da riprovare", "body": "<p>Esce da solo.</p>", "category": "Cronaca", "status": "published",
                              "published_at": time.strftime("%Y-%m-%dT%H:%M", time.localtime(time.time() + 86400))})
with db() as c: c.execute("UPDATE posts SET published_at = ? WHERE title = 'Programmato da riprovare'", (int(time.time()) + 35,))
home = f"{DIR}/public/index.html"
os.remove(home); os.mkdir(home)  # la home non si può scrivere: la pubblicazione programmata fallisce
deadline = time.time() + 150
while time.time() < deadline and "riprovo tra 30 secondi" not in open(f"{DIR}/s.log").read(): time.sleep(3)
ok("riprovo tra 30 secondi" in open(f"{DIR}/s.log").read(), "la pubblicazione programmata fallisce e il programma dice che riproverà")
os.rmdir(home)  # guasto risolto
deadline = time.time() + 70
while time.time() < deadline and not (os.path.isfile(home) and "Programmato da riprovare" in open(home, encoding="utf-8").read()): time.sleep(3)
ok(os.path.isfile(home) and "Programmato da riprovare" in open(home, encoding="utf-8").read(), "al giro successivo riesce: l'articolo è in home")

print("== 5. Programma fermato mentre scriveva pagine")
stop()
open(f"{DIR}/.generazione-in-corso", "w").close()
os.remove(home)
start()
deadline = time.time() + 60
while time.time() < deadline and not os.path.isfile(home): time.sleep(1)
ok(os.path.isfile(home), "al riavvio il sito si rigenera da solo: la home è di nuovo al suo posto")
ok(not os.path.exists(f"{DIR}/.generazione-in-corso"), "e il segno di lavoro interrotto sparisce")
ok("si era fermato mentre aggiornava il sito" in open(f"{DIR}/s.log").read(), "nel registro del servizio c'è la spiegazione")

print("== 6. Impostazioni, integrazioni e profili salvati da due persone")
login(A, "andrea@example.com")
def seen(path, field): return re.search(rf'name="{field}" value="(\d+)"', curl(A, B + path)).group(1)
sv = seen("/admin/settings", "sv")
base = {"base_url": "http://127.0.0.1:8080", "sv": sv}
ok(msg(post(A, "/admin/settings", {**base, "site_name": "Primo salvataggio"})).startswith("Impostazioni salvate"), "impostazioni: il primo salvataggio passa")
m = msg(post(A, "/admin/settings", {**base, "site_name": "Secondo, dalla pagina vecchia"}))
with db() as c: name = c.execute("SELECT value FROM settings WHERE key = 'site_name'").fetchone()[0]
ok("nel frattempo un altro amministratore" in m and name == "Primo salvataggio", "impostazioni: il secondo, partito dalla pagina vecchia, viene fermato e non sovrascrive")
iv = seen("/admin/integrations", "sv")
ok(msg(post_url(A, "/admin/integrations", {"sv": iv, "ai_style": "Tono sobrio."})).startswith("Integrazioni salvate"), "integrazioni: il primo salvataggio passa")
ok("nel frattempo un altro amministratore" in msg(post_url(A, "/admin/integrations", {"sv": iv, "ai_style": "Tono diverso."})), "integrazioni: il secondo, dalla pagina vecchia, viene fermato")
uv = seen(f"/admin/users/{idb}", "uv")
ok(msg(post(A, f"/admin/users/{idb}", {"uv": uv, "name": "Bruno Rossi", "email": "bruno@example.com", "role": "editor"})).startswith("Profilo salvato"), "profilo: il primo salvataggio passa")
m = msg(post(A, f"/admin/users/{idb}", {"uv": uv, "name": "Bruno Verdi", "email": "bruno@example.com", "role": "editor"}))
with db() as c: bn = c.execute("SELECT name FROM users WHERE id = ?", (idb,)).fetchone()[0]
ok("nel frattempo qualcun altro" in m and bn == "Bruno Rossi", "profilo: il secondo, dalla pagina vecchia, viene fermato e non sovrascrive")

print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
