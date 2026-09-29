# Page builder: sezioni salvate («I miei blocchi») e riusate in un'altra pagina (Chromium). Usa il sito di prova in /tmp/shots.
import asyncio, sqlite3, json
from playwright.async_api import async_playwright
A = "http://127.0.0.1:8211"
res = []
def ok(c, l): res.append(("  OK  " if c else "  NO  ") + l)
async def main():
    q = "SELECT id FROM posts WHERE kind='page' AND title='Chi siamo' AND status='published' ORDER BY id DESC LIMIT 1"
    row = sqlite3.connect("/tmp/shots/presstatic.db").execute(q).fetchone()
    if not row:  # prova autonoma: se la pagina non c'è (per esempio dopo una pulizia), la crea
        import subprocess
        subprocess.run(["curl", "-s", "-o", "/dev/null", "-b", "/tmp/shots.jar", "-H", "Origin: " + A, "--form-string", "title=Chi siamo", "--form-string", "slug=chi-siamo-prova", "--form-string", "body=<p>Testo.</p>", "--form-string", "status=published", "--form-string", "kind=page", A + "/admin/edit/0?k=page"])
        row = sqlite3.connect("/tmp/shots/presstatic.db").execute(q).fetchone()
    pid = row[0]
    async with async_playwright() as p:
        b = await p.chromium.launch(); pg = await (await b.new_context(viewport={"width": 1600, "height": 950}, locale="it-IT")).new_page()
        errs = []; pg.on("pageerror", lambda e: errs.append(str(e)))
        pg.on("dialog", lambda d: asyncio.ensure_future(d.accept("Griglia delle notizie") if d.type == "prompt" else d.accept()))
        await pg.goto(A + "/admin/login"); await pg.fill("input[name=email]", "andrea@example.com"); await pg.fill("input[name=password]", "password-lunga-123"); await pg.click("form button"); await pg.wait_for_load_state("networkidle")
        await pg.goto(A + "/admin/builder"); await pg.wait_for_timeout(1500)
        fr = pg.frame_locator("#pb-frame")
        ok("Nessun blocco ancora" in await pg.inner_text("#pb-blocks"), "all'inizio «I miei blocchi» è vuoto, con la spiegazione")
        await fr.locator("[data-pb=section]").nth(1).click(position={"x": 4, "y": 4}); await pg.wait_for_timeout(300)
        await fr.locator(".pbf-bar button[data-op=savesec]").click(); await pg.wait_for_timeout(800)
        ok("Griglia delle notizie" in await pg.inner_text("#pb-blocks"), "★ nella barra blu salva la sezione con il nome scelto")
        with sqlite3.connect("/tmp/shots/presstatic.db") as c: saved = json.loads(c.execute("SELECT data FROM layout_blocks").fetchone()[0])
        ok(any(w["type"] == "posts_grid" for col in saved["columns"] for w in col["widgets"]), "nel blocco salvato c'è la sezione con i suoi widget")
        await pg.goto(A + f"/admin/builder?pagina=p{pid}"); await pg.wait_for_timeout(1500)
        before = await fr.locator("[data-pb=section]").count()
        await pg.click("#pb-blocks .pb-block button:first-child"); await pg.wait_for_timeout(800)
        ok(await fr.locator("[data-pb=section]").count() == before + 1 and await fr.locator("[data-pb-type=posts_grid]").count() >= 1, "in un'altra pagina, un clic sul blocco inserisce la sezione")
        ids = await pg.frame(url=lambda u: "builder/frame" in u).evaluate("[...document.querySelectorAll('[data-pb=section]')].map(s => s.dataset.pbId)")
        ok(saved["id"] not in ids, "è una copia indipendente (identificativi nuovi): modificarla non tocca l'originale")
        await pg.click("#pb-blocks .pb-block .del"); await pg.wait_for_timeout(600)
        with sqlite3.connect("/tmp/shots/presstatic.db") as c: n = c.execute("SELECT COUNT(*) FROM layout_blocks").fetchone()[0]
        ok(n == 0 and "Nessun blocco ancora" in await pg.inner_text("#pb-blocks") and await fr.locator("[data-pb-type=posts_grid]").count() >= 1, "eliminando il blocco salvato, la pagina in cui è inserito non cambia")
        ok(not errs, "nessun errore JavaScript" + (f": {errs}" if errs else ""))
        await b.close()
asyncio.run(main())
print("\n".join(res))
