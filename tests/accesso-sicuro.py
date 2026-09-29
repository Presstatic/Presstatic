# Email, recupero della password, verifica in due passaggi e ricerca nel testo degli articoli.
# Le email arrivano a un server SMTP finto (aiosmtpd) che le conserva per il test.
# Uso: python3 tests/accesso-sicuro.py http://127.0.0.1:8203 /percorso/del/sito
import subprocess, sys, re, time, hmac, hashlib, struct, base64, sqlite3, urllib.parse, email, email.policy, quopri
from aiosmtpd.controller import Controller
B, DIR = sys.argv[1], sys.argv[2]
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)

inbox = []
class Box:
    async def handle_DATA(self, server, session, envelope):
        msg = email.message_from_bytes(envelope.content, policy=email.policy.default)
        text = msg.get_body(("plain",)).get_content() if msg.is_multipart() else msg.get_content()
        inbox.append({"to": envelope.rcpt_tos, "subject": msg["subject"], "text": text}); return "250 OK"
smtp = Controller(Box(), hostname="127.0.0.1", port=2526); smtp.start()

def curl(jar, *a): return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
def post(jar, path, fields, multipart=False):
    args = []
    for k, v in fields.items(): args += ["--form-string" if multipart else "--data-urlencode", f"{k}={v}"]
    return curl(jar, "-o", "/dev/null", "-w", "%{redirect_url}", *(args or ["-X", "POST"]), B + path)  # senza campi curl farebbe un GET
def msg(r): return urllib.parse.unquote_plus(r.split("msg=", 1)[-1]) if "msg=" in r else r
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def wait_mail(n, secs=10):
    end = time.time() + secs
    while time.time() < end and len(inbox) < n: time.sleep(0.2)
    return len(inbox) >= n
def totp(secret, t=None):
    key = base64.b32decode(secret.replace(" ", "") + "=" * (-len(secret.replace(" ", "")) % 8))
    h = hmac.new(key, struct.pack(">Q", int((t or time.time()) // 30)), hashlib.sha1).digest()
    o = h[19] & 15
    return "%06d" % ((struct.unpack(">I", h[o:o + 4])[0] & 0x7fffffff) % 1000000)
def login(jar, pw="password-lunga-123"):
    open(jar, "w").close()
    return post(jar, "/admin/login", {"email": "andrea@example.com", "password": pw})

A = "/tmp/as-a.jar"
login(A)
print("== 1. Configurazione e prova delle email")
sv = re.search(r'name="sv" value="(\d+)"', curl(A, B + "/admin/integrations")).group(1)
post(A, "/admin/integrations", {"sv": sv, "smtp_host": "127.0.0.1", "smtp_port": "2526", "smtp_security": "none", "smtp_from": "redazione@example.com", "smtp_from_name": "Redazione di prova", "admin_url": B})
m = msg(post(A, "/admin/integrations/email-test", {}))
ok("Email di prova inviata" in m and wait_mail(1) and inbox[0]["to"] == ["andrea@example.com"], "l'email di prova arriva all'amministratore")

print("== 2. Recupero della password")
m1 = msg(post("/tmp/as-x.jar", "/admin/recupero", {"email": "nessuno@example.com"}))
m2 = msg(post("/tmp/as-x.jar", "/admin/recupero", {"email": "andrea@example.com"}))
ok(m1 == m2 and "Se l'indirizzo corrisponde" in m1, "la risposta è identica per un'email esistente e per una inesistente")
ok(wait_mail(2) and len(inbox) == 2, "arriva un'email solo per l'account esistente")
link = re.search(r"(http\S+/admin/recupero/[0-9a-f]+)", inbox[1]["text"]).group(1)
ok(link.startswith(B + "/admin/recupero/"), "il link usa l'indirizzo del pannello impostato, non quello della richiesta")
with db() as c: stored = c.execute("SELECT token_hash FROM password_resets").fetchone()[0]
ok(link.rsplit("/", 1)[1] not in stored, "nel database c'è solo l'impronta del link, non il link")
S = "/tmp/as-s.jar"; login(S)  # una sessione aperta prima del cambio
path = link.replace(B, "")
ok("Nuova password" in curl("/tmp/as-x.jar", B + path), "il link apre la pagina per la nuova password")
ok("coincidono" in msg(post("/tmp/as-x.jar", path, {"password": "nuova-password-123", "password2": "diversa-password-1"})), "due password diverse vengono rifiutate")
ok("Password cambiata" in msg(post("/tmp/as-x.jar", path, {"password": "nuova-password-123", "password2": "nuova-password-123"})), "la password viene cambiata")
ok("non è più valido" in msg(curl("/tmp/as-x.jar", "-o", "/dev/null", "-w", "%{redirect_url}", B + path)), "il link non si può usare una seconda volta")
ok(curl(S, "-o", "/dev/null", "-w", "%{redirect_url}", B + "/admin").endswith("/admin/login"), "le sessioni aperte prima del cambio vengono chiuse")
ok(wait_mail(3) and "cambiata" in inbox[2]["subject"], "arriva l'avviso «La tua password è stata cambiata»")
ok("/admin/login?" in login(A, "password-lunga-123"), "la vecchia password non vale più")
ok(login(A, "nuova-password-123").endswith("/admin"), "si entra con la nuova password")

print("== 3. Verifica in due passaggi")
page = curl(A, B + "/admin/2fa")
key = re.search(r'class="tf-key">([A-Z2-7 ]+)<', page).group(1)
ok("<svg" in page and len(key.replace(" ", "")) == 32, "la pagina mostra il codice QR e la chiave da scrivere a mano")
ok("Codice non corretto" in msg(post(A, "/admin/2fa/enable", {"code": "000000"})), "un codice sbagliato non la attiva")
page = curl(A, "-X", "POST", "--data-urlencode", "code=" + totp(key), B + "/admin/2fa/enable")
codes = re.findall(r"<code>([a-z0-9]{4}-[a-z0-9]{4})</code>", page)
ok(len(codes) == 10, "con il codice giusto si attiva e mostra 10 codici di recupero")
with db() as c: ok(c.execute("SELECT COUNT(*) FROM recovery_codes WHERE code_hash = ?", (codes[0],)).fetchone()[0] == 0, "nel database i codici di recupero non sono in chiaro")
time.sleep(max(0, 31 - time.time() % 30))  # passo nuovo: il codice usato per attivarla non vale più
r = login(A, "nuova-password-123")
ok(r.endswith("/admin/login/2fa"), "dopo la password giusta serve il codice")
ok(curl(A, "-o", "/dev/null", "-w", "%{redirect_url}", B + "/admin").endswith("/admin/login"), "senza codice non si entra nel pannello")
ok("non corretto" in msg(post(A, "/admin/login/2fa", {"code": "123456"})), "un codice sbagliato viene rifiutato")
code = totp(key)
ok(post(A, "/admin/login/2fa", {"code": code}).endswith("/admin"), "con il codice dell'app si entra")
login(A, "nuova-password-123")
ok("non corretto" in msg(post(A, "/admin/login/2fa", {"code": code})), "lo stesso codice non vale una seconda volta")
ok(post(A, "/admin/login/2fa", {"code": codes[0].upper()}).endswith("/admin"), "un codice di recupero fa entrare")
login(A, "nuova-password-123")
ok("non corretto" in msg(post(A, "/admin/login/2fa", {"code": codes[0]})), "e non vale una seconda volta")
login(A, "nuova-password-123")
for _ in range(5): post(A, "/admin/login/2fa", {"code": "111111"})
ok("Troppi codici sbagliati" in msg(post(A, "/admin/login/2fa", {"code": totp(key)})), "dopo 5 codici sbagliati bisogna rifare l'accesso")
login(A, "nuova-password-123"); post(A, "/admin/login/2fa", {"code": codes[1]})
ok("2 passaggi" in curl(A, B + "/admin/users"), "nell'elenco utenti si vede chi ha la verifica attiva")
ok("Password non corretta" in msg(post(A, "/admin/2fa/disable", {"password": "sbagliata-sbagliata"})), "per disattivarla serve la password giusta")
ok("disattivata" in msg(post(A, "/admin/2fa/disable", {"password": "nuova-password-123"})), "con la password giusta si disattiva")
ok(login(A, "nuova-password-123").endswith("/admin"), "da disattivata basta di nuovo la password")

print("== 4. Ricerca nel titolo e nel testo")
post(A, "/admin/edit/0", {"title": "Consiglio comunale", "body": "<p>Approvato il piano per la <strong>viabilità</strong> della città vecchia.</p>", "category": "Politica", "status": "published"}, multipart=True)
lst = lambda q: curl(A, B + "/admin?q=" + urllib.parse.quote(q))
ok("Consiglio comunale" in lst("viabilita"), "trova una parola che è solo nel testo, anche senza accento")
ok("Consiglio comunale" in lst("citta vec"), "trova più parole anche scritte a metà")
ok("Consiglio comunale" not in lst("ferrovia"), "non trova parole che non ci sono")
with db() as c: pid = c.execute("SELECT id FROM posts WHERE title = 'Consiglio comunale'").fetchone()[0]
curl(A, "-o", "/dev/null", "-X", "POST", B + f"/admin/delete/{pid}")
ok("Consiglio comunale" not in lst("viabilita"), "dopo l'eliminazione non compare più")

smtp.stop()
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
