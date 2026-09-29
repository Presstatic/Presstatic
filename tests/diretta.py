# Dirette e pubblicità tra le notizie. Uso: python3 tests/diretta.py http://127.0.0.1:PORTA /percorso/del/sito
import subprocess, sys, sqlite3, time, asyncio, json, re
from playwright.async_api import async_playwright
B, D = sys.argv[1], sys.argv[2]; S = "http://127.0.0.1:8473"
fails = 0
def ok(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l, flush=True)
J = D + "/j"
def login(email="a@x.it"):
    open(J, "w").close(); c("-o", "/dev/null", "--data-urlencode", f"email={email}", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
def c(*a): return subprocess.run(["curl", "-s", "-m", "60", "-b", J, "-c", J, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
def js(url, **kv):
    args = []
    for k, v in kv.items(): args += ["--data-urlencode", f"{k}={v}"]
    out = c("-X", "POST", "-H", "Accept: application/json", *args, B + url)  # sempre un invio, anche senza dati
    try: return json.loads(out)
    except Exception: return {"raw": out[:120]}
page = lambda p: open(f"{D}/public/{p}", encoding="utf-8").read()
login()
with sqlite3.connect(D + "/presstatic.db") as d:
    for k, v in [("base_url", S), ("menu", "Sport\nCronaca")]: d.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (k, v))
extra = [(f"Notizia {i}", f"notizia-{i}", "Sport" if i % 2 else "Cronaca") for i in range(1, 15)]  # abbastanza articoli per avere sezioni in home
for t, sl, cat in extra + [("Derby in diretta", "derby", "Sport"), ("Altra notizia", "altra", "Cronaca"), ("Terza notizia", "terza", "Sport"), ("Quarta notizia", "quarta", "Cronaca")]:
    c("-o", "/dev/null", "--form-string", f"title={t}", "--form-string", f"slug={sl}", "--form-string", "body=<p>Il riassunto della partita.</p>", "--form-string", f"category={cat}", "--form-string", "status=published", B + "/admin/edit/0")
pid = sqlite3.connect(D + "/presstatic.db").execute("SELECT id FROM posts WHERE slug = 'derby'").fetchone()[0]
print("== Diretta")
ok(js(f"/admin/edit/{pid}/diretta", body="troppo presto").get("ok") is False, "prima di avviarla, gli aggiornamenti non si accettano")
ok(js(f"/admin/edit/{pid}/diretta/stato", action="start").get("state") == 1, "l'articolo diventa una diretta")
js(f"/admin/edit/{pid}/diretta", body="Fischio d'inizio, squadre in campo."); time.sleep(1.1)
r = js(f"/admin/edit/{pid}/diretta", title="Gol!", body="Vantaggio dei padroni di casa al 12'.", key="on"); time.sleep(1.1)
js(f"/admin/edit/{pid}/diretta", body="<script>alert(1)</script> tentativo")
ok(r.get("ok") and r.get("id"), "gli aggiornamenti si pubblicano dall'editor")
h = page("derby/index.html")
ld = json.loads(re.search(r'<script type="application/ld\+json">(.*?)</script>', h, re.S).group(1))
art = next((x for x in (ld if isinstance(ld, list) else ld.get("@graph", [ld])) if x.get("@type") == "LiveBlogPosting"), None)
ok('id="diretta" data-live="1"' in h and h.count('<li id="agg-') == 3 and 'class="key"' in h, "la pagina mostra la diretta con i tre aggiornamenti, quello importante evidenziato")
ok(h.index("tentativo") < h.index("Vantaggio") < h.index("Fischio"), "gli aggiornamenti sono dal più recente")
ok(art and len(art.get("liveBlogUpdate", [])) == 3 and art.get("coverageStartTime") and "coverageEndTime" not in art and art["liveBlogUpdate"][0]["url"].endswith("#agg-" + h.split('<li id="agg-')[1].split('"')[0]), "dati strutturati LiveBlogPosting con inizio e aggiornamenti (nessuna fine finché è in corso)")
ok("<script>alert(1)</script>" not in h.split('id="diretta"')[1].split("</section>")[0] and "&lt;script&gt;" in h, "il codice scritto in un aggiornamento resta testo")
ok('class="live-badge"' in h and 'class="live-badge"' in page("index.html"), "etichetta «In diretta» nell'articolo e in home")
srv = subprocess.Popen(["python3", "-m", "http.server", "8473", "-d", D + "/public"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); time.sleep(1)
async def reader():
    async with async_playwright() as p:
        b = await p.chromium.launch(); pg = await b.new_page(); await pg.goto(S + "/derby/"); await pg.wait_for_timeout(300)
        before = await pg.locator(".ps-feed>li").count()
        js(f"/admin/edit/{pid}/diretta", body="Raddoppio al 30'.")
        await pg.evaluate("window.psLiveCheck()"); await pg.wait_for_timeout(500)
        shown = await pg.is_visible(".ps-new button"); label = await pg.inner_text(".ps-new button")
        await pg.click(".ps-new button"); await pg.wait_for_timeout(200)
        first = await pg.inner_text(".ps-feed>li:first-child"); after = await pg.locator(".ps-feed>li").count()
        await b.close(); return before, shown, label, first, after
before, shown, label, first, after = asyncio.run(reader())
ok(shown and label == "1 nuovo aggiornamento" and "Raddoppio" in first and after == before + 1, "chi sta leggendo riceve «1 nuovo aggiornamento» e lo vede in cima, senza ricaricare")
async def editor():
    async with async_playwright() as p:
        b = await p.chromium.launch(); pg = await b.new_page(); bad = []
        pg.on("console", lambda m: "Content Security Policy" in m.text and bad.append(m.text[:80]))
        await pg.goto(B + "/admin/login"); await pg.fill("input[name=email]", "a@x.it"); await pg.fill("input[name=password]", "password-lunga-123"); await pg.click("form button"); await pg.wait_for_load_state("networkidle")
        other = sqlite3.connect(D + "/presstatic.db").execute("SELECT id FROM posts WHERE slug = 'terza'").fetchone()[0]
        await pg.goto(B + f"/admin/edit/{other}"); await pg.wait_for_timeout(500)
        title = await pg.title()
        await pg.click("#livebox [data-live-state=start]"); await pg.wait_for_timeout(1500)
        state = await pg.get_attribute("#livebox", "data-state")
        await pg.fill("#lv-body", "Primo aggiornamento dall'editor"); await pg.click("#lv-add"); await pg.wait_for_timeout(1500)
        items = await pg.locator("#lv-list li").count()
        await b.close(); return title, state, items, bad
title, state, items, bad = asyncio.run(editor())
ok("<" not in title and state == "1" and items == 1 and not bad, f"nell'editor: «Trasforma in diretta» e un aggiornamento, senza ricaricare e senza script bloccati (titolo «{title[:30]}»)")
js(f"/admin/edit/{pid}/diretta/stato", action="end")
h = page("derby/index.html"); art = json.loads(re.search(r'<script type="application/ld\+json">(.*?)</script>', h, re.S).group(1))
art = next((x for x in (art if isinstance(art, list) else art.get("@graph", [art])) if x.get("@type") == "LiveBlogPosting"), {})
ok('data-live="0"' in h and "Diretta conclusa" in h and art.get("coverageEndTime") and "psLiveCheck" not in h, "chiusa: «Diretta conclusa», fine nei dati strutturati, nessun controllo automatico")
ok('<span class="live-badge">' not in page("index.html").split("Derby in diretta")[0][-400:], "in home l'etichetta «In diretta» sparisce")
uid = sqlite3.connect(D + "/presstatic.db").execute("SELECT id FROM diretta WHERE body LIKE 'Raddoppio%'").fetchone()[0]
js(f"/admin/diretta/{uid}/elimina")
ok("Raddoppio" not in page("derby/index.html"), "un aggiornamento eliminato sparisce dalla pagina")
c("-o", "/dev/null", "--form-string", "name=Aut", "--form-string", "email=aut@x.it", "--form-string", "password=password-lunga-123", "--form-string", "role=author", B + "/admin/users/0")
login("aut@x.it")
ok(c("-o", "/dev/null", "-w", "%{http_code}", "--data-urlencode", "body=x", B + f"/admin/edit/{pid}/diretta") == "403", "un autore non può aggiornare la diretta")
print("== Pubblicità tra le notizie")
login()
with sqlite3.connect(D + "/presstatic.db") as d:
    for k, v in [("ad_list", "<div class='annuncio'>ANNUNCIO LISTA</div>"), ("ad_list_every", "1"), ("ad_sticky", "<div class='fisso'>ANNUNCIO FISSO</div>"), ("per_page", "4")]:
        d.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (k, v))
c("-o", "/dev/null", "-X", "POST", B + "/admin/rebuild")
h = page("index.html")
ok(h.count('class="ad ad-list"') >= 1 and "ANNUNCIO FISSO" in h and "Chiudi l'annuncio" in h, "in home: annuncio tra le sezioni e annuncio fisso chiudibile")
with sqlite3.connect(D + "/presstatic.db") as d: d.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('consent_mode', 'native')")
c("-o", "/dev/null", "-X", "POST", B + "/admin/rebuild")
h = page("index.html")
ok("<template data-consent=\"ads\"><div class='annuncio'>" in h and "<template data-consent=\"ads\"><div class='fisso'>" in h, "con il banner dei cookie anche questi annunci aspettano il consenso")
srv.terminate()
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
