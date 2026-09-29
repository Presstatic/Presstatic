# Page builder per le pagine singole e widget avanzati (Chromium): modello «Chi siamo», schede e fisarmonica senza
# JavaScript, pubblicazione della sola pagina. Usa il sito di prova in /tmp/shots.
import asyncio, sqlite3, re, subprocess
from playwright.async_api import async_playwright
A, OUT = "http://127.0.0.1:8211", "/mnt/user-data/outputs/pannello"
res = []
def ok(c, l): res.append(("  OK  " if c else "  NO  ") + l)
async def main():
    subprocess.run(["curl", "-s", "-o", "/dev/null", "-b", "/tmp/shots.jar", "-H", "Origin: " + A, "--form-string", "title=Chi siamo", "--form-string", "slug=chi-siamo", "--form-string", "body=<p>Testo scritto nell'editor.</p>", "--form-string", "status=published", "--form-string", "kind=page", A + "/admin/edit/0?k=page"])
    with sqlite3.connect("/tmp/shots/presstatic.db") as c: pid, kind, slug = c.execute("SELECT id, kind, slug FROM posts WHERE kind = 'page' AND title = 'Chi siamo' AND status = 'published' ORDER BY id DESC LIMIT 1").fetchone()
    async with async_playwright() as p:
        b = await p.chromium.launch(); ctx = await b.new_context(viewport={"width": 1600, "height": 950}, device_scale_factor=1.25, locale="it-IT"); pg = await ctx.new_page()
        errs = []; pg.on("pageerror", lambda e: errs.append(str(e))); pg.on("dialog", lambda d: asyncio.ensure_future(d.accept()))
        await pg.goto(A + "/admin/login"); await pg.fill("input[name=email]", "andrea@example.com"); await pg.fill("input[name=password]", "password-lunga-123"); await pg.click("form button"); await pg.wait_for_load_state("networkidle")
        await pg.goto(A + f"/admin/edit/{pid}"); await pg.wait_for_timeout(400)
        ok(kind == "page" and await pg.locator(f"a[href='/admin/builder?pagina=p{pid}']").count() == 1, "nell'editor della pagina c'è «Costruisci con il page builder»")
        await pg.goto(A + f"/admin/builder?pagina=p{pid}"); await pg.wait_for_timeout(1500)
        fr = pg.frame_locator("#pb-frame")
        ok(await fr.locator("#pb-root h1").inner_text() == "Chi siamo" and await fr.locator("[data-pb-type=counter]").count() == 3, "il modello «Chi siamo» parte con il titolo vero della pagina, testo, foto, numeri e domande")
        for t in ["tabs", "carousel", "quote", "card"]:
            await pg.click(f"#pb-palette button[data-type={t}]"); await pg.wait_for_timeout(600)
        ok(all([await fr.locator(f"[data-pb-type={t}]").count() >= 1 for t in ["tabs", "accordion", "carousel", "quote", "card", "counter"]]), "i widget avanzati si aggiungono e compaiono nell'anteprima")
        await fr.locator("[data-pb-type=tabs] .pb-tabs label").nth(1).click(); await pg.wait_for_timeout(300)
        vis = await fr.locator("[data-pb-type=tabs] .pb-panel").nth(1).is_visible()
        ok(vis, "schede: cliccando la seconda linguetta si vede il secondo testo (solo CSS)")
        await pg.screenshot(path=f"{OUT}/38-builder-pagina.png")
        await pg.click("#pb-publish"); await pg.wait_for_timeout(2500)
        html = open(f"/tmp/shots/public/{slug}/index.html", encoding="utf-8").read()
        ok('id="pb-page"' in html and "Testo scritto nell'editor." not in html and html.count("<h1") == 1, "pubblicata: la pagina mostra la composizione (un solo H1) al posto del testo")
        ok("data-pb" not in html and "<script" not in html.split('id="pb-page"')[1].split("</main>")[0], "nel contenuto costruito non c'è JavaScript né niente dell'editor")
        sp = await (await b.new_context(viewport={"width": 1280, "height": 900})).new_page()
        await sp.goto(f"http://127.0.0.1:8412/{slug}/"); await sp.wait_for_timeout(400)
        await sp.locator(".pb-acc summary").nth(1).click(); await sp.wait_for_timeout(200)
        ok(await sp.locator(".pb-acc details").nth(1).get_attribute("open") is not None, "sul sito la fisarmonica si apre al clic (elemento HTML nativo)")
        await sp.locator(".pb-tabs label").nth(1).click(); await sp.wait_for_timeout(200)
        ok(await sp.locator(".pb-panel").nth(1).is_visible() and not await sp.locator(".pb-panel").nth(0).is_visible(), "sul sito le schede cambiano al clic")
        await sp.keyboard.press("ArrowLeft"); await sp.wait_for_timeout(200)
        ok(await sp.locator(".pb-panel").nth(0).is_visible(), "e anche con le frecce della tastiera")
        await sp.screenshot(path=f"{OUT}/39-pagina-chi-siamo.png", full_page=True)
        ok(not errs, "nessun errore JavaScript" + (f": {errs}" if errs else ""))
        await b.close()
asyncio.run(main())
print("\n".join(res))
