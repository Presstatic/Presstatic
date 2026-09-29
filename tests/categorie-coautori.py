# Più categorie per articolo, sottocategorie, gestione delle categorie, coautori.
# Uso: python3 tests/categorie-coautori.py http://127.0.0.1:8207 /percorso/del/sito
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
def urlenc(jar, path, fields):
    args = []
    for k, v in fields: args += ["--data-urlencode", f"{k}={v}"]
    return curl(jar, "-o", "/dev/null", "-w", "%{redirect_url}", *args, B + path)
def msg(r): return urllib.parse.unquote_plus(r.split("msg=", 1)[-1]) if "msg=" in r else r
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def page(path): f = f"{PUB}/{path.strip('/')}/index.html"; return open(f, encoding="utf-8").read() if os.path.isfile(f) else ""
def ld(html): return [json.loads(x) for x in re.findall(r'<script type="application/ld\+json">(.*?)</script>', html, re.S)]
def login(jar, email):
    open(jar, "w").close(); curl(jar, "-o", "/dev/null", "--data-urlencode", f"email={email}", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
A, C = "/tmp/cc-a.jar", "/tmp/cc-c.jar"
login(A, "andrea@example.com")
form(A, "/admin/users/0", [("name", "Carla Neri"), ("email", "carla@example.com"), ("password", "password-lunga-123"), ("role", "author"), ("bio", "Cronista sportiva.")])
with db() as c: carla = c.execute("SELECT id, slug FROM users WHERE email = 'carla@example.com'").fetchone()
login(C, "carla@example.com")

print("== 1. Categorie e sottocategorie")
ok("salvata" in msg(urlenc(A, "/admin/categorie", [("name", "Sport"), ("parent", ""), ("description", "")])), "creata la sezione Sport")
urlenc(A, "/admin/categorie", [("name", "Calcio"), ("parent", "Sport"), ("description", "Serie A, B e calcio dilettanti della provincia.")])
r = form(A, "/admin/edit/0", [("title", "Derby vinto al novantesimo"), ("slug", "derby-vinto"), ("body", "<p>Cronaca.</p>"), ("category", "Calcio"), ("categories", "Cronaca"), ("coauthors", str(carla[0])), ("status", "published")])
pid = int(re.search(r"/admin/edit/(\d+)", r).group(1))
calcio, sport, cronaca = page("category/calcio"), page("category/sport"), page("category/cronaca")
ok("Derby vinto" in calcio and "Derby vinto" in sport and "Derby vinto" in cronaca, "l'articolo compare in Calcio, nella madre Sport e nella categoria aggiuntiva Cronaca")
ok("Serie A, B e calcio dilettanti" in calcio and 'href="http://127.0.0.1:8080/category/sport/">Sport</a>' in calcio, "la pagina Calcio ha la descrizione e il collegamento alla sezione madre")
ok('class="subcats"' in sport and ">Calcio</a>" in sport.split('class="subcats"', 1)[1][:400], "la pagina Sport mostra la sottosezione Calcio")
art = page("derby-vinto")
crumbs = [x for x in ld(art) if isinstance(x, dict) and x.get("@type") == "BreadcrumbList"] or [y for x in ld(art) if isinstance(x, list) for y in x if y.get("@type") == "BreadcrumbList"]
names = [i["name"] for i in crumbs[0]["itemListElement"]] if crumbs else []
ok(names[1:3] == ["Sport", "Calcio"], f"briciole di pane per Google: Home › Sport › Calcio › articolo ({' › '.join(names)})")

print("== 2. Coautori")
ok(re.search(r"Andrea Admin</strong></a>\s*e\s*<a href=\"[^\"]+/autori/" + carla[1] + r"/\"><strong>Carla Neri", art) is not None, "la firma dice «Andrea Admin e Carla Neri», con i link alle pagine autore")
art_ld = [y for x in ld(art) for y in (x if isinstance(x, list) else [x]) if y.get("@type") in ("NewsArticle", "Article")]
ok(art_ld and isinstance(art_ld[0]["author"], list) and len(art_ld[0]["author"]) == 2, "nei dati per Google l'articolo ha due autori")
cp = page(f"autori/{carla[1]}")
ok("Derby vinto" in cp and "Cronista sportiva." in cp, "la pagina autore di Carla elenca l'articolo, con la sua biografia")
r = form(C, "/admin/edit/0", [("title", "Bozza di Carla"), ("body", "<p>x</p>"), ("coauthors", "1"), ("status", "pending")])
with db() as c: co = c.execute("SELECT coauthors FROM posts WHERE title = 'Bozza di Carla'").fetchone()[0]
ok(co == "", "un autore non può aggiungere coautori da solo")

print("== 3. Rinomina, elimina, niente cerchi")
ok("rinominata" in msg(urlenc(A, "/admin/categorie/rinomina", [("old", "Calcio"), ("new", "Pallone")])), "Calcio rinominata in Pallone")
with db() as c: ok(c.execute("SELECT category FROM posts WHERE id = ?", (pid,)).fetchone()[0] == "Pallone", "l'articolo ora è in Pallone")
ok("Derby vinto" in page("category/pallone") and "category/pallone/" in page("category/calcio") and "http-equiv" in page("category/calcio"), "la nuova pagina c'è e il vecchio indirizzo /category/calcio/ porta lì")
ok("sottocategoria" in msg(urlenc(A, "/admin/categorie", [("name", "Sport"), ("parent", "Pallone"), ("description", "")])), "Sport non può finire dentro la sua sottocategoria Pallone")
ok("eliminata" in msg(urlenc(A, "/admin/categorie/elimina", [("name", "Cronaca"), ("to", "Sport")])), "Cronaca eliminata, con i suoi articoli spostati in Sport")
with db() as c: extra = c.execute("SELECT categories FROM posts WHERE id = ?", (pid,)).fetchone()[0]
ok(extra == "" and "category/sport/" in page("category/cronaca"), "tolta dalle categorie dell'articolo; il vecchio indirizzo porta a Sport")
cats = curl(A, B + "/admin/categorie")
ok("Pallone" in cats and "Sport" in cats and "Cronaca" not in cats.split('<ul class="ctree">', 1)[-1].split("</ul>", 1)[0], "la pagina Categorie mostra l'albero aggiornato")

print("== 4. Ripristino e nel browser")
with db() as c: rev = c.execute("SELECT id FROM revisions WHERE post_id = ? ORDER BY id LIMIT 1", (pid,)).fetchone()[0]
curl(A, "-o", "/dev/null", "-X", "POST", B + f"/admin/revision/{rev}")
with db() as c: co2 = c.execute("SELECT coauthors, category FROM posts WHERE id = ?", (pid,)).fetchone()
ok(co2 == (str(carla[0]), "Pallone"), "il ripristino di una versione mantiene coautori e categoria attuali")
from playwright.async_api import async_playwright
async def browser():
    async with async_playwright() as p:
        b = await p.chromium.launch(); ctx = await b.new_context(viewport={"width": 1440, "height": 900}, device_scale_factor=1.5, locale="it-IT"); pg = await ctx.new_page()
        errs = []; pg.on("pageerror", lambda e: errs.append(str(e)))
        await pg.goto(B + "/admin/login"); await pg.fill("input[name=email]", "andrea@example.com"); await pg.fill("input[name=password]", "password-lunga-123"); await pg.click("form button"); await pg.wait_for_load_state("networkidle")
        await pg.goto(B + f"/admin/edit/{pid}"); await pg.wait_for_timeout(500)
        await pg.locator("#catslist input[value=Sport]").check(); await pg.locator(f"#colist input[value='{carla[0]}']").check()
        await pg.click("button[name=status][value=published]"); await pg.wait_for_load_state("networkidle")
        with db() as c: v = c.execute("SELECT categories, coauthors FROM posts WHERE id = ?", (pid,)).fetchone()
        ok(v[0] == "Sport" and v[1] == str(carla[0]), "nell'editor si scelgono altre categorie e coautori con le caselle")
        ok(await pg.locator("#colist input[value='1']").count() == 0, "l'autore dell'articolo non compare tra i possibili coautori")
        await pg.goto(B + "/admin/categorie"); await pg.wait_for_timeout(400); await pg.locator(".ctree summary").nth(1).click(); await pg.wait_for_timeout(300)
        await pg.screenshot(path="/mnt/user-data/outputs/pannello/31-categorie.png")
        ok(not errs, "nessun errore JavaScript" + (f": {errs}" if errs else ""))
        await b.close()
asyncio.run(browser())
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
