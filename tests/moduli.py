# Moduli: creazione dal pannello (anche nel browser), inserimento con [modulo N], invio, email con «Rispondi a»,
# errori, antispam, sicurezza, CSV, consenso facoltativo, conservazione, eliminazione.
# Uso: python3 tests/moduli.py http://127.0.0.1:PORTA /percorso/del/sito
import subprocess, sys, sqlite3, os, json, time, asyncio, email, email.policy
from aiosmtpd.controller import Controller
B, D = sys.argv[1], sys.argv[2]
fails = 0
def ok(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l, flush=True)
got = []
class Box:
    async def handle_DATA(self, s, ss, env): got.append(email.message_from_bytes(env.content, policy=email.policy.default)); return "250 OK"
Controller(Box(), hostname="127.0.0.1", port=2553).start()
J = D + "/j"; open(J, "w").close()
def c(*a): return subprocess.run(["curl", "-s", "-m", "60", "-b", J, "-c", J, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
def post(fid, data, ref="/contatti/"):
    open("/tmp/mod.out", "w").close()  # un reindirizzamento non ha contenuto: curl non creerebbe il file
    args = ["curl", "-s", "-m", "20", "-o", "/tmp/mod.out", "-w", "%{http_code} %{redirect_url}", "-e", B + ref]
    for k, v in data.items(): args += ["--data-urlencode", f"{k}={v}"]
    r = subprocess.run(args + [f"{B}/modulo/{fid}"], capture_output=True, text=True).stdout
    return r, open("/tmp/mod.out", encoding="utf-8", errors="replace").read()
db = lambda: sqlite3.connect(D + "/presstatic.db")
n_entries = lambda: db().execute("SELECT COUNT(*) FROM form_entries").fetchone()[0]
with db() as d:
    for k, v in [("smtp_host", "127.0.0.1"), ("smtp_port", "2553"), ("smtp_security", "none"), ("smtp_from", "sito@prova.it"), ("base_url", B)]: d.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (k, v))
c("-o", "/dev/null", "--data-urlencode", "email=a@x.it", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
print("== Creazione nel browser")
from playwright.async_api import async_playwright
async def browser():
    async with async_playwright() as p:
        b = await p.chromium.launch(); pg = await b.new_page(); bad = []
        pg.on("console", lambda m: ("Content Security Policy" in m.text) and bad.append(m.text[:90]))
        await pg.goto(B + "/admin/login"); await pg.fill("input[name=email]", "a@x.it"); await pg.fill("input[name=password]", "password-lunga-123"); await pg.click("form button"); await pg.wait_for_load_state("networkidle")
        await pg.goto(B + "/admin/moduli/0"); await pg.wait_for_timeout(400)
        start = await pg.locator(".frow").count()
        await pg.click("#fadd"); rows = pg.locator(".frow")
        await rows.nth(start).locator("input[type=text]").fill("Argomento"); await rows.nth(start).locator("select").select_option("select")
        await rows.nth(start).locator("textarea").fill("Cronaca\nSport\nAltro"); await rows.nth(start).locator("input[type=checkbox]").check()
        await pg.fill("input[name=notify]", "redazione@prova.it"); await pg.check("#fconsent"); await pg.fill("input[name=privacy_url]", "https://www.prova.it/privacy/")
        await pg.click("button:has-text(\"Salva il modulo\")"); await pg.wait_for_load_state("networkidle")
        await b.close(); return start, bad
start, bad = asyncio.run(browser())
f = db().execute("SELECT id, fields, notify, consent FROM forms").fetchone()
ok(f and start == 3 and len(json.loads(f[1])) == 4 and f[2] == "redazione@prova.it" and f[3] == 1, f"modulo creato dal browser: parte con 3 campi, +1 elenco, email e consenso ({f and len(json.loads(f[1]))} campi)")
ok(not bad, "editor dei campi: nessuno script bloccato dalla CSP del pannello")
FID = f[0]
fields = json.loads(f[1]); fields[0]["label"] = "Nome <script>alert(1)</script>"
c("-o", "/dev/null", "--data-urlencode", "name=Contatti", "--data-urlencode", "notify=redazione@prova.it", "--data-urlencode", "consent=on", "--data-urlencode", f"fields={json.dumps(fields)}", B + f"/admin/moduli/{FID}")
pg = db().execute("SELECT id FROM posts WHERE kind = 'page' AND slug = 'contatti'").fetchone()  # la pagina «Contatti» creata all'installazione
c("-o", "/dev/null", "--form-string", "title=Contatti", "--form-string", "slug=contatti", "--form-string", "kind=page", "--form-string", "status=published", "--form-string", f"body=<p>Scrivici.</p><p>[modulo {FID}]</p>", B + f"/admin/edit/{pg[0] if pg else 0}?k=page")
print("== Il modulo nel sito")
h = open(f"{D}/public/contatti/index.html", encoding="utf-8").read()
ok(f'action="{B}/modulo/{FID}"' in h and 'name="f_messaggio"' in h and "<option>Sport</option>" in h, "[modulo N] nella pagina diventa il modulo, con tutti i campi")
ok('name="consenso"' in h and "privacy/" in h and 'name="website"' in h, "consenso con link all'informativa e campo trappola presenti")
ok("<script>alert(1)</script>" not in h and "&lt;script&gt;" in h, "un'etichetta con codice viene mostrata come testo")
ok(f'id="modulo-{FID}-inviato"' in h and "<p><div" not in h, "messaggio di conferma pronto (senza JavaScript) e nessun paragrafo rotto intorno")
print("== Invio")
good = {"f_nome": "Mario Rossi", "f_email": "mario@lettore.it", "f_messaggio": "Buongiorno, <img src=x onerror=alert(1)> vi segnalo...", "f_c3": "Sport", "consenso": "on", "t": "8000"}
fk = [x["id"] for x in json.loads(db().execute("SELECT fields FROM forms").fetchone()[0])]
good = {("f_" + fk[3] if k == "f_c3" else k): v for k, v in good.items()}
r, _ = post(FID, good)
ok(r.startswith("303") and r.endswith(f"/contatti/#modulo-{FID}-inviato"), f"invio corretto: torna alla pagina con la conferma ({r})")
ok(n_entries() == 1, "il messaggio è salvato")
for _ in range(20):
    if got: break
    time.sleep(0.3)
m = got[0] if got else None
ok(m is not None and m["To"] == "redazione@prova.it" and "mario@lettore.it" in str(m.get("Reply-To", "")), "email alla redazione con «Rispondi a» su chi ha scritto")
ok(m is not None and "Mario Rossi" in m.get_body(("plain",)).get_content(), "nell'email ci sono i dati del modulo")
print("== Errori spiegati")
for data, expect, label in [({**good, "f_nome": ""}, "compila il campo", "campo obbligatorio vuoto"), ({**good, "f_email": "non-una-email"}, "email valido", "email non valida"),
                            ({**good, "f_" + fk[3]: "Politica"}, "elenco", "voce che non è nell'elenco"), ({k: v for k, v in good.items() if k != "consenso"}, "consenso", "consenso mancante")]:
    r, body = post(FID, {**data, "f_messaggio": data["f_messaggio"] + label})
    ok(r.startswith("400") and expect in body.lower(), f"{label}: rifiutato con un messaggio chiaro")
print("== Antispam")
before = n_entries()
post(FID, {**good, "website": "http://spam.example", "f_messaggio": "trappola"}); ok(n_entries() == before, "campo trappola compilato: scartato in silenzio")
post(FID, {**good, "t": "800", "f_messaggio": "veloce"}); ok(n_entries() == before, "compilato in meno di 2,5 secondi: scartato")
post(FID, good); ok(n_entries() == before, "stesso messaggio due volte: niente doppione")
for i in range(4): post(FID, {**good, "f_messaggio": f"messaggio {i}"})
r, body = post(FID, {**good, "f_messaggio": "il sesto"})
ok(r.startswith("400") and "troppi" in body, "sesto invio in dieci minuti: fermato")
big = "x" * 40000
r = subprocess.run(["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", "--data-urlencode", f"f_messaggio={big}", f"{B}/modulo/{FID}"], capture_output=True, text=True).stdout
ok(r == "413", f"invio troppo grande: rifiutato subito ({r})")
print("== Pannello")
p = c(B + f"/admin/moduli/{FID}/messaggi")
ok("<img src=x" not in p and "&lt;img src=x" in p, "nei messaggi il codice inviato da un lettore è solo testo")
with db() as d: d.execute("INSERT INTO form_entries(form_id, data, ip_hash, seen, created_at) VALUES (?, ?, 'x', 1, 0)", (FID, json.dumps([["Nome", "=HYPERLINK(\"http://x\")"]])))
csv = c(B + f"/admin/moduli/{FID}/csv")
ok("'=HYPERLINK" in csv and "Mario Rossi" in csv, "CSV: dati esportati e formule neutralizzate")
c("-o", "/dev/null", "--data-urlencode", "keep=30", B + "/admin/moduli"); c("-o", "/dev/null", B + "/admin/moduli")
ok(db().execute("SELECT COUNT(*) FROM form_entries WHERE created_at = 0").fetchone()[0] == 0, "i messaggi più vecchi del periodo scelto si cancellano da soli")
c("-o", "/dev/null", "--data-urlencode", "name=Contatti", "--data-urlencode", f"fields={json.dumps(fields)}", B + f"/admin/moduli/{FID}")
h = open(f"{D}/public/contatti/index.html", encoding="utf-8").read()
ok('name="consenso"' not in h, "consenso spento: la casella sparisce dal modulo")
with db() as d: d.execute("DELETE FROM form_entries")  # azzera il limite di invii scattato nelle prove precedenti
r, _ = post(FID, {**{k: v for k, v in good.items() if k != "consenso"}, "f_messaggio": "senza consenso"}, ref="/contatti/")
ok(r.startswith("303"), "e l'invio senza consenso è accettato")
c("-o", "/dev/null", "-X", "POST", B + f"/admin/moduli/{FID}/elimina")
h = open(f"{D}/public/contatti/index.html", encoding="utf-8").read()
ok('class="ps-form"' not in h and db().execute("SELECT COUNT(*) FROM form_entries").fetchone()[0] == 0, "modulo eliminato: sparisce dalla pagina, con i suoi messaggi")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
