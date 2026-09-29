# IndexNow: file di chiave, invii a pubblicazione, modifica, cambio di indirizzo ed eliminazione, prova, errori.
# Uso: python3 tests/indexnow.py http://127.0.0.1:8216 /percorso/del/sito   (programma avviato con PRESSTATIC_INDEXNOW_API=http://127.0.0.1:8330/indexnow)
import subprocess, sys, re, json, os, time, threading, http.server, urllib.parse, sqlite3
B, DIR = sys.argv[1], sys.argv[2]
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
got, mode = [], {"code": 202}
class IX(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        got.append({"ct": self.headers.get("Content-Type"), "body": json.loads(self.rfile.read(int(self.headers["Content-Length"])))})
        self.send_response(mode["code"]); self.end_headers()
    def log_message(self, *a): pass
threading.Thread(target=http.server.HTTPServer(("127.0.0.1", 8330), IX).serve_forever, daemon=True).start()
J = "/tmp/ix.jar"; open(J, "w").close()
def curl(*a): return subprocess.run(["curl", "-s", "-b", J, "-c", J, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
msg = lambda r: urllib.parse.unquote_plus(r.split("msg=", 1)[-1])
curl("-o", "/dev/null", "--data-urlencode", "email=andrea@example.com", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
def save(title, slug, pid=0, status="published"):
    return curl("-o", "/dev/null", "-w", "%{redirect_url}", "--form-string", f"title={title}", "--form-string", f"slug={slug}", "--form-string", "body=<p>Testo.</p>", "--form-string", f"status={status}", B + f"/admin/edit/{pid}")
def settings(**kv):
    sv = re.search(r'name="sv" value="(\d+)"', curl(B + "/admin/indicizzazione")).group(1)
    args = ["--data-urlencode", f"sv={sv}"] + [x for k, v in kv.items() for x in ("--data-urlencode", f"{k}={v}")]
    return msg(curl("-o", "/dev/null", "-w", "%{redirect_url}", *args, B + "/admin/indicizzazione"))
save("Prima notizia", "prima"); time.sleep(0.5)
ok(not got, "spento: pubblicando non parte niente")
ok("Indicizzazione rapida</a>" in curl(B + "/admin"), "nella barra laterale la voce si chiama «Indicizzazione rapida»")
settings(indexnow_on="on")
with sqlite3.connect(f"{DIR}/presstatic.db") as c: key = c.execute("SELECT value FROM settings WHERE key = 'indexnow_key'").fetchone()[0]
ok(re.fullmatch(r"[0-9a-f]{32}", key) and open(f"{DIR}/public/{key}.txt").read() == key, "acceso: chiave di 32 caratteri e file di verifica alla radice del sito, con dentro la chiave")
ok(f"{key}.txt" in curl(B + "/admin/indicizzazione"), "la pagina mostra la chiave e il link al file di verifica")
r = save("Nuova notizia", "nuova"); pid = re.search(r"/admin/edit/(\d+)", r).group(1)
b = got[-1]["body"] if got else {}
ok(b.get("host") == "127.0.0.1:8080" and b.get("key") == key and b.get("keyLocation") == f"http://127.0.0.1:8080/{key}.txt" and b.get("urlList") == ["http://127.0.0.1:8080/nuova/"] and "json" in got[-1]["ct"], "pubblicazione: invio con dominio, chiave, file di verifica e indirizzo dell'articolo")
ok("IndexNow avvisato (1 indirizzo)" in msg(r), "il messaggio dopo la pubblicazione lo dice")
n = len(got); save("Nuova notizia, aggiornata", "nuova", pid)
ok(len(got) == n + 1, "anche una semplice modifica viene inviata (IndexNow non ha quote)")
save("Nuova notizia, aggiornata", "nuova-titolo", pid)
ok(sorted(got[-1]["body"]["urlList"]) == ["http://127.0.0.1:8080/nuova-titolo/", "http://127.0.0.1:8080/nuova/"], "cambio di indirizzo: si inviano il nuovo e il vecchio")
curl("-o", "/dev/null", "-X", "POST", B + f"/admin/delete/{pid}")
ok(got[-1]["body"]["urlList"] == ["http://127.0.0.1:8080/nuova-titolo/"], "eliminazione: si invia l'indirizzo tolto, così i motori lo rimuovono")
m = msg(curl("-o", "/dev/null", "-w", "%{redirect_url}", "-X", "POST", B + "/admin/indicizzazione/indexnow"))
ok("Prova riuscita" in m and got[-1]["body"]["urlList"] == ["http://127.0.0.1:8080/"], "«Prova IndexNow» invia la home")
ok("· IndexNow" in curl(B + "/admin/indicizzazione"), "gli invii compaiono negli ultimi invii, con la scritta IndexNow")
mode["code"] = 403
m = msg(curl("-o", "/dev/null", "-w", "%{redirect_url}", "-X", "POST", B + "/admin/indicizzazione/indexnow"))
ok("chiave non riconosciuta" in m, "se i motori rifiutano la chiave, il messaggio spiega cosa controllare")
mode["code"] = 202
settings()
ok(not os.path.exists(f"{DIR}/public/{key}.txt"), "rispento: il file di verifica sparisce dal sito")
n = len(got); save("Dopo lo spegnimento", "dopo"); time.sleep(0.5)
ok(len(got) == n, "e pubblicando non parte più niente")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
