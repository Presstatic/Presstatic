# Notifiche push: un finto servizio push (come quelli di Google o Mozilla) controlla la firma VAPID e decifra ogni
# notifica secondo lo standard (RFC 8291), con un'implementazione indipendente da quella del programma.
# Uso: python3 tests/push.py http://127.0.0.1:8212 /percorso/del/sito   (programma avviato con PRESSTATIC_TEST_LOCAL_FETCH=1)
import subprocess, sys, re, json, sqlite3, os, time, base64, threading, http.server, urllib.parse
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import encode_dss_signature
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.kdf.hkdf import HKDF
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
B, DIR = sys.argv[1], sys.argv[2]
PUB = f"{DIR}/public"
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
b64u = lambda b: base64.urlsafe_b64encode(b).rstrip(b"=").decode()
unb64u = lambda s: base64.urlsafe_b64decode(s + "=" * (-len(s) % 4))
got = []
class PushService(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        got.append({"path": self.path, "headers": dict(self.headers), "body": body})
        self.send_response(410 if self.path.endswith("/gone") else 201); self.end_headers()
    def log_message(self, *a): pass
threading.Thread(target=http.server.HTTPServer(("127.0.0.1", 8311), PushService).serve_forever, daemon=True).start()
ua = ec.generate_private_key(ec.SECP256R1()); ua_pub = ua.public_key().public_bytes(serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint)
auth = os.urandom(16)
def hk(salt, ikm, info, n): return HKDF(algorithm=hashes.SHA256(), length=n, salt=salt, info=info).derive(ikm)
def decrypt(body):
    salt, rs, idlen = body[:16], int.from_bytes(body[16:20], "big"), body[20]
    as_pub, ct = body[21:21 + idlen], body[21 + idlen:]
    shared = ua.exchange(ec.ECDH(), ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), as_pub))
    ikm = hk(auth, shared, b"WebPush: info\0" + ua_pub + as_pub, 32)
    pt = AESGCM(hk(salt, ikm, b"Content-Encoding: aes128gcm\0", 16)).decrypt(hk(salt, ikm, b"Content-Encoding: nonce\0", 12), ct, None)
    return rs, json.loads(pt.rstrip(b"\0")[:-1])  # delimitatore 0x02 dell'ultimo blocco
def vapid_ok(h, key):
    m = re.match(r"vapid t=([^,]+), k=(\S+)", h.get("Authorization", ""))
    if not m or m.group(2) != key: return False, "chiave assente o diversa"
    head, claims, sig = m.group(1).split(".")
    raw = unb64u(sig); der = encode_dss_signature(int.from_bytes(raw[:32], "big"), int.from_bytes(raw[32:], "big"))
    pk = ec.EllipticCurvePublicKey.from_encoded_point(ec.SECP256R1(), unb64u(key))
    try: pk.verify(der, f"{head}.{claims}".encode(), ec.ECDSA(hashes.SHA256()))
    except Exception: return False, "firma non valida"
    c = json.loads(unb64u(claims))
    return c["aud"] == "http://127.0.0.1:8311" and c["exp"] > time.time() and c["sub"], c
def curl(jar, *a): return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, *a], capture_output=True, text=True).stdout
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def page(slug): f = f"{PUB}/{slug}/index.html"; return open(f, encoding="utf-8").read() if os.path.isfile(f) else ""
A = "/tmp/pu-a.jar"; open(A, "w").close()
curl(A, "-o", "/dev/null", "--data-urlencode", "email=andrea@example.com", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
adm = lambda path, *a: curl(A, "-H", "Origin: " + B, *a, B + path)
def publish(title, slug, pid=0): return adm(f"/admin/edit/{pid}", "-o", "/dev/null", "-w", "%{redirect_url}", "--form-string", f"title={title}", "--form-string", f"slug={slug}", "--form-string", "body=<p>Testo.</p>", "--form-string", "status=published")
WWW = "https://www.giornale-di-prova.it"
def subscribe(endpoint, p256dh=b64u(ua_pub), a=b64u(auth)):
    return curl("/tmp/pu-x.jar", "-H", "Origin: " + WWW, "-H", "Content-Type: text/plain", "-w", "%{http_code}", "--data-binary", json.dumps({"endpoint": endpoint, "keys": {"p256dh": p256dh, "auth": a}}), B + "/push/iscrivi")
publish("Articolo di partenza", "partenza")
print("== 1. Spente e accese")
off = page("partenza")
ok("pushbox" not in off and not os.path.exists(f"{PUB}/sw.js"), "spente: niente pulsante, niente file sul sito")
sv = re.search(r'name="sv" value="(\d+)"', adm("/admin/push")).group(1)
adm("/admin/push", "-o", "/dev/null", "--data-urlencode", f"sv={sv}", "--data-urlencode", "push_on=on", "--data-urlencode", "push_on_publish=on")
with db() as c: key = c.execute("SELECT value FROM settings WHERE key = 'push_vapid_public'").fetchone()[0]
on = page("partenza")
ok(len(unb64u(key)) == 65 and unb64u(key)[0] == 4, "attivandole si crea la chiave VAPID (punto P-256 non compresso, 65 byte)")
ok(f'data-key="{key}"' in on and os.path.exists(f"{PUB}/sw.js") and os.path.exists(f"{PUB}/push.js"), "negli articoli c'è il pulsante; alla radice del sito service worker e script")
ok(on.count("<script") == off.count("<script"), "nessuno script caricato con la pagina: /push.js arriva solo al clic")
print("== 2. Iscrizioni")
ok(subscribe("http://127.0.0.1:8311/push/abc") == "204", "iscrizione dal sito accettata")
ok(subscribe("https://evil.example.com/push/abc").startswith("servizio push non riconosciuto"), "un indirizzo che non è di un servizio push viene rifiutato (il server non chiama indirizzi a caso)")
ok(subscribe("http://127.0.0.1:8311/push/abc", p256dh=b64u(b"x" * 10)).startswith("chiavi"), "chiavi dell'iscrizione non valide: rifiutata")
print("== 3. Notifica a ogni nuovo articolo")
got.clear(); r = publish("Chiusa la statale per una frana", "frana")
for _ in range(40):
    if got: break
    time.sleep(0.25)
ok(len(got) == 1, "pubblicato un articolo: parte una notifica")
if got:
    h = got[0]["headers"]
    ok(h.get("Content-Encoding") == "aes128gcm" and h.get("TTL") == "86400", "intestazioni dello standard (aes128gcm, durata 24 ore)")
    v, claims = vapid_ok(h, key)
    ok(v, f"firma VAPID verificata con la chiave pubblica del sito (destinatario {claims.get('aud') if isinstance(claims, dict) else claims})")
    rs, payload = decrypt(got[0]["body"])
    ok(rs == 4096 and payload["body"] == "Chiusa la statale per una frana" and payload["url"].endswith("/frana/"), "decifrata con le chiavi dell'iscritto: titolo e indirizzo dell'articolo giusti")
pid = int(re.search(r"/admin/edit/(\d+)", r).group(1))
got.clear(); publish("Chiusa la statale per una frana (aggiornato)", "frana", pid); time.sleep(3)
ok(not got, "correggendo l'articolo non parte un'altra notifica")
print("== 4. Invio a mano e iscrizioni scadute")
subscribe("http://127.0.0.1:8311/push/gone")
got.clear()
m = urllib.parse.unquote_plus(adm("/admin/push/invia", "-o", "/dev/null", "-w", "%{redirect_url}", "--data-urlencode", "title=Il Corriere", "--data-urlencode", "body=Ultim'ora: riaperta la statale", "--data-urlencode", "url=").split("msg=", 1)[-1])
ok("inviata a 1 iscritti" in m and "1 iscrizioni scadute tolte" in m, f"invio a mano: consegnata, e l'iscrizione scaduta (410) viene tolta ({m})")
ok(any(decrypt(g["body"])[1]["body"] == "Ultim'ora: riaperta la statale" for g in got if g["path"].endswith("/abc")), "il testo scritto a mano arriva cifrato e si decifra")
with db() as c: ok(c.execute("SELECT COUNT(*) FROM push_subs").fetchone()[0] == 1, "nel database resta solo l'iscrizione valida")
print("== 5. Rispente")
sv = re.search(r'name="sv" value="(\d+)"', adm("/admin/push")).group(1)
adm("/admin/push", "-o", "/dev/null", "--data-urlencode", f"sv={sv}")
ok("pushbox" not in page("partenza") and not os.path.exists(f"{PUB}/sw.js"), "spente: pulsante e file spariscono dal sito")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
