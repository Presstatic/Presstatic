# Prove delle correzioni dalle revisioni esterne: ogni blocco esegue l'attacco vero e controlla che ora fallisca.
# Uso: python3 tests/revisione-esterna.py http://127.0.0.1:PORTA /percorso/del/sito   (server avviato SENZA variabili di test)
import subprocess, sys, re, sqlite3, time, json, base64, hmac, hashlib, struct, email, email.policy, urllib.parse
from aiosmtpd.controller import Controller
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives import serialization
B, DIR = sys.argv[1], sys.argv[2]
fails = 0
def ok(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l, flush=True)
inbox = []
class Box:
    async def handle_DATA(self, s, ss, env): inbox.append(email.message_from_bytes(env.content, policy=email.policy.default)); return "250 OK"
Controller(Box(), hostname="127.0.0.1", port=2541).start()
def curl(jar, *a): return subprocess.run(["curl", "-s", "-m", "8", "-b", jar, "-c", jar, *a], capture_output=True, text=True).stdout
def db(): return sqlite3.connect(DIR + "/presstatic.db")
def setting(k, v):
    with db() as c: c.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (k, v))
msg = lambda r: urllib.parse.unquote_plus(r.split("msg=", 1)[-1]) if "msg=" in r else r
A = DIR + "/a.jar"; open(A, "w").close()
login = lambda jar, ip: curl(jar, "-o", "/dev/null", "-w", "%{redirect_url}", "-H", f"X-Real-IP: {ip}", "--data-urlencode", "email=a@x.it", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
login(A, "10.0.0.1")
for k, v in [("smtp_host", "127.0.0.1"), ("smtp_port", "2541"), ("smtp_security", "none"), ("smtp_from", "r@x.it"), ("admin_url", B), ("base_url", "https://www.sito.it")]: setting(k, v)

print("== 1. Verifica in due passaggi: niente tentativi infiniti rifacendo l'accesso")
with db() as c: c.execute("UPDATE users SET totp_on = 1, totp_secret = 'JBSWY3DPEHPK3PXP', totp_last = 0 WHERE email = 'a@x.it'")
def totp(secret, t):
    k = base64.b32decode(secret); h = hmac.new(k, struct.pack(">Q", int(t // 30)), hashlib.sha1).digest(); o = h[-1] & 15
    return str((struct.unpack(">I", h[o:o + 4])[0] & 0x7fffffff) % 1000000).zfill(6)
tried = 0; blocked_msg = ""
for round_ in range(3):                 # 3 accessi con la password giusta, 5 codici sbagliati ciascuno
    J = DIR + f"/t{round_}.jar"; open(J, "w").close(); login(J, f"10.0.1.{round_}")
    for i in range(5):
        r = msg(curl(J, "-o", "/dev/null", "-w", "%{redirect_url}", "--data-urlencode", "code=000000", B + "/admin/login/2fa"))
        tried += 1
        if "bloccato per 15 minuti" in r: blocked_msg = r; break
    if blocked_msg: break
ok(blocked_msg != "" and tried <= 11, f"dopo {tried} codici sbagliati il secondo passaggio è bloccato per l'account, anche rifacendo l'accesso")
J = DIR + "/tok.jar"; open(J, "w").close(); login(J, "10.0.2.9")
r = msg(curl(J, "-o", "/dev/null", "-w", "%{redirect_url}", "--data-urlencode", f"code={totp('JBSWY3DPEHPK3PXP', time.time())}", B + "/admin/login/2fa"))
ok("bloccato per 15 minuti" in r, "durante il blocco nemmeno il codice giusto entra (niente tentativi utili per chi attacca)")
time.sleep(2)
ok(any("Tentativi di accesso" in (m["subject"] or "") for m in inbox), "il titolare riceve un'email di avviso")
with db() as c: c.execute("UPDATE users SET totp_on = 0 WHERE email = 'a@x.it'")

print("== 2. Anteprima del builder: niente script di terzi sul dominio del pannello")
setting("head_scripts", "<script>window.TERZI_HEAD=1</script>"); setting("body_scripts", "<script>window.TERZI_BODY=1</script>"); setting("ad_top", "<script>window.ANNUNCIO=1</script>")
A2 = DIR + "/a2.jar"; open(A2, "w").close(); login(A2, "10.0.3.1")
h = subprocess.run(["curl", "-s", "-m", "20", "-D", "-", "-b", A2, B + "/admin/builder/frame"], capture_output=True, text=True).stdout
csp = next((l for l in h.lower().splitlines() if l.startswith("content-security-policy")), "")
ok("script-src 'self'" in csp and "frame-ancestors 'self'" in csp, f"l'anteprima ha la CSP che lascia girare solo gli script del pannello ({csp.strip()[:90]})")
found = [m for m in ("TERZI_HEAD", "TERZI_BODY", "ANNUNCIO") if m in h]
for m in found: i = h.index(m); print("      trovato", m, "in:", repr(h[max(0, i - 160):i + 20]))
ok(not found, "script dell'intestazione, del corpo e annunci non compaiono nell'anteprima")
for k in ("head_scripts", "body_scripts", "ad_top"): setting(k, "")

print("== 3. Notifiche push: l'elenco dei servizi ammessi non si aggira")
A3 = DIR + "/a3.jar"; open(A3, "w").close(); login(A3, "10.0.4.1")
sv = re.search(r'name="sv" value="(\d+)"', curl(A3, B + "/admin/push")).group(1)
curl(A3, "-o", "/dev/null", "-H", "Origin: " + B, "--data-urlencode", f"sv={sv}", "--data-urlencode", "push_on=on", B + "/admin/push")
pk = ec.generate_private_key(ec.SECP256R1()).public_key().public_bytes(serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint)
b64u = lambda b: base64.urlsafe_b64encode(b).rstrip(b"=").decode()
def sub(ep, ip): return curl(DIR + "/x.jar", "-w", "|%{http_code}", "-H", f"X-Real-IP: {ip}", "-H", "Content-Type: text/plain", "--data-binary", json.dumps({"endpoint": ep, "keys": {"p256dh": b64u(pk), "auth": b64u(b"0123456789abcdef")}}), B + "/push/iscrivi")
for i, ep in enumerate(["https://attaccante.example?.push.apple.com/x", "https://attaccante.example#.push.apple.com", "https://fcm.googleapis.com@attaccante.example/x", "https://127.0.0.1?.notify.windows.com/x", "https://web.push.apple.com:8443/x"]):
    ok(sub(ep, f"10.9.0.{i}").endswith("|400"), f"rifiutato: {ep}")
ok(sub("https://web.push.apple.com/QGuQyavXutnMH", "10.9.1.1").endswith("|204"), "accettato un indirizzo vero di Apple")
ok(sub("https://wns2-par02p.notify.windows.com/w/?token=BQYAAABx", "10.9.1.2").endswith("|204"), "accettato un indirizzo vero di Windows, con la query string")

print("== 4. Recupero password: niente raffiche di email alla stessa persona")
inbox.clear()
for i in range(6): curl(DIR + "/r.jar", "-o", "/dev/null", "-H", f"X-Real-IP: 172.16.{i}.1", "--data-urlencode", "email=a@x.it", B + "/admin/recupero")
time.sleep(3)
n = sum(1 for m in inbox if "Nuova password" in (m["subject"] or ""))
ok(n <= 3, f"6 richieste da 6 indirizzi diversi: partono {n} email (al massimo 3 ogni 15 minuti)")
with db() as c: c.execute("UPDATE password_resets SET expires = 0"); 
curl(DIR + "/r.jar", "-o", "/dev/null", "-H", "X-Real-IP: 172.17.0.1", "--data-urlencode", "email=nessuno@x.it", B + "/admin/recupero")

print("== 5. Newsletter: niente raffiche di conferme a un indirizzo altrui")
setting("newsletter_on", "on"); inbox.clear()
for i in range(4): curl(DIR + "/n.jar", "-o", "/dev/null", "-H", "Origin: https://www.sito.it", "-H", f"X-Real-IP: 192.0.2.{i}", "--data-urlencode", "email=vittima@esempio.it", "--data-urlencode", "consent=on", "--data-urlencode", "website=", B + "/newsletter/iscriviti")
time.sleep(3)
n = sum(1 for m in inbox if m["to"] == "vittima@esempio.it")
ok(n == 1, f"4 iscrizioni dello stesso indirizzo da 4 IP diversi: {n} email di conferma (al massimo 1 ogni 12 ore)")

print("== 6. Impronta degli IP: con chiave segreta, e senza collisioni")
for ip, em in [("1.23.4.5", "uno@esempio.it"), ("12.3.4.5", "due@esempio.it")]:
    curl(DIR + "/n2.jar", "-o", "/dev/null", "-H", "Origin: https://www.sito.it", "-H", f"X-Real-IP: {ip}", "--data-urlencode", f"email={em}", "--data-urlencode", "consent=on", "--data-urlencode", "website=", B + "/newsletter/iscriviti")
with db() as c: h1, h2 = [r[0] for r in c.execute("SELECT ip_hash FROM subscribers WHERE email IN ('uno@esempio.it','due@esempio.it') ORDER BY email DESC")]
ok(h1 != h2, "1.23.4.5 e 12.3.4.5 hanno impronte diverse (prima erano uguali)")
ok(h1 != hashlib.sha256(b"1.23.4.5").hexdigest() and h1 != hashlib.sha256(b"12345").hexdigest(), "l'impronta non è uno SHA-256 semplice: senza la chiave del sito non si risale all'IP")

print("== 7. Intestazioni di sicurezza del pannello")
h = subprocess.run(["curl", "-s", "-D", "-", "-o", "/dev/null", B + "/admin/login"], capture_output=True, text=True).stdout.lower()
ok("object-src 'none'" in h and "base-uri 'self'" in h and "form-action 'self'" in h and "frame-ancestors 'self'" in h, "il pannello manda una CSP di base (niente plugin, base e destinazioni dei moduli bloccate)")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
