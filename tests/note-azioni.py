# Note della redazione e azioni in blocco nell'elenco degli articoli.
# Uso: python3 tests/note-azioni.py http://127.0.0.1:8206 /percorso/del/sito
import subprocess, sys, re, json, sqlite3, os, urllib.parse, asyncio
B, DIR = sys.argv[1], sys.argv[2]
PUB = f"{DIR}/public"
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
def curl(jar, *a): return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
def form(jar, path, fields):
    args = []
    for k, v in fields: args += ["--form-string", f"{k}={v}"]
    return curl(jar, "-o", "/dev/null", "-w", "%{redirect_url}", *args, B + path)
def urlenc(jar, path, fields, accept=None):
    args = (["-H", f"Accept: {accept}"] if accept else [])
    for k, v in fields: args += ["--data-urlencode", f"{k}={v}"]
    return curl(jar, "-w", "\n%{redirect_url}", *args, B + path)
def msg(r): return urllib.parse.unquote_plus(r.split("msg=", 1)[-1]) if "msg=" in r else r
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def page(slug): f = f"{PUB}/{slug}/index.html"; return open(f, encoding="utf-8").read() if os.path.isfile(f) else ""
def login(jar, email):
    open(jar, "w").close(); curl(jar, "-o", "/dev/null", "--data-urlencode", f"email={email}", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
A, U = "/tmp/na-a.jar", "/tmp/na-u.jar"
login(A, "andrea@example.com")
form(A, "/admin/users/0", [("name", "Luca Autore"), ("email", "luca@example.com"), ("password", "password-lunga-123"), ("role", "author")])
login(U, "luca@example.com")

print("== 1. Note della redazione")
r = form(U, "/admin/edit/0", [("title", "Bozza di Luca"), ("body", "<p>Testo.</p>"), ("status", "pending")])
pid = int(re.search(r"/admin/edit/(\d+)", r).group(1))
j = json.loads(urlenc(A, f"/admin/edit/{pid}/note", [("body", "Manca la fonte del secondo paragrafo.")], "application/json").split("\n")[0])
ok(j.get("ok"), "il redattore aggiunge una nota (senza ricaricare la pagina)")
ed = curl(U, B + f"/admin/edit/{pid}")
ok("Manca la fonte del secondo paragrafo." in ed and "Andrea Admin" in ed, "l'autore la vede nell'editor, con chi l'ha scritta")
ok("1 nota" in curl(U, B + "/admin"), "nell'elenco l'articolo mostra quante note ha")
urlenc(U, f"/admin/edit/{pid}/note", [("body", "Aggiunta, grazie!")], "application/json")
with db() as c: nid_admin, nid_user = [r[0] for r in c.execute("SELECT id FROM post_notes WHERE post_id = ? ORDER BY id", (pid,))]
ok(curl(U, "-o", "/dev/null", "-w", "%{http_code}", "-X", "POST", B + f"/admin/note/{nid_admin}/delete") == "403", "l'autore non può eliminare la nota del redattore")
curl(U, "-o", "/dev/null", "-X", "POST", "-H", "Accept: application/json", B + f"/admin/note/{nid_user}/delete")
with db() as c: ok(c.execute("SELECT COUNT(*) FROM post_notes WHERE post_id = ?", (pid,)).fetchone()[0] == 1, "ma può eliminare la sua")
ok("Manca la fonte" not in page("bozza-di-luca") and "Manca la fonte" not in open(f"{PUB}/index.html", encoding="utf-8").read(), "le note non finiscono mai sul sito")
j = json.loads(urlenc(A, f"/admin/edit/{pid}/note", [("body", "")], "application/json").split("\n")[0])
ok("error" in j, "una nota vuota viene rifiutata")

print("== 2. Azioni in blocco")
ids = []
for i in range(5):
    r = form(A, "/admin/edit/0", [("title", f"Notizia in blocco {i}"), ("slug", f"blocco-{i}"), ("body", "<p>Testo.</p>"), ("category", "Cronaca"), ("status", "published")])
    ids.append(re.search(r"/admin/edit/(\d+)", r).group(1))
home = lambda: open(f"{PUB}/index.html", encoding="utf-8").read()
m = msg(urlenc(A, "/admin/bulk", [("ids", ids[0]), ("ids", ids[1]), ("action", "draft"), ("back", "/admin")]))
ok("2 articoli riportati in bozza" in m, "due articoli riportati in bozza insieme")
ok(not page("blocco-0") and not page("blocco-1") and "Notizia in blocco 0" not in home() and "Notizia in blocco 2" in home(), "le loro pagine spariscono e la home si aggiorna")
m = msg(urlenc(A, "/admin/bulk", [("ids", ids[0]), ("ids", ids[1]), ("action", "publish"), ("back", "/admin")]))
ok("pubblicati" in m and page("blocco-0") and "Notizia in blocco 0" in home(), "ripubblicati insieme: pagine e home tornano")
urlenc(A, "/admin/bulk", [("ids", i) for i in ids[:3]] + [("action", "category"), ("value", "Sport"), ("back", "/admin")])
sport, cronaca = page("category/sport"), page("category/cronaca")
ok(all(f"Notizia in blocco {i}" in sport for i in range(3)) and "Notizia in blocco 0" not in cronaca and "Notizia in blocco 4" in cronaca, "spostati in Sport: compaiono lì e spariscono da Cronaca")
urlenc(A, "/admin/bulk", [("ids", ids[3]), ("ids", ids[4]), ("action", "tag"), ("value", "Viabilità"), ("back", "/admin")])
with db() as c: tg = [r[0] for r in c.execute(f"SELECT tags FROM posts WHERE id IN ({ids[3]}, {ids[4]})")]
ok(tg == ["Viabilità", "Viabilità"] and "Notizia in blocco 3" in page("tag/viabilita"), "tag aggiunto a due articoli, con la sua pagina")
urlenc(A, "/admin/bulk", [("ids", ids[4]), ("action", "feature"), ("back", "/admin")])
with db() as c: ok(c.execute("SELECT featured FROM posts WHERE id = ?", (ids[4],)).fetchone()[0] == 1, "messo in evidenza in home")
m = msg(urlenc(A, "/admin/bulk", [("ids", ids[3]), ("ids", ids[4]), ("action", "delete"), ("back", "/admin")]))
ok("2 eliminati" in m and not page("blocco-3") and not page("blocco-4"), "due articoli eliminati insieme")
m = msg(urlenc(U, "/admin/bulk", [("ids", ids[0]), ("action", "draft"), ("back", "/admin")]))
ok("redattori e amministratori" in m and page("blocco-0"), "un autore non può usare le azioni in blocco")
ok("scrivi la categoria" in msg(urlenc(A, "/admin/bulk", [("ids", ids[0]), ("action", "category"), ("value", ""), ("back", "/admin")])), "senza il nome della categoria viene rifiutato")

print("== 3. Nel browser")
curl(U, "-o", "/dev/null", "-X", "POST", B + f"/admin/lock/{pid}/release")  # l'autore chiude l'editor: l'articolo non è più bloccato
from playwright.async_api import async_playwright
async def browser():
    async with async_playwright() as p:
        b = await p.chromium.launch(); ctx = await b.new_context(viewport={"width": 1440, "height": 900}, device_scale_factor=1.5, locale="it-IT"); pg = await ctx.new_page()
        errs = []; pg.on("pageerror", lambda e: errs.append(str(e)))
        await pg.goto(B + "/admin/login"); await pg.fill("input[name=email]", "andrea@example.com"); await pg.fill("input[name=password]", "password-lunga-123"); await pg.click("form button"); await pg.wait_for_load_state("networkidle")
        ok(await pg.locator("#bulkbar").is_hidden(), "la barra delle azioni resta nascosta finché non scegli articoli")
        boxes = pg.locator("input[name=ids]"); await boxes.nth(0).check(); await boxes.nth(1).check()
        ok(await pg.locator("#bulkbar").is_visible() and "2 scelti" in await pg.inner_text("#bulkn"), "scegliendo due articoli compare la barra con «2 scelti»")
        await pg.select_option("#bulkact", "category"); ok(await pg.locator("#bulkval").is_visible(), "scegliendo «Sposta in un'altra categoria» compare il campo per il nome")
        await pg.screenshot(path="/mnt/user-data/outputs/pannello/29-azioni-in-blocco.png")
        await pg.goto(B + f"/admin/edit/{pid}"); await pg.wait_for_timeout(500)
        await pg.fill("textarea[name=title]", "Titolo cambiato e non salvato")
        await pg.fill("#notebody", "Nota scritta dal browser"); await pg.click("#noteadd"); await pg.wait_for_timeout(600)
        ok("Nota scritta dal browser" in await pg.inner_text("#notelist") and await pg.input_value("textarea[name=title]") == "Titolo cambiato e non salvato", "la nota si aggiunge senza ricaricare: il titolo non ancora salvato resta")
        await pg.locator("#notes").scroll_into_view_if_needed(); await pg.screenshot(path="/mnt/user-data/outputs/pannello/30-note-redazione.png")
        ok(not errs, "nessun errore JavaScript" + (f": {errs}" if errs else ""))
        await b.close()
asyncio.run(browser())
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
