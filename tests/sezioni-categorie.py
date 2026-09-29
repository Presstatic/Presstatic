# Sezioni dell'installazione guidata = menu + categorie; pagine delle sezioni vuote; rinomina ed eliminazione
# che aggiornano il menu. Uso: python3 tests/sezioni-categorie.py http://127.0.0.1:PORTA /percorso/del/sito
import subprocess, sys, sqlite3, os, re
B, D = sys.argv[1], sys.argv[2]
fails = 0
def ok(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l, flush=True)
def c(*a): return subprocess.run(["curl", "-s", "-m", "60", "-b", D + "/j", "-c", D + "/j", "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
db = lambda: sqlite3.connect(D + "/presstatic.db")
cats = lambda: sorted(r[0] for r in db().execute("SELECT name FROM categories"))
menu = lambda: (db().execute("SELECT value FROM settings WHERE key = 'menu'").fetchone() or [""])[0].splitlines()
page = lambda p: open(f"{D}/public/{p}", encoding="utf-8").read() if os.path.exists(f"{D}/public/{p}") else ""
tok = open(D + "/setup-token.txt").read().strip()
c("-o", "/dev/null", "--form-string", f"token={tok}", "--form-string", "site_name=Prova", "--form-string", "base_url=https://www.prova.it",
  "--form-string", "name=Admin", "--form-string", "email=a@x.it", "--form-string", "password=password-lunga-123",
  "--form-string", "menu=Cronaca\nPolitica\nEconomia\nContatti | /contatti/", B + "/admin/setup")
print("== Installazione guidata")
ok(cats() == ["Cronaca", "Economia", "Politica"], f"le sezioni diventano categorie, il link «Contatti» no ({cats()})")
h = page("category/politica/index.html")
ok("Non ci sono ancora articoli" in h, "la sezione ancora vuota ha la sua pagina, non «non trovata»")
ok('content="noindex' in h, "e la pagina vuota non si fa indicizzare")
ok("/category/politica/" not in page("sitemaps/pages.xml"), "e non è nella sitemap")
ok("Politica" in c(B + "/admin/edit/0"), "l'editor propone la sezione anche senza articoli")
print("== Primo articolo")
c("-o", "/dev/null", "--form-string", "title=Il consiglio approva il bilancio", "--form-string", "body=<p>Testo.</p>", "--form-string", "category=Politica", "--form-string", "status=published", B + "/admin/edit/0")
h = page("category/politica/index.html")
ok("Il consiglio approva il bilancio" in h and 'content="noindex' not in h, "con il primo articolo la pagina diventa quella vera, indicizzabile")
ok("/category/politica/" in page("sitemaps/pages.xml"), "e entra nella sitemap")
print("== Rinomina ed eliminazione")
c("-o", "/dev/null", "--data-urlencode", "old=Economia", "--data-urlencode", "new=Finanza", B + "/admin/categorie/rinomina")  # modulo semplice, non multipart
ok("Finanza" in menu() and "Economia" not in menu(), f"rinominare una categoria aggiorna il menu ({menu()})")
ok("Non ci sono ancora articoli" in page("category/finanza/index.html"), "e la sua pagina ha il nuovo indirizzo")
c("-o", "/dev/null", "--data-urlencode", "name=Cronaca", "--data-urlencode", "to=", B + "/admin/categorie/elimina")
ok("Cronaca" not in menu() and "Contatti | /contatti/" in menu(), f"eliminarla la toglie dal menu, il resto resta ({menu()})")
print("== Impostazioni")
s = sqlite3.connect(D + "/presstatic.db"); keys = dict(s.execute("SELECT key, value FROM settings").fetchall())
c("-o", "/dev/null", "--form-string", "site_name=Prova", "--form-string", "base_url=https://www.prova.it", "--form-string", "theme=classico",
  "--form-string", "menu=Politica\nFinanza\nSport\npolitica\nContatti | /contatti/", B + "/admin/settings")
ok("Sport" in cats() and "politica" not in cats(), f"una sezione nuova nel menu diventa categoria, senza doppioni per le maiuscole ({cats()})")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
