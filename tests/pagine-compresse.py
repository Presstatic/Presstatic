# Pagine già compresse: dopo pubblicazioni e rigenerazioni, accanto a ogni pagina ci sono index.html.gz e index.html.br
# identiche alla pagina; se la pagina cambia, le versioni compresse si aggiornano; il file manager non le mostra.
import subprocess, os, re, time, gzip, glob, urllib.parse, brotli
B = "http://127.0.0.1:8195"; D = "/tmp/pc"; os.chdir(D); fails = 0
def c(*a, out="/dev/null"): return subprocess.run(["curl", "-s", "-b", "j", "-c", "j", "-o", out, "-w", "%{redirect_url}", *a], capture_output=True, text=True).stdout
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
def same(f):
    d = open(f, "rb").read()
    try: return gzip.decompress(open(f + ".gz", "rb").read()) == d and brotli.decompress(open(f + ".br", "rb").read()) == d
    except FileNotFoundError: return False
def wait_all(limit=60):
    for _ in range(limit * 5):
        if all(os.path.exists(f + ".br") and os.path.exists(f + ".gz") for f in glob.glob("public/**/*.html", recursive=True)): return True
        time.sleep(0.2)
    return False
c("-d", "email=admin@x.it&password=password-lunga-123", B + "/admin/login")
for i in range(5):
    c("--form-string", f"title=Articolo {i}", "--form-string", f"slug=articolo-{i}", "--form-string", "body=<p>" + "testo di prova " * 200 + "</p>", "--form-string", "category=Cronaca", "--form-string", "status=published", B + "/admin/edit/0")
c("-X", "POST", B + "/admin/rebuild")
ok(wait_all(), "ogni pagina HTML ha la sua versione gzip e brotli")
pages = glob.glob("public/**/*.html", recursive=True)
ok(all(same(f) for f in pages), f"le versioni compresse sono identiche alla pagina ({len(pages)} pagine)")
ok(same("public/sitemap.xml") if os.path.exists("public/sitemap.xml.br") else False, "anche la sitemap ha le versioni compresse")
import sqlite3
pid = sqlite3.connect("presstatic.db").execute("SELECT id FROM posts WHERE slug = 'articolo-0'").fetchone()[0]
c("--form-string", "title=Titolo cambiato", "--form-string", "slug=articolo-0", "--form-string", "body=<p>Testo nuovo</p>", "--form-string", "category=Cronaca", "--form-string", "status=published", B + f"/admin/edit/{pid}")
time.sleep(1.5)
d = open("public/articolo-0/index.html", "rb").read()
ok(b"Titolo cambiato" in d and same("public/articolo-0/index.html"), "dopo una modifica le versioni compresse riportano il testo nuovo")
listing = subprocess.run(["curl", "-s", "-b", "j", B + "/admin/files?path=public/articolo-1"], capture_output=True, text=True).stdout
ok("index.html" in listing and ".html.br" not in listing and ".html.gz" not in listing, "il file manager mostra index.html ma non le versioni compresse")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
