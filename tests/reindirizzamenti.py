# Reindirizzamenti dei vecchi indirizzi e ripristino dalla cronologia.
# Uso: python3 tests/reindirizzamenti.py http://127.0.0.1:8200 /percorso/del/sito
import subprocess, sys, re, sqlite3, os, json, urllib.parse
B, DIR = sys.argv[1], sys.argv[2]
PUB = f"{DIR}/public"
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
def curl(jar, *a): return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
def form(jar, path, fields):
    args = []
    for k, v in fields.items(): args += ["--form-string", f"{k}={v}"]
    return curl(jar, "-o", "/dev/null", "-w", "%{redirect_url}", *args, B + path)
def urlenc(jar, path, fields):
    args = []
    for k, v in fields.items(): args += ["--data-urlencode", f"{k}={v}"]
    return curl(jar, "-o", "/dev/null", "-w", "%{redirect_url}", *args, "-X", "POST", B + path)
def msg(r): return urllib.parse.unquote_plus(r.split("msg=", 1)[-1]) if "msg=" in r else r
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def page(slug):
    f = f"{PUB}/{slug}/index.html"
    return open(f, encoding="utf-8").read() if os.path.isfile(f) else None
def target(slug):
    h = page(slug) or ""
    m = re.search(r'http-equiv="refresh" content="0; url=([^"]+)"', h)
    return m.group(1) if m else None
def save(jar, pid, title, slug, status="published", extra=None):
    r = form(jar, f"/admin/edit/{pid}", {"title": title, "slug": slug, "body": f"<p>{title}</p>", "category": "Cronaca", "status": status, **(extra or {})})
    return int(re.search(r"/admin/edit/(\d+)", r).group(1)), msg(r)
base = json.loads("{}")
with db() as c: SITE = c.execute("SELECT value FROM settings WHERE key = 'base_url'").fetchone()
SITE = (SITE[0] if SITE else "").rstrip("/")

A = "/tmp/rd-a.jar"; open(A, "w").close()
curl(A, "-o", "/dev/null", "--data-urlencode", "email=andrea@example.com", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")

print("== 1. Cambio di indirizzo di un articolo online")
a, _ = save(A, 0, "Ponte chiuso", "ponte-chiuso")
save(A, a, "Ponte chiuso", "ponte-chiuso-per-lavori")
ok(page("ponte-chiuso-per-lavori") and "Ponte chiuso" in page("ponte-chiuso-per-lavori"), "il nuovo indirizzo ha l'articolo")
ok(target("ponte-chiuso") == f"{SITE}/ponte-chiuso-per-lavori/", "il vecchio indirizzo porta subito al nuovo")
ok('rel="canonical" href="' + f"{SITE}/ponte-chiuso-per-lavori/" in (page("ponte-chiuso") or ""), "con il nuovo indirizzo come canonico")
save(A, a, "Ponte chiuso", "ponte-chiuso-fino-a-maggio")
ok(target("ponte-chiuso") == f"{SITE}/ponte-chiuso-fino-a-maggio/" and target("ponte-chiuso-per-lavori") == f"{SITE}/ponte-chiuso-fino-a-maggio/",
   "cambiato di nuovo: tutti i vecchi indirizzi portano all'ultimo, senza catene")

print("== 2. Un nuovo articolo prende un vecchio indirizzo")
n, _ = save(A, 0, "Nuovo articolo sul ponte", "ponte-chiuso")
ok(target("ponte-chiuso") is None and "Nuovo articolo sul ponte" in (page("ponte-chiuso") or ""), "l'indirizzo è del nuovo articolo, non reindirizza più")
with db() as c: ok(c.execute("SELECT COUNT(*) FROM redirects WHERE slug = 'ponte-chiuso'").fetchone()[0] == 0, "e il reindirizzamento è stato tolto")

print("== 3. Articolo ritirato e ripubblicato")
save(A, a, "Ponte chiuso", "ponte-chiuso-fino-a-maggio", status="draft")
ok(page("ponte-chiuso-per-lavori") is None, "ritirato dal sito: il vecchio indirizzo non porta più a una pagina che non c'è")
save(A, a, "Ponte chiuso", "ponte-chiuso-fino-a-maggio")
ok(target("ponte-chiuso-per-lavori") == f"{SITE}/ponte-chiuso-fino-a-maggio/", "ripubblicato: il reindirizzamento torna")

print("== 4. Eliminazione con reindirizzamento alla home")
h, _ = save(A, 0, "Da eliminare verso la home", "verso-la-home")
save(A, h, "Da eliminare verso la home", "verso-la-home-2")
m = msg(urlenc(A, f"/admin/delete/{h}", {"redirect": "home"}))
ok("portato alla home" in m, "il messaggio dice dove finiscono i lettori")
ok(target("verso-la-home-2") == f"{SITE}/" and target("verso-la-home") == f"{SITE}/", "l'indirizzo e il suo vecchio indirizzo portano alla home")

print("== 5. Eliminazione con reindirizzamento a un altro articolo")
d, _ = save(A, 0, "Vecchia notizia", "vecchia-notizia")
save(A, d, "Vecchia notizia", "vecchia-notizia-aggiornata")
m = msg(urlenc(A, f"/admin/delete/{d}", {"redirect": "post", "target_id": str(a)}))
ok("Ponte chiuso" in m, "il messaggio nomina l'articolo di destinazione")
ok(target("vecchia-notizia-aggiornata") == f"{SITE}/ponte-chiuso-fino-a-maggio/" and target("vecchia-notizia") == f"{SITE}/ponte-chiuso-fino-a-maggio/",
   "l'indirizzo e il suo vecchio indirizzo portano all'articolo scelto")
save(A, a, "Ponte chiuso", "ponte-riaperto")
ok(target("vecchia-notizia") == f"{SITE}/ponte-riaperto/", "se poi l'articolo scelto cambia indirizzo, i reindirizzamenti lo seguono")

print("== 6. Eliminazione senza reindirizzamento, e casi da rifiutare")
x, _ = save(A, 0, "Senza seguito", "senza-seguito")
urlenc(A, f"/admin/delete/{x}", {"redirect": "none"})
ok(page("senza-seguito") is None, "nessun reindirizzamento: l'indirizzo risponde «pagina non trovata»")
y, _ = save(A, 0, "Bozza da eliminare", "bozza-da-eliminare", status="draft")
urlenc(A, f"/admin/delete/{y}", {"redirect": "home"})
with db() as c: ok(c.execute("SELECT COUNT(*) FROM redirects WHERE slug = 'bozza-da-eliminare'").fetchone()[0] == 0, "una bozza mai uscita non crea reindirizzamenti")
z, _ = save(A, 0, "Resta online", "resta-online")
dr, _ = save(A, 0, "Solo bozza", "solo-bozza", status="draft")
m = msg(urlenc(A, f"/admin/delete/{z}", {"redirect": "post", "target_id": str(dr)}))
ok("scegli un articolo o una pagina pubblicati" in m and page("resta-online"), "destinazione non online: rifiutato, l'articolo resta")
m = msg(urlenc(A, f"/admin/delete/{z}", {"redirect": "post"}))
ok("scegli l'articolo o la pagina" in m and page("resta-online"), "destinazione mancante: rifiutato")

print("== 7. Rigenerazione completa")
os.remove(f"{PUB}/vecchia-notizia/index.html")
curl(A, "-o", "/dev/null", "-X", "POST", B + "/admin/rebuild")
ok(target("vecchia-notizia") == f"{SITE}/ponte-riaperto/" and target("verso-la-home") == f"{SITE}/", "i reindirizzamenti vengono riscritti, verso articoli e verso la home")

print("== 8. Scelta della destinazione nel pannello")
sug = json.loads(curl(A, B + f"/admin/suggest?q=ponte&except={n}"))
ok(any(s["id"] == a for s in sug) and all(s["id"] != n for s in sug), "la ricerca propone gli articoli online e non quello che si elimina")
ed = curl(A, B + f"/admin/edit/{a}")
ok('id="deldlg"' in ed and 'value="home"' in ed and 'value="none"' in ed, "per un articolo online, «Elimina» apre la finestra con le tre scelte")
ok('id="deldlg"' not in curl(A, B + f"/admin/edit/{dr}"), "per una bozza resta la semplice conferma")

print("== 9. Ripristino dalla cronologia")
r_id, _ = save(A, 0, "Testo da ripristinare", "testo-da-ripristinare", extra={"link_keywords": "ponte sul canale"})
with db() as c: first = c.execute("SELECT id FROM revisions WHERE post_id = ? ORDER BY id LIMIT 1", (r_id,)).fetchone()[0]
form(A, f"/admin/edit/{r_id}", {"title": "Testo da ripristinare", "slug": "testo-da-ripristinare", "body": "<p>Seconda stesura</p>", "category": "Sport", "status": "published", "link_keywords": "ponte sul canale"})
curl(A, "-o", "/dev/null", "-X", "POST", B + f"/admin/revision/{first}")
with db() as c: body, cat, kw = c.execute("SELECT body, category, link_keywords FROM posts WHERE id = ?", (r_id,)).fetchone()
ok("Testo da ripristinare" in body, "il testo torna quello della versione scelta")
ok(cat == "Sport" and kw == "ponte sul canale", "categoria e parole chiave restano quelle attuali (prima il ripristino cancellava le parole chiave)")

print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
