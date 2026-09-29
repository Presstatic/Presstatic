# Ultimi punti della revisione di sicurezza: intestazioni delle email (niente destinatari aggiunti con un «a capo» nel
# titolo), limiti di dimensione sulle rotte pubbliche, archivi ZIP malevoli nel file manager, ricerca con sintassi ostile.
# Uso: python3 tests/revisione-finale.py http://127.0.0.1:PORTA /percorso/del/sito
import subprocess, sys, os, re, time, sqlite3, zipfile, stat, email, email.policy, urllib.parse
from aiosmtpd.controller import Controller
B, DIR = sys.argv[1], sys.argv[2]
fails = 0
def ok(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l, flush=True)
got = []
class Box:
    async def handle_DATA(self, s, ss, env): got.append((list(env.rcpt_tos), env.content.decode("utf-8", "replace"))); return "250 OK"
Controller(Box(), hostname="127.0.0.1", port=2551).start()
def c(jar, *a): return subprocess.run(["curl", "-s", "-m", "30", "-b", jar, "-c", jar, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
A, U = DIR + "/a.jar", DIR + "/u.jar"
for j in (A, U): open(j, "w").close()
db = lambda: sqlite3.connect(DIR + "/presstatic.db")
with db() as d:
    for k, v in [("smtp_host", "127.0.0.1"), ("smtp_port", "2551"), ("smtp_security", "none"), ("smtp_from", "r@x.it"), ("admin_url", B)]: d.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (k, v))
c(A, "-o", "/dev/null", "--data-urlencode", "email=a@x.it", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
c(A, "-o", "/dev/null", "--form-string", "name=Autore", "--form-string", "email=aut@x.it", "--form-string", "password=password-lunga-123", "--form-string", "role=author", B + "/admin/users/0")
c(U, "-o", "/dev/null", "--data-urlencode", "email=aut@x.it", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")

print("== 1. Intestazioni delle email: un «a capo» nel titolo non aggiunge destinatari")
evil = "Titolo\r\nBcc: vittima@esempio.it\r\nX-Iniettato: si"
c(U, "-o", "/dev/null", "--form-string", f"title={evil}", "--form-string", "slug=iniezione", "--form-string", "body=<p>x</p>", "--form-string", "status=draft", B + "/admin/edit/0")
with db() as d: pid, title = d.execute("SELECT id, title FROM posts WHERE slug = 'iniezione'").fetchone()
got.clear()
c(A, "-o", "/dev/null", "--data-urlencode", "body=Una nota della redazione", B + f"/admin/edit/{pid}/note")
for _ in range(20):
    if got: break
    time.sleep(0.3)
rcpts = [r for rs, _ in got for r in rs]
ok("\r" not in title and "\n" not in title, "il titolo salvato è su una riga sola (nessun a capo né ritorno carrello)")
ok(got and rcpts == ["aut@x.it"], f"la nota arriva solo all'autore (destinatari: {rcpts})")
head = got[0][1].split("\r\n\r\n", 1)[0] if got else ""
ok(got and not re.search(r"(?im)^(bcc|x-iniettato):", head), "nelle intestazioni dell'email non compaiono righe aggiunte dal titolo")

print("== 2. Limiti di dimensione sulle rotte pubbliche")
big = DIR + "/grande.bin"; open(big, "wb").write(b"a=" + b"x" * (2 * 1024 * 1024))
for path in (f"/commenti/{pid}", "/newsletter/iscriviti", "/push/iscrivi"):
    t = time.time(); code = subprocess.run(["curl", "-s", "-m", "20", "-o", "/dev/null", "-w", "%{http_code}", "--data-binary", "@" + big, B + path], capture_output=True, text=True).stdout
    ok(code == "413", f"{path}: 2 MB rifiutati con 413 in {time.time() - t:.2f} s")

print("== 3. File manager: archivi ZIP malevoli")
os.makedirs(DIR + "/fuori", exist_ok=True); os.makedirs(DIR + "/public/prove", exist_ok=True)
def z(name, entries):
    with zipfile.ZipFile(f"{DIR}/{name}", "w", zipfile.ZIP_DEFLATED) as zf:
        for info, data in entries: zf.writestr(info, data)
z("slip.zip", [(zipfile.ZipInfo("../../../fuori/slip.txt"), b"ZIP SLIP"), (zipfile.ZipInfo("ok.txt"), b"legittimo")])
z("assoluto.zip", [(zipfile.ZipInfo(DIR + "/fuori/assoluto.txt"), b"ASSOLUTO")])
li = zipfile.ZipInfo("link-passwd"); li.create_system = 3; li.external_attr = (stat.S_IFLNK | 0o777) << 16
z("link.zip", [(li, b"/etc/passwd")])
with zipfile.ZipFile(f"{DIR}/bomba.zip", "w", zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
    with zf.open("zeri.bin", "w", force_zip64=True) as f:
        for _ in range(400): f.write(b"\0" * (1 << 20))
for name in ("slip", "assoluto", "link", "bomba"):
    c(A, "-o", "/dev/null", "-F", "action=upload", "-F", "path=public/prove", "-F", f"files=@{DIR}/{name}.zip", B + "/admin/files")
    c(A, "-o", "/dev/null", "-F", "action=extract", "-F", f"path=public/prove/{name}.zip", B + "/admin/files")
ok(not os.listdir(DIR + "/fuori"), "zip slip e percorso assoluto: nessun file scritto fuori dal sito")
ok(os.path.exists(DIR + "/public/prove/ok.txt"), "il file legittimo dello stesso archivio viene estratto")
ok(not os.path.islink(DIR + "/public/prove/link-passwd"), "il collegamento verso /etc/passwd non viene creato")
ok(not os.path.exists(DIR + "/public/prove/zeri.bin"), "la zip bomb (400 MB da 398 KB) viene fermata e ripulita")

print("== 4. Ricerca: sintassi ostile e tentativi di iniezione")
c(A, "-o", "/dev/null", "--form-string", "title=Il ponte riaperto", "--form-string", "slug=ponte", "--form-string", "body=<p>Il ponte sul fiume</p>", "--form-string", "status=published", B + "/admin/edit/0")
bad = []
for q in ['"ponte', 'pon*', 'NEAR(ponte fiume)', 'ponte OR 1=1', '-ponte', '^ponte', "ponte'; DROP TABLE posts;--", 'title:ponte', '(((', '""""']:
    code = subprocess.run(["curl", "-s", "-m", "10", "-o", "/dev/null", "-w", "%{http_code}", "-b", A, "-G", "--data-urlencode", f"q={q}", B + "/admin"], capture_output=True, text=True).stdout
    if code != "200": bad.append((q, code))
with db() as d: n = d.execute("SELECT COUNT(*) FROM posts WHERE slug = 'ponte'").fetchone()[0]
ok(not bad and n == 1, "10 ricerche ostili: nessun errore, tabella degli articoli intatta" + (f" {bad}" if bad else ""))
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
