# Newsletter: iscrizione con doppia conferma, riepilogo dei nuovi articoli, disiscrizione con un clic.
# Uso: python3 tests/newsletter.py http://127.0.0.1:8210 /percorso/del/sito
import urllib.parse, subprocess, sys, re, time, sqlite3, os, email, email.policy, urllib.parse
from aiosmtpd.controller import Controller
B, DIR = sys.argv[1], sys.argv[2]
PUB = f"{DIR}/public"
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
inbox = []
class Box:
    async def handle_DATA(self, server, session, envelope):
        m = email.message_from_bytes(envelope.content, policy=email.policy.default)
        inbox.append({"to": envelope.rcpt_tos[0], "subject": m["subject"], "unsub": m["List-Unsubscribe"], "post": m["List-Unsubscribe-Post"], "text": m.get_body(("plain",)).get_content()}); return "250 OK"
smtp = Controller(Box(), hostname="127.0.0.1", port=2528); smtp.start()
def curl(jar, *a): return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, *a], capture_output=True, text=True).stdout
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def page(slug): f = f"{PUB}/{slug}/index.html"; return open(f, encoding="utf-8").read() if os.path.isfile(f) else ""
def setting(k, v):
    with db() as c: c.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (k, v))
def wait(n, secs=15):
    end = time.time() + secs
    while time.time() < end and len(inbox) < n: time.sleep(0.2)
    return len(inbox) >= n
A = "/tmp/nl-a.jar"; open(A, "w").close()
curl(A, "-o", "/dev/null", "--data-urlencode", "email=andrea@example.com", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
adm = lambda path, *a: curl(A, "-H", "Origin: " + B, *a, B + path)
WWW = "https://www.giornale-di-prova.it"
def sub_(mail, consent=True, trap=""):
    args = ["-H", "Origin: " + WWW, "-w", "\n%{http_code}", "--data-urlencode", f"email={mail}", "--data-urlencode", f"website={trap}"] + (["--data-urlencode", "consent=on"] if consent else [])
    return curl("/tmp/nl-x.jar", *args, B + "/newsletter/iscriviti")
adm("/admin/edit/0", "-o", "/dev/null", "--form-string", "title=Primo articolo", "--form-string", "slug=primo", "--form-string", "body=<p>Uno.</p>", "--form-string", "status=published")
print("== 1. Spenta e accesa")
off = page("primo")
ok('class="nl"' not in off, "spenta: negli articoli non c'è il modulo")
for k, v in [("smtp_host", "127.0.0.1"), ("smtp_port", "2528"), ("smtp_security", "none"), ("smtp_from", "redazione@example.com"), ("admin_url", B), ("newsletter_last", str(int(time.time())))]: setting(k, v)
sv = re.search(r'name="sv" value="(\d+)"', adm("/admin/newsletter")).group(1)
adm("/admin/newsletter", "-o", "/dev/null", "--data-urlencode", f"sv={sv}", "--data-urlencode", "newsletter_on=on", "--data-urlencode", "newsletter_title=La Gazzetta del mattino", "--data-urlencode", "newsletter_every=off", "--data-urlencode", "newsletter_hour=7")
on = page("primo")
ok('/newsletter/iscriviti"' in on and f'action="{B}/' not in on and "La Gazzetta del mattino" in on, "accesa: il modulo è in fondo all'articolo e invia al sito, non al pannello (che può restare chiuso)")
ok(on.count("<script") == off.count("<script"), "nessun JavaScript in più")
print(f"   peso aggiunto all'articolo: {len(on.encode()) - len(off.encode())} byte, prima della compressione")
print("== 2. Iscrizione con doppia conferma")
r = sub_("lettore@example.com")
ok(r.endswith("200") and "Controlla la tua email" in r, "iscrizione dal sito: «controlla la tua email»")
with db() as c: ok(c.execute("SELECT status FROM subscribers WHERE email = 'lettore@example.com'").fetchone()[0] == "pending", "resta in attesa finché non conferma")
ok(wait(1) and "Conferma" in inbox[0]["subject"], "arriva l'email di conferma")
link = re.search(r"(http\S+/newsletter/conferma/[0-9a-f]+)", inbox[0]["text"]).group(1)
# In produzione il link punta al sito e Nginx lo passa al programma; qui non c'è Nginx, quindi lo si segue sul programma.
link = B + urllib.parse.urlparse(link).path
ok("Iscrizione confermata" in curl("/tmp/nl-x.jar", link.replace(B, B)), "il link di conferma conferma l'iscrizione")
n = len(inbox); sub_("lettore@example.com"); time.sleep(1)
ok(len(inbox) == n, "chi è già iscritto non riceve un'altra email")
sub_("in-attesa@example.com"); wait(n + 1)
with db() as c: before = c.execute("SELECT COUNT(*) FROM subscribers").fetchone()[0]
r = sub_("bot@example.com", trap="http://spam")
with db() as c: ok(c.execute("SELECT COUNT(*) FROM subscribers").fetchone()[0] == before, "campo trappola riempito: nessuna iscrizione")
ok(sub_("senza@example.com", consent=False).endswith("400"), "senza consenso all'informativa non ci si iscrive")
ok(sub_("non-una-email").endswith("400"), "un indirizzo non valido viene rifiutato")
print("== 3. Invio del riepilogo")
for t in ["Secondo articolo", "Terzo articolo"]:
    adm("/admin/edit/0", "-o", "/dev/null", "--form-string", f"title={t}", "--form-string", "body=<p>Testo.</p>", "--form-string", "status=published")
n = len(inbox)
adm("/admin/newsletter/invia", "-o", "/dev/null", "-X", "POST")
for _ in range(50):
    if '"running":false' in adm("/admin/newsletter/stato") and len(inbox) > n: break
    time.sleep(0.2)
got = inbox[n:]
ok(len(got) == 1 and got[0]["to"] == "lettore@example.com", "arriva solo agli iscritti confermati (non a chi è in attesa)")
d = got[0] if got else {"subject": "", "text": "", "unsub": "", "post": ""}
ok("Secondo articolo" in d["text"] and "Terzo articolo" in d["text"] and "Primo articolo" not in d["text"], "contiene gli articoli usciti dall'ultimo invio, non quelli già mandati")
ok(d["subject"].startswith("La Gazzetta del mattino: Terzo articolo"), "oggetto: titolo della newsletter e articolo più recente")
ok((d["unsub"] or "").startswith("<http") and d["post"] == "List-Unsubscribe=One-Click", "ha le intestazioni per annullare l'iscrizione con un clic")
unsub = B + urllib.parse.urlparse(d["unsub"].strip("<>")).path
ok("Sì, annulla" in curl("/tmp/nl-x.jar", unsub), "il link di disiscrizione chiede conferma (gli antivirus che aprono i link non disiscrivono nessuno)")
ok("Iscrizione annullata" in curl("/tmp/nl-x.jar", "-X", "POST", unsub), "con un clic sul pulsante l'iscrizione è annullata")
with db() as c: ok(c.execute("SELECT status FROM subscribers WHERE email = 'lettore@example.com'").fetchone()[0] == "unsubscribed", "e nella lista risulta annullata")
csv = adm("/admin/newsletter/iscritti.csv")
ok(csv.startswith("email,stato") and '"lettore@example.com","unsubscribed"' in csv, "gli iscritti si esportano in CSV (celle tra virgolette, protette dalle formule)")
page_ = adm("/admin/newsletter")
ok("Hanno annullato" in page_ and "La Gazzetta del mattino: Terzo articolo" in page_, "il pannello mostra i numeri e lo storico degli invii")
smtp.stop()
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
