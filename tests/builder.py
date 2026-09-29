# Page builder nel browser (Chromium): anteprima, clic e trascinamento dei widget, impostazioni per dispositivo,
# annulla/ripeti, barra degli strumenti, colonne, bozza e pubblicazione. Usa il sito di prova in /tmp/shots.
import asyncio, json, sqlite3, re
from playwright.async_api import async_playwright
A, OUT = "http://127.0.0.1:8211", "/mnt/user-data/outputs/pannello"
res = []
def ok(c, l): res.append(("  OK  " if c else "  NO  ") + l)
def db(): return sqlite3.connect("/tmp/shots/presstatic.db")
async def main():
    async with async_playwright() as p:
        b = await p.chromium.launch(); ctx = await b.new_context(viewport={"width": 1600, "height": 950}, device_scale_factor=1.25, locale="it-IT"); pg = await ctx.new_page()
        errs = []; pg.on("pageerror", lambda e: errs.append("editor: " + str(e))); pg.on("dialog", lambda d: asyncio.ensure_future(d.accept()))
        await pg.goto(A + "/admin/login"); await pg.fill("input[name=email]", "andrea@example.com"); await pg.fill("input[name=password]", "password-lunga-123"); await pg.click("form button"); await pg.wait_for_load_state("networkidle")
        await pg.goto(A + "/admin/builder"); await pg.wait_for_timeout(1500)
        fr = pg.frame_locator("#pb-frame"); frame = pg.frame(url=re.compile(".*/admin/builder/frame"))
        frame.on("pageerror", lambda e: errs.append("anteprima: " + str(e))) if hasattr(frame, "on") else None
        ok(await fr.locator("#pb-root section.pb-s").count() == 3 and await fr.locator(".pb-hero").count() == 1, "l'anteprima mostra la home del modello «Giornale», nel tema del sito")
        await pg.screenshot(path=f"{OUT}/33-builder.png")
        # clic su un widget nell'anteprima: il pannello a destra mostra le sue impostazioni
        await fr.locator("[data-pb-type=posts_grid]").first.click(); await pg.wait_for_timeout(300)
        ok(await pg.inner_text("#pb-props-title") == "Griglia di articoli" and await pg.locator("#pb-props select").count() >= 1, "cliccando la griglia nell'anteprima, a destra compaiono le sue impostazioni")
        # widget aggiunto con un clic (dopo quello scelto) e trascinato nell'anteprima (evento di rilascio vero dentro l'anteprima)
        await pg.click("#pb-palette button[data-type=heading]"); await pg.wait_for_timeout(700)
        ok(await fr.locator("[data-pb-type=heading]").count() == 1, "un clic su «Titolo» lo aggiunge dopo l'elemento scelto")
        col = await frame.evaluate("document.querySelectorAll('[data-pb=col]')[1].dataset.pbId")
        await frame.evaluate("""id => { const c = document.querySelector('[data-pb-id="' + id + '"]'), r = c.getBoundingClientRect(), dt = new DataTransfer(); dt.setData('text/plain', 'new:button');
            c.dispatchEvent(new DragEvent('dragover', {dataTransfer: dt, clientX: r.left + 20, clientY: r.bottom - 5, bubbles: true, cancelable: true}));
            c.dispatchEvent(new DragEvent('drop', {dataTransfer: dt, clientX: r.left + 20, clientY: r.bottom - 5, bubbles: true, cancelable: true})); }""", col)
        await pg.wait_for_timeout(700)
        ok(await fr.locator(f"[data-pb-id='{col}'] [data-pb-type=button]").count() == 1, "trascinando «Pulsante» in una colonna dell'anteprima, finisce proprio lì")
        # modifica del titolo e dimensione diversa sul telefono
        await fr.locator("[data-pb-type=heading]").click(); await pg.wait_for_timeout(300)
        await pg.fill("#pb-props input[type=text]", "Le notizie di oggi"); await pg.dispatch_event("#pb-props input[type=text]", "change"); await pg.wait_for_timeout(600)
        ok("Le notizie di oggi" in await fr.locator("[data-pb-type=heading]").inner_text(), "il testo cambiato compare subito nell'anteprima")
        await pg.click("[data-dev=_m]"); await pg.click(".pbf-tabs button:nth-child(2)"); await pg.wait_for_timeout(200)
        ok("per telefono" in await pg.inner_text("#pb-props"), "in vista telefono le impostazioni valgono per il telefono")
        await pg.fill("#pb-props input[type=number]", "22"); await pg.dispatch_event("#pb-props input[type=number]", "change"); await pg.wait_for_timeout(700)
        css = await frame.evaluate("document.querySelector('#pb-root style').textContent")
        ok(re.search(r"@media \(max-width:767px\)\{[^@]*#pb-w\w+ \.pb-h\{font-size:22px\}", css) is not None, "nel CSS la dimensione vale solo sotto i 767 px (telefono)")
        await pg.screenshot(path=f"{OUT}/34-builder-telefono.png")
        await pg.click("[data-dev='']"); await pg.wait_for_timeout(300)
        # annulla e ripeti
        await pg.click("#pb-undo"); await pg.wait_for_timeout(600)
        css = await frame.evaluate("document.querySelector('#pb-root style').textContent")
        ok("font-size:22px" not in css, "Annulla toglie l'ultima modifica")
        await pg.click("#pb-redo"); await pg.wait_for_timeout(600)
        ok("font-size:22px" in await frame.evaluate("document.querySelector('#pb-root style').textContent"), "Ripeti la rimette")
        # barra degli strumenti nell'anteprima: duplica ed elimina
        await fr.locator("[data-pb-type=button]").click(); await pg.wait_for_timeout(300)
        await fr.locator(".pbf-bar button[data-op=dup]").click(); await pg.wait_for_timeout(700)
        ok(await fr.locator("[data-pb-type=button]").count() == 2, "la barra blu duplica il widget")
        await fr.locator(".pbf-bar button[data-op=del]").click(); await pg.wait_for_timeout(700)
        ok(await fr.locator("[data-pb-type=button]").count() == 1, "e lo elimina")
        # colonne della sezione
        await fr.locator("[data-pb=section]").nth(1).click(position={"x": 5, "y": 5}); await pg.wait_for_timeout(300)
        await pg.click(".pbf-tabs button:nth-child(1)"); await pg.locator(".pbf-layouts button").nth(1).click(); await pg.wait_for_timeout(700)
        ok(await fr.locator("[data-pb=section]").nth(1).locator("[data-pb=col]").count() == 2, "la sezione passa a due colonne, senza perdere i widget")
        # salvataggio della bozza: il sito non cambia
        await pg.click("#pb-save"); await pg.wait_for_timeout(800)
        with db() as c: draft, pub = c.execute("SELECT draft, published FROM layouts WHERE name = 'home'").fetchone()
        home = open("/tmp/shots/public/index.html", encoding="utf-8").read()
        ok("Le notizie di oggi" in draft and pub == "" and "pb-home" not in home, "«Salva la bozza» salva, ma il sito resta com'è")
        await pg.click("#pb-publish"); await pg.wait_for_timeout(1500)
        home = open("/tmp/shots/public/index.html", encoding="utf-8").read()
        ok("id=\"pb-home\"" in home and "Le notizie di oggi" in home, "«Pubblica»: la home del sito diventa quella costruita")
        ok("data-pb" not in home and "pb-frame" not in home and "draggable" not in home and "pb-editor" not in home, "nella home pubblicata non c'è niente dell'editor (né script né segni)")
        ok(home.count("<h1") == 1, "un solo titolo principale (H1) nella pagina")
        ok(not errs, "nessun errore JavaScript" + (f": {errs}" if errs else ""))
        await b.close()
asyncio.run(main())
print("\n".join(res))
