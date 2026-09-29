# CSP con nonce: apre ogni pagina del pannello in Chromium e raccoglie gli script bloccati (devono essere zero),
# poi controlla che l'editor, le conferme e la verifica in due passaggi obbligatoria funzionino.
# Uso: python3 tests/csp-pannello.py http://127.0.0.1:PORTA /percorso/del/sito
import asyncio, sys, sqlite3, subprocess, re
from playwright.async_api import async_playwright
B, DIR = sys.argv[1], sys.argv[2]
fails = 0
def ok(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l, flush=True)
def curl(*a): return subprocess.run(["curl", "-s", "-m", "10", "-b", DIR + "/j", "-c", DIR + "/j", "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
curl("-o", "/dev/null", "--data-urlencode", "email=a@x.it", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
curl("-o", "/dev/null", "--form-string", "title=Articolo di prova", "--form-string", "slug=prova", "--form-string", "body=<p>Testo.</p>", "--form-string", "status=published", B + "/admin/edit/0")
curl("-o", "/dev/null", "--form-string", "title=Articolo di prova (2)", "--form-string", "slug=prova", "--form-string", "body=<p>Testo 2.</p>", "--form-string", "status=published", B + "/admin/edit/1")
db = sqlite3.connect(DIR + "/presstatic.db")
pid = db.execute("SELECT id FROM posts WHERE slug = 'prova'").fetchone()[0]
rev = (db.execute("SELECT id FROM revisions ORDER BY id LIMIT 1").fetchone() or [0])[0]
db.execute("INSERT INTO comments(post_id, name, email, body, status, created_at) VALUES (?, 'Mario', 'm@x.it', 'Commento di prova', 'pending', 0)", (pid,)); db.commit()
PAGES = ["/admin", "/admin?k=page", "/admin/edit/0", f"/admin/edit/{pid}", "/admin/media", "/admin/categorie", "/admin/commenti", "/admin/ai",
         "/admin/settings", "/admin/integrations", "/admin/indicizzazione", "/admin/files", "/admin/builder", "/admin/builder?pagina=testata",
         "/admin/builder?pagina=piede", "/admin/newsletter", "/admin/push", "/admin/importa", "/admin/backup", "/admin/users", "/admin/users/1",
         "/admin/users/0", "/admin/updates", "/admin/2fa", "/admin/profile", f"/admin/revision/{rev}", f"/admin/preview/{pid}"]
async def main():
    async with async_playwright() as p:
        b = await p.chromium.launch(); ctx = await b.new_context(viewport={"width": 1400, "height": 900}, locale="it-IT")
        blocked = []; errs = []; preview_blocked = []
        def on_console(m, url):
            t = m.text
            if "Content Security Policy" in t or "Refused to" in t:
                # L'anteprima del builder (riquadro) ha di proposito «script-src 'self'» senza nonce: gli script del tema
                # lì vanno bloccati. Contano solo i blocchi delle pagine del pannello, la cui regola ha il nonce.
                m = re.search(r'directive: "([^"]+)"', t); rule = m.group(1) if m else ""
                (blocked if "nonce-" in rule else preview_blocked).append(f"{url}: {t[:150]}")
        pg = await ctx.new_page()
        pg.on("pageerror", lambda e: errs.append(f"{pg.url}: {str(e)[:120]}"))
        pg.on("console", lambda m: on_console(m, pg.url))
        await pg.goto(B + "/admin/login"); await pg.fill("input[name=email]", "a@x.it"); await pg.fill("input[name=password]", "password-lunga-123"); await pg.click("form button"); await pg.wait_for_load_state("networkidle")
        for path in PAGES:
            r = await pg.goto(B + path); await pg.wait_for_timeout(700)
            csp = (await r.all_headers()).get("content-security-policy", "") if r else ""
            if "nonce-" not in csp and "sandbox" not in csp: blocked.append(f"{path}: pagina senza CSP con nonce ({csp[:60]})")
        ok(not blocked, f"{len(PAGES)} pagine del pannello: nessuno script bloccato dalla CSP" + ("" if not blocked else ":\n      " + "\n      ".join(blocked[:8])))
        ok(len(preview_blocked) > 0, f"nell'anteprima del builder gli script del tema sono bloccati, come previsto ({len(preview_blocked)} blocchi)")
        ok(not errs, "nessun errore JavaScript" + ("" if not errs else ": " + "; ".join(errs[:4])))
        # l'editor degli articoli (Quill) funziona
        await pg.goto(B + f"/admin/edit/{pid}"); await pg.wait_for_timeout(800)
        ed = pg.locator(".ql-editor").first
        await ed.click(); await pg.keyboard.type(" aggiunto"); await pg.wait_for_timeout(200)
        ok("aggiunto" in await ed.inner_text(), "l'editor degli articoli si scrive normalmente")
        # le conferme ora passano dal gestore unico: il dialogo compare e «Annulla» ferma l'azione
        dialogs = []
        # «Vuoi uscire senza salvare?» (beforeunload) si accetta; le conferme di eliminazione si annullano.
        pg.on("dialog", lambda d: asyncio.ensure_future(d.accept()) if d.type == "beforeunload" else (dialogs.append(d.message), asyncio.ensure_future(d.dismiss())))
        curl("-o", "/dev/null", "--form-string", "title=Bozza da eliminare", "--form-string", "slug=bozza", "--form-string", "body=<p>x</p>", "--form-string", "status=draft", B + "/admin/edit/0")
        did = db.execute("SELECT id FROM posts WHERE slug = 'bozza'").fetchone()[0]
        await pg.goto(B + f"/admin/edit/{did}"); await pg.wait_for_timeout(600)
        await pg.locator("button[form=del]").first.click(); await pg.wait_for_timeout(600)
        still = db.execute("SELECT COUNT(*) FROM posts WHERE id = ?", (did,)).fetchone()[0]
        ok(dialogs and "Eliminare" in dialogs[-1] and still == 1, "il pulsante Elimina chiede conferma, e «Annulla» non elimina niente")
        # scelta del commento (data-pick) e conferma sul pulsante
        await pg.goto(B + "/admin/commenti"); await pg.wait_for_timeout(500)
        dialogs.clear()
        await pg.locator("[data-pick][data-confirm]").first.click(); await pg.wait_for_timeout(500)
        ok(dialogs and "Eliminare il commento" in dialogs[-1] and db.execute("SELECT COUNT(*) FROM comments").fetchone()[0] == 1, "commenti: eliminare chiede conferma, «Annulla» non tocca niente")
        await b.close()
asyncio.run(main())
print("== Verifica in due passaggi obbligatoria")
curl("-o", "/dev/null", "--form-string", "name=Red", "--form-string", "email=red@x.it", "--form-string", "password=password-lunga-123", "--form-string", "role=editor", B + "/admin/users/0")
curl("-o", "/dev/null", "--form-string", "name=Aut", "--form-string", "email=aut@x.it", "--form-string", "password=password-lunga-123", "--form-string", "role=author", B + "/admin/users/0")
curl("-o", "/dev/null", "--data-urlencode", "twofa_required=on", B + "/admin/users/obbligo-2fa")
def as_user(email, path):
    j = DIR + "/" + email + ".jar"; open(j, "w").close()
    subprocess.run(["curl", "-s", "-o", "/dev/null", "-b", j, "-c", j, "--data-urlencode", f"email={email}", "--data-urlencode", "password=password-lunga-123", B + "/admin/login"])
    return subprocess.run(["curl", "-s", "-o", "/dev/null", "-w", "%{http_code} %{redirect_url}", "-b", j, B + path], capture_output=True, text=True).stdout
r = as_user("red@x.it", "/admin")
ok(r.startswith("303") and "/admin/2fa" in r, "un redattore senza verifica in due passaggi viene portato ad attivarla")
ok(as_user("red@x.it", "/admin/2fa").startswith("200"), "e la pagina per attivarla si apre")
ok(as_user("aut@x.it", "/admin").startswith("200"), "gli autori non sono obbligati (l'HTML libero non ce l'hanno)")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
