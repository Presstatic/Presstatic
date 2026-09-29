# Consenso cookie: banner di Presstatic (codici inerti fino al sì, per categoria), piattaforma esterna (per prima nella
# pagina), Google Consent Mode v2, nessun banner. Uso: python3 tests/consenso.py http://127.0.0.1:PORTA /percorso/del/sito
import subprocess, sys, sqlite3, time, asyncio, re
from playwright.async_api import async_playwright
B, D = sys.argv[1], sys.argv[2]; S = "http://127.0.0.1:8471"
fails = 0
def ok(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l, flush=True)
J = D + "/j"; open(J, "w").close()
def c(*a): return subprocess.run(["curl", "-s", "-m", "60", "-b", J, "-c", J, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
c("-o", "/dev/null", "--data-urlencode", "email=a@x.it", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
body = "".join(f"<p>Paragrafo {i} del testo di prova.</p>" for i in range(1, 7))
c("-o", "/dev/null", "--form-string", "title=Articolo con pubblicità", "--form-string", "slug=articolo", "--form-string", f"body={body}", "--form-string", "status=published", B + "/admin/edit/0")
def setup(**kv):
    base = {"base_url": S, "head_scripts": "<script>window.__stats=1</script>", "body_scripts": "<script>window.__body=1</script>", "ad_head": "<script>window.__ads=1</script>",
            "ad_top": "<div id=\"adtop\">ANNUNCIO</div>", "ad_inarticle": "<div id=\"adin\">ANNUNCIO NEL TESTO</div>", "ad_paragraph": "2", "consent_gcm": "on", "consent_cmp": "", "consent_policy": "/cookie-policy/"}
    base.update(kv)
    with sqlite3.connect(D + "/presstatic.db") as d:
        for k, v in base.items(): d.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (k, v))
    c("-o", "/dev/null", "-X", "POST", B + "/admin/rebuild")
page = lambda p="index.html": open(f"{D}/public/{p}", encoding="utf-8").read()
srv = subprocess.Popen(["python3", "-m", "http.server", "8471", "-d", D + "/public"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); time.sleep(1)
print("== Banner di Presstatic")
setup(consent_mode="native")
h = page(); head = h.split("</head>")[0]
ok('<template data-consent="stats"><script>window.__stats=1</script></template>' in head and '<template data-consent="ads"><script>window.__ads=1</script></template>' in head, "i codici esterni sono nella pagina ma inerti, ognuno con la sua categoria")
ok("gtag('consent','default'" in head and head.index("gtag('consent','default'") < head.index("window.__stats"), "Google Consent Mode v2: consenso negato di partenza, prima dei codici")
a = page("articolo/index.html")
ok('<template data-consent="ads"><div id="adtop">' in a and '<template data-consent="ads"><div id="adin">' in a, "anche gli annunci dell'articolo (in alto e nel testo) aspettano il consenso")
V = "() => [window.__stats, window.__body, window.__ads].map(x => x === 1)"
async def main():
    async with async_playwright() as p:
        b = await p.chromium.launch()
        async def fresh():
            pg = await (await b.new_context()).new_page(); await pg.goto(S + "/articolo/"); await pg.wait_for_timeout(400); return pg
        pg = await fresh()
        ok(await pg.is_visible("#ps-cc") and await pg.evaluate(V) == [False, False, False], "prima visita: banner visibile, nessun codice esterno partito")
        ok(not await pg.is_visible("#ps-cc .ps-cc-opts") and not await pg.is_visible("#ps-cc-re"), "le caselle compaiono solo con «Personalizza», e il pulsante per cambiare idea solo dopo la scelta")
        await pg.click("#ps-cc [data-cc=reject]"); await pg.reload(); await pg.wait_for_timeout(400)
        ok(not await pg.is_visible("#ps-cc") and await pg.evaluate(V) == [False, False, False] and await pg.is_visible("#ps-cc-re"), "«Rifiuta»: niente parte, anche ricaricando; resta il pulsante per cambiare idea")
        pg = await fresh(); await pg.click(".ps-cc-x"); await pg.wait_for_timeout(200)
        ok(await pg.evaluate("JSON.parse(localStorage.getItem('ps-consent')).stats === false") and await pg.evaluate(V) == [False, False, False], "la ✕ vale come rifiuto")
        pg = await fresh(); await pg.click("#ps-cc [data-cc=accept]"); await pg.wait_for_timeout(400)
        dl = await pg.evaluate("JSON.stringify(window.dataLayer)")
        ok(await pg.evaluate(V) == [True, True, True] and await pg.is_visible("#adtop") and await pg.is_visible("#adin"), "«Accetta tutti»: partono statistiche e pubblicità, compresi gli annunci nell'articolo")
        ok('"analytics_storage":"granted"' in dl and '"ad_storage":"granted"' in dl, "e Google riceve il consenso aggiornato")
        await pg.reload(); await pg.wait_for_timeout(400)
        ok(not await pg.is_visible("#ps-cc") and await pg.evaluate(V) == [True, True, True], "alla visita dopo, partono subito, senza banner")
        await pg.click("#ps-cc-re"); await pg.uncheck("#ps-cc-ads"); await pg.click("#ps-cc [data-cc=save]"); await pg.wait_for_load_state("load"); await pg.wait_for_timeout(600)
        ok(await pg.evaluate(V) == [True, True, False] and not await pg.is_visible("#adtop"), "revoca della pubblicità: la pagina si ricarica e gli annunci non partono più")
        pg = await fresh(); await pg.click("#ps-cc [data-cc=custom]"); await pg.check("#ps-cc-stats"); await pg.click("#ps-cc [data-cc=save]"); await pg.wait_for_timeout(300)
        ok(await pg.evaluate(V) == [True, True, False], "«Personalizza»: solo le statistiche, la pubblicità no")
        await b.close()
asyncio.run(main())
print("== Piattaforma esterna")
setup(consent_mode="external", consent_cmp="<script id=\"cmp\">window.__cmp=1</script>")
head = page().split("</head>")[0]
ok('id="cmp"' in head and head.index('id="cmp"') < head.index("window.__stats") and head.index("gtag('consent','default'") < head.index('id="cmp"'), "Consent Mode, poi lo script della piattaforma, poi i codici della redazione")
ok("<template data-consent" not in page() and 'id="ps-cc"' not in page(), "niente banner di Presstatic e codici lasciati alla piattaforma")
print("== Nessun banner")
setup(consent_mode="")
h = page()
ok("gtag('consent'" not in h and 'id="ps-cc"' not in h and "<script>window.__stats=1</script>" in h, "consenso spento: codici normali, nessun banner")
ok("Cookie e consenso" in c(B + "/admin/settings"), "nel pannello c'è il riquadro «Cookie e consenso»")
srv.terminate()
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
