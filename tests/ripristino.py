# Ripristino dei backup dal pannello: copia sul server, archivio caricato, archivio manomesso, permessi.
# Uso: python3 tests/ripristino.py http://127.0.0.1:PORTA /percorso/del/sito
import subprocess, sys, sqlite3, os, time, tarfile, io, glob
B, D = sys.argv[1], sys.argv[2]
fails = 0
def ok(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l, flush=True)
J = D + "/j"
def login(email="a@x.it"):
    open(J, "w").close(); c("-o", "/dev/null", "--data-urlencode", f"email={email}", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
def c(*a): return subprocess.run(["curl", "-s", "-m", "120", "-b", J, "-c", J, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
def msg(r): import urllib.parse; return urllib.parse.unquote(r.split("msg=", 1)[-1]) if "msg=" in r else r
db = lambda: sqlite3.connect(D + "/presstatic.db")
titles = lambda: sorted(t for (t,) in db().execute("SELECT title FROM posts WHERE kind = 'post'"))
copies = lambda: sorted(os.path.basename(p) for p in glob.glob(D + "/backups/presstatic-backup-*.tar.gz"))
login()
c("-o", "/dev/null", "--form-string", "title=Articolo di prima", "--form-string", "slug=prima", "--form-string", "body=<p>Testo.</p>", "--form-string", "status=published", B + "/admin/edit/0")
c("-o", "/dev/null", "-X", "POST", B + "/admin/backup/esegui")
for _ in range(60):
    if copies(): break
    time.sleep(0.5)
time.sleep(1.5)
copia = copies()[0] if copies() else ""
ok(bool(copia), f"backup fatto: {copia}")
pid = db().execute("SELECT id FROM posts WHERE slug = 'prima'").fetchone()[0]
c("-o", "/dev/null", "--form-string", "title=Articolo di dopo", "--form-string", "slug=dopo", "--form-string", "body=<p>Testo.</p>", "--form-string", "status=published", B + "/admin/edit/0")
c("-o", "/dev/null", "-X", "POST", B + f"/admin/delete/{pid}")
with db() as d: d.execute("UPDATE settings SET value = 'Nome cambiato' WHERE key = 'site_name'")
ok(titles() == ["Articolo di dopo"], f"dopo il backup il sito è cambiato ({titles()})")
print("== Ripristino di una copia sul server")
before = len(copies())
r = c("-o", "/dev/null", "-w", "%{redirect_url}", "--data-urlencode", f"name={copia}", B + "/admin/backup/ripristina")
m = msg(r)
ok("Ripristino completato" in m, f"il pannello conferma: «{m[:110]}…»")
ok(titles() == ["Articolo di prima"], f"articoli tornati com'erano ({titles()})")
ok(os.path.exists(D + "/public/prima/index.html") and not os.path.exists(D + "/public/dopo/index.html"), "sito rigenerato: la pagina di prima c'è, quella nata dopo il backup no")
ok(len(copies()) == before + 1, "prima del ripristino è stata salvata una copia dello stato precedente")
print("== Ripristino di un archivio caricato")
login()
c("-o", "/dev/null", "--form-string", "title=Articolo intermedio", "--form-string", "slug=intermedio", "--form-string", "body=<p>x</p>", "--form-string", "status=published", B + "/admin/edit/0")
r = c("-o", "/dev/null", "-w", "%{redirect_url}", "-F", f"archivio=@{D}/backups/{copia};filename=backup.tar.gz", B + "/admin/backup/ripristina-file")
ok("Ripristino completato" in msg(r) and titles() == ["Articolo di prima"], f"archivio caricato dal computer: ripristinato ({titles()})")
ok(not glob.glob(D + "/backups/caricato-*"), "l'archivio caricato non resta sul server")
print("== Archivio manomesso")
login()
c("-o", "/dev/null", "--form-string", "title=Da non perdere", "--form-string", "slug=da-non-perdere", "--form-string", "body=<p>x</p>", "--form-string", "status=published", B + "/admin/edit/0")
bad = D + "/cattivo.tar.gz"
with tarfile.open(bad, "w:gz") as t:
    data = open(D + "/presstatic.db", "rb").read(); i = tarfile.TarInfo("presstatic.db"); i.size = len(data); t.addfile(i, io.BytesIO(data))
    i = tarfile.TarInfo("../fuori.txt"); i.size = 5; t.addfile(i, io.BytesIO(b"FUORI"))
r = c("-o", "/dev/null", "-w", "%{redirect_url}", "-F", f"archivio=@{bad};filename=cattivo.tar.gz", B + "/admin/backup/ripristina-file")
ok("non ammesso" in msg(r) and "Da non perdere" in titles() and not os.path.exists(os.path.dirname(D) + "/fuori.txt"), "archivio con percorsi fuori dal sito: rifiutato, niente toccato")
with tarfile.open(bad, "w:gz") as t:
    i = tarfile.TarInfo("public/link"); i.type = tarfile.SYMTYPE; i.linkname = "/etc"; t.addfile(i)
r = c("-o", "/dev/null", "-w", "%{redirect_url}", "-F", f"archivio=@{bad};filename=cattivo.tar.gz", B + "/admin/backup/ripristina-file")
ok("collegamento" in msg(r) and not os.path.islink(D + "/public/link"), "archivio con un collegamento a /etc: rifiutato")
print("== Permessi")
c("-o", "/dev/null", "--form-string", "name=Red", "--form-string", "email=red@x.it", "--form-string", "password=password-lunga-123", "--form-string", "role=editor", B + "/admin/users/0")
login("red@x.it")
code = c("-o", "/dev/null", "-w", "%{http_code}", "--data-urlencode", f"name={copia}", B + "/admin/backup/ripristina")
ok(code in ("403", "303") and "Da non perdere" in titles(), f"un redattore non può ripristinare ({code})")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
