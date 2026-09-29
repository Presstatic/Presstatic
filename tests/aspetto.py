# Pagina «Aspetto»: temi con anteprima, colore, modalità chiara/scura/automatica, pulsante sole/luna.
import subprocess, sys, sqlite3, time, asyncio
from playwright.async_api import async_playwright
B, D = sys.argv[1], sys.argv[2]; S = "http://127.0.0.1:8472"
fails = 0
def ok(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l, flush=True)
J = D + "/j"; open(J, "w").close()
def c(*a): return subprocess.run(["curl", "-s", "-m", "60", "-b", J, "-c", J, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
c("-o", "/dev/null", "--data-urlencode", "email=a@x.it", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
with sqlite3.connect(D + "/presstatic.db") as d: d.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('base_url', ?)", (S,))
p = c(B + "/admin/aspetto")
ok("tema-classico.webp" in p and "tema-moderno.webp" in p and "Attivo" in p, "la pagina mostra i temi con l'anteprima e quello attivo")
ok(c("-o", "/dev/null", "-w", "%{http_code}", B + "/admin/assets/tema-moderno.webp") == "200", "le immagini d'anteprima si aprono")
ok("Aspetto" in c(B + "/admin") and "Costruisci" in c(B + "/admin"), "la voce «Aspetto» è nel menu e il pannello funziona")
def save(**kv):
    args = []
    for k, v in kv.items(): args += ["--form-string", f"{k}={v}"]  # la pagina invia il modulo con i file (logo e icona)
    import urllib.parse
    return urllib.parse.unquote(c("-o", "/dev/null", "-w", "%{redirect_url}", *args, B + "/admin/aspetto"))  # il messaggio è codificato nell'indirizzo
save(theme="moderno", accent="#0f766e", color_mode="dark", mode_toggle="on")
h = open(D + "/public/index.html", encoding="utf-8").read()
ok('data-mode="dark"' in h and "--accent:#0f766e" in h and "Bricolage" in h and "data-mode-toggle" in h, "tema Moderno, colore, modalità sempre scura e pulsante sole/luna applicati al sito")
ok("errore" in save(theme="inesistente").lower() or "non valido" in save(theme="inesistente"), "un tema inesistente viene rifiutato")
ok("non valido" in save(theme="moderno", accent="rosso"), "un colore non valido viene rifiutato")
srv = subprocess.Popen(["python3", "-m", "http.server", "8472", "-d", D + "/public"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); time.sleep(1)
BG = "getComputedStyle(document.body).backgroundColor"
async def main():
    async with async_playwright() as p:
        b = await p.chromium.launch()
        pg = await (await b.new_context(color_scheme="light")).new_page(); await pg.goto(S + "/"); await pg.wait_for_timeout(300)
        dark1 = await pg.evaluate(BG)
        await pg.click("[data-mode-toggle]"); await pg.wait_for_timeout(200); light = await pg.evaluate(BG)
        await pg.reload(); await pg.wait_for_timeout(300); kept = await pg.evaluate(BG)
        ok(dark1 != light and kept == light, f"sempre scura anche con il dispositivo chiaro; il pulsante passa al chiaro e la scelta resta ({dark1} → {light})")
        await b.close()
    save(theme="classico", accent="", color_mode="light", mode_toggle="")
    async with async_playwright() as p:
        b = await p.chromium.launch()
        pg = await (await b.new_context(color_scheme="dark")).new_page(); await pg.goto(S + "/"); await pg.wait_for_timeout(300)
        forced = await pg.evaluate(BG); tog = await pg.locator("[data-mode-toggle]").count()
        await b.close()
    save(theme="classico", accent="", color_mode="auto", mode_toggle="")
    async with async_playwright() as p:
        b = await p.chromium.launch()
        pg = await (await b.new_context(color_scheme="dark")).new_page(); await pg.goto(S + "/"); await pg.wait_for_timeout(300); auto = await pg.evaluate(BG)
        await b.close()
    ok(forced == "rgb(255, 255, 255)" and tog == 0, f"Classico sempre chiaro anche con il dispositivo scuro, senza pulsante ({forced})")
    ok(auto != "rgb(255, 255, 255)", f"in automatico, con il dispositivo scuro, il sito è scuro ({auto})")
asyncio.run(main()); srv.terminate()
c("-o", "/dev/null", "--form-string", "name=Red", "--form-string", "email=red@x.it", "--form-string", "password=password-lunga-123", "--form-string", "role=editor", B + "/admin/users/0")
open(J, "w").close(); c("-o", "/dev/null", "--data-urlencode", "email=red@x.it", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
ok(c("-o", "/dev/null", "-w", "%{http_code}", B + "/admin/aspetto") == "403" and "/admin/aspetto" not in c(B + "/admin"), "un redattore non vede e non apre «Aspetto»")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
