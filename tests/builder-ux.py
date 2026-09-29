# Builder: segnaposto e anteprime con pochi articoli, libreria delle immagini, ricerca dei widget; logo e icona in Aspetto.
import subprocess, sys, sqlite3, asyncio, base64, zlib, struct
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
def png(w=512):  # un PNG quadrato valido, generato qui
    raw = b"".join(b"\x00" + bytes([30, 107, 255]) * w for _ in range(w))
    ch = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    return b"\x89PNG\r\n\x1a\n" + ch(b"IHDR", struct.pack(">IIBBBBB", w, w, 8, 2, 0, 0, 0)) + ch(b"IDAT", zlib.compress(raw)) + ch(b"IEND", b"")
open(D + "/icona.png", "wb").write(png())
async def main():
    async with async_playwright() as p:
        b = await p.chromium.launch(); pg = await (await b.new_context(viewport={"width": 1920, "height": 953})).new_page(); bad = []
        # l'anteprima blocca apposta gli script del sito (CSP): quei messaggi non sono errori del builder
        pg.on("console", lambda m: ("Content Security Policy" in m.text or m.type == "error") and "/admin/builder/frame" not in m.location.get("url", "") and bad.append(m.text[:90]))
        await pg.goto(B + "/admin/login"); await pg.fill("input[name=email]", "a@x.it"); await pg.fill("input[name=password]", "password-lunga-123"); await pg.click("form button"); await pg.wait_for_load_state("networkidle")
        await pg.goto(B + "/admin/builder?pagina=home"); await pg.wait_for_timeout(2000)
        fr = pg.frame_locator("#pb-frame")
        for t in ("hero", "posts_grid", "posts_list"): await pg.click(f"#pb-palette button[data-type={t}]"); await pg.wait_for_timeout(500)
        ghost = await fr.locator(".pb-ghost").count(); none = await fr.get_by_text("Nessun articolo per questa scelta").count()
        ok(ghost >= 1 and none == 0, f"con due soli articoli, i blocchi successivi li mostrano in anteprima con la nota ({ghost} blocchi), niente «Nessun articolo»")
        await pg.click("#pb-palette button[data-type=image]"); await pg.wait_for_timeout(800)
        ok(await pg.evaluate("document.getElementById('pb-pick').open"), "aggiungendo un'immagine si apre da sola la libreria")
        await pg.click("#pb-pick-close"); await pg.wait_for_timeout(300)
        ok(await fr.locator(".pb-ph-img").count() == 1, "nell'anteprima c'è il segnaposto dell'immagine")
        await fr.locator(".pb-ph-img").click(); await pg.wait_for_timeout(800)
        ok(await pg.evaluate("document.getElementById('pb-pick').open"), "cliccando il segnaposto la libreria si riapre")
        await pg.click("#pb-pick-close")
        await pg.screenshot(path=f"{O}/builder-nuovo.png")
        await pg.fill("#pb-find", "imm"); await pg.wait_for_timeout(200)
        vis = await pg.evaluate("[...document.querySelectorAll('#pb-palette .pb-pal-grid button')].filter(b => !b.hidden).map(b => b.dataset.type)")
        ok(vis == ["image"], f"«Cerca un widget»: scrivendo «imm» resta solo Immagine ({vis})")
        ok(await pg.locator("#pb-palette button[data-type=image] svg").count() == 1, "la tavolozza ha le icone disegnate")
        await pg.fill("#pb-find", ""); await pg.wait_for_timeout(200)
        await pg.screenshot(path=f"{O}/builder-tavolozza.png", clip={"x": 0, "y": 0, "width": 320, "height": 953})
        ok(not bad, f"nessun errore nella console ({bad[:2]})")
        await b.close()
asyncio.run(main())
print("== Logo e icona in Aspetto")
r = c("-o", "/dev/null", "-w", "%{redirect_url}", "-F", "theme=classico", "-F", "color_mode=auto", "-F", f"favicon_upload=@{D}/icona.png;type=image/png", B + "/admin/aspetto")
fav = sqlite3.connect(D + "/presstatic.db").execute("SELECT value FROM settings WHERE key = 'favicon'").fetchone()
h = open(D + "/public/index.html", encoding="utf-8").read()
ok(fav and fav[0] and f'rel="icon" href="' in h and fav[0].split("/")[-1] in h, f"icona caricata da Aspetto e presente nel sito ({fav and fav[0]})")
ok("Icona (favicon)" in c(B + "/admin/aspetto") and "Icona attuale" in c(B + "/admin/aspetto"), "Aspetto mostra l'icona attuale")
c("-o", "/dev/null", "-F", "theme=classico", "-F", "color_mode=auto", "-F", "favicon_remove=on", B + "/admin/aspetto")
ok(sqlite3.connect(D + "/presstatic.db").execute("SELECT value FROM settings WHERE key = 'favicon'").fetchone()[0] == "", "«Togli l'icona» la toglie")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
