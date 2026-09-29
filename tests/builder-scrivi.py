# Builder: scrivere titoli e testi nell'anteprima; articoli veri nei widget anche con sezione vuota.
import subprocess, sys, sqlite3, asyncio
from playwright.async_api import async_playwright
B, D = sys.argv[1], sys.argv[2]; O = "/mnt/user-data/outputs/screenshot-temi"
fails = 0
def ok(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l, flush=True)
J = D + "/j"; open(J, "w").close()
def c(*a): return subprocess.run(["curl", "-s", "-m", "60", "-b", J, "-c", J, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
c("-o", "/dev/null", "--data-urlencode", "email=a@x.it", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
for t in ("Primo articolo di prova", "Secondo articolo di prova"):
    c("-o", "/dev/null", "--form-string", f"title={t}", "--form-string", "body=<p>Testo.</p>", "--form-string", "category=Cronaca", "--form-string", "status=published", B + "/admin/edit/0")
with sqlite3.connect(D + "/presstatic.db") as d: d.execute("INSERT OR IGNORE INTO categories(name) VALUES ('Sport')")
async def main():
    async with async_playwright() as p:
        b = await p.chromium.launch(); pg = await (await b.new_context(viewport={"width": 1920, "height": 953})).new_page(); errs = []
        pg.on("pageerror", lambda e: errs.append(str(e)[:80]))
        await pg.goto(B + "/admin/login"); await pg.fill("input[name=email]", "a@x.it"); await pg.fill("input[name=password]", "password-lunga-123"); await pg.click("form button"); await pg.wait_for_load_state("networkidle")
        await pg.goto(B + "/admin/builder?pagina=home"); await pg.wait_for_timeout(2000); fr = pg.frame_locator("#pb-frame")
        print("== Articoli veri nell'anteprima")
        await pg.click("#pb-palette button[data-type=hero]"); await pg.wait_for_timeout(600)
        hero = await fr.locator(".pbf-sel").inner_text()
        ok("Secondo articolo di prova" in hero and await fr.locator(".pbf-sel .pb-skel").count() == 0, "l'apertura mostra l'articolo vero pubblicato")
        await pg.click("#pb-palette button[data-type=posts_grid]"); await pg.wait_for_timeout(600)
        await pg.select_option("#pb-props select:has(option[value='Sport'])", "Sport"); await pg.wait_for_timeout(800)
        note = await fr.locator(".pbf-sel .pb-note").inner_text() if await fr.locator(".pbf-sel .pb-note").count() else ""
        arts = await fr.locator(".pbf-sel article").count()
        ok("in «Sport» non ci sono ancora articoli" in note and arts >= 1, f"griglia su una sezione vuota: mostra gli ultimi articoli con la nota ({arts} articoli)")
        print("== Scrivere nell'anteprima")
        await pg.click("#pb-palette button[data-type=heading]"); await pg.wait_for_timeout(600)
        h = fr.locator(".pbf-sel .pb-h"); await h.dblclick(); await pg.wait_for_timeout(200)
        await pg.keyboard.press("Control+A"); await pg.keyboard.type("Titolo scritto nell'anteprima"); await pg.keyboard.press("Enter"); await pg.wait_for_timeout(800)
        txt = await fr.locator(".pbf-sel .pb-h").inner_text(); panel_val = await pg.evaluate("[...document.querySelectorAll('#pb-props input, #pb-props textarea')].map(x => x.value).join('|')")
        ok(txt == "Titolo scritto nell'anteprima" and "Titolo scritto nell'anteprima" in panel_val and "non salvate" in await pg.inner_text("#pb-status"), "doppio clic su un titolo: si scrive lì, e l'impostazione a destra si aggiorna")
        await fr.locator(".pbf-sel .pb-h").dblclick(); await pg.keyboard.press("Control+A"); await pg.keyboard.type("DA ANNULLARE"); await pg.keyboard.press("Escape"); await pg.wait_for_timeout(800)
        ok(await fr.locator(".pbf-sel .pb-h").inner_text() == "Titolo scritto nell'anteprima", "Esc annulla quello che stavi scrivendo")
        await fr.locator(".pbf-sel .pb-h").click(); await pg.wait_for_timeout(200)
        ok(await fr.locator(".pbf-sel .pb-h[contenteditable]").count() == 1, "un clic su un widget già scelto basta per scrivere")
        await pg.keyboard.press("Escape"); await pg.wait_for_timeout(500)
        await pg.click("#pb-palette button[data-type=text]"); await pg.wait_for_timeout(600)
        await fr.locator(".pbf-sel .pb-t").dblclick(); await pg.keyboard.press("Control+A")
        await pg.keyboard.type("Primo paragrafo"); await pg.keyboard.press("Enter"); await pg.keyboard.press("Enter"); await pg.keyboard.type("Secondo paragrafo")
        await fr.locator("body").click(position={"x": 5, "y": 5}); await pg.wait_for_timeout(800)
        paras = await fr.locator(".pb-t").last.locator("p").all_inner_texts()
        ok(paras == ["Primo paragrafo", "Secondo paragrafo"], f"nel testo Invio va a capo e i paragrafi restano separati ({paras})")
        await pg.screenshot(path=f"{O}/builder-scrivi.png")
        await pg.click("#pb-save"); await pg.wait_for_timeout(800); await pg.reload(); await pg.wait_for_timeout(2000)
        ok(await pg.frame_locator("#pb-frame").get_by_text("Titolo scritto nell'anteprima").count() == 1, "dopo «Salva la bozza» e il ricaricamento il titolo scritto c'è ancora")
        ok(not errs, f"nessun errore JavaScript ({errs[:2]})")
        await b.close()
asyncio.run(main())
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
