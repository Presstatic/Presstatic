# Aggiornamento dal pannello mentre il sito si sta rigenerando: il programma deve uscire solo dopo aver finito.
# Una release finta, firmata con una chiave generata qui, viene servita da un finto GitHub locale.
# Uso: python3 tests/aggiornamento-durante-pubblicazione.py /percorso/presstatic /percorso/database-con-molti-articoli.db
import subprocess, sys, os, json, time, shutil, hashlib, threading, http.server
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
BIN, DBSRC = sys.argv[1], sys.argv[2]
W = "/tmp/upd"; API, PORT = 8301, 8302
B = f"http://127.0.0.1:{PORT}"
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)

shutil.rmtree(W, ignore_errors=True); os.makedirs(f"{W}/bin"); os.makedirs(f"{W}/sito")
shutil.copy(BIN, f"{W}/bin/presstatic"); shutil.copy(DBSRC, f"{W}/sito/presstatic.db")
# la "nuova versione": risponde come Presstatic 1.0.1 alla prova generale che il programma fa prima di installarla
new_bin = b"#!/bin/sh\necho 'Presstatic 1.0.1'\n"
key = Ed25519PrivateKey.generate()
pub = key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()
sig = key.sign(f"1.0.1\n{hashlib.sha256(new_bin).hexdigest()}".encode())
release = {"tag_name": "v1.0.1", "body": "Versione di prova", "assets": [
    {"name": "presstatic-linux-x86_64", "browser_download_url": f"http://127.0.0.1:{API}/bin"},
    {"name": "presstatic-linux-x86_64.sig", "browser_download_url": f"http://127.0.0.1:{API}/bin.sig"}]}
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = {"/repos/prova/presstatic/releases/latest": json.dumps(release).encode(), "/bin": new_bin, "/bin.sig": sig}.get(self.path)
        self.send_response(200 if body else 404); self.end_headers(); self.wfile.write(body or b"")
    def log_message(self, *a): pass
threading.Thread(target=http.server.HTTPServer(("127.0.0.1", API), H).serve_forever, daemon=True).start()

env = dict(os.environ, PRESSTATIC_ADDR=f"127.0.0.1:{PORT}", PRESSTATIC_UPDATE_API=f"http://127.0.0.1:{API}", PRESSTATIC_UPDATE_REPO="prova/presstatic", PRESSTATIC_UPDATE_KEY=pub)
proc = subprocess.Popen([f"{W}/bin/presstatic"], cwd=f"{W}/sito", env=env, stdout=open(f"{W}/s.log", "w"), stderr=subprocess.STDOUT)
for _ in range(50):
    if subprocess.run(["curl", "-s", "-o", "/dev/null", B + "/admin/login"]).returncode == 0: break
    time.sleep(0.2)
jar = f"{W}/k.jar"
def curl(*a): return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
curl("-o", "/dev/null", "--data-urlencode", "email=admin@x.it", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
shutil.rmtree(f"{W}/sito/public", ignore_errors=True)  # rigenerazione da zero: dura parecchi secondi

mark = f"{W}/sito/.generazione-in-corso"
t = {}
def rebuild():
    t["rebuild_start"] = time.time(); curl("-o", "/dev/null", "--max-time", "280", "-X", "POST", B + "/admin/rebuild"); t["rebuild_answer"] = time.time()
threading.Thread(target=rebuild).start()
while not os.path.exists(mark): time.sleep(0.05)
time.sleep(2)
page = curl("-X", "POST", B + "/admin/update/check")  # senza -L: seguire il reindirizzamento ripeterebbe un POST, cioè «Aggiorna ora»
upd = curl(B + "/admin/update")
ok("1.0.1" in upd, "il pannello trova la versione 1.0.1 sul finto GitHub")
if "1.0.1" not in upd: print("   pagina:", upd[:300].replace("\n", " "), "| dopo il controllo:", page[:200].replace("\n", " "))
t["update"] = time.time()
out = curl("-X", "POST", B + "/admin/update")
ok("riavviando" in out, "l'aggiornamento viene installato e il pannello annuncia il riavvio")
if "riavviando" not in out: print("   risposta:", out[:300].replace("\n", " "))
ok(os.path.exists(mark), "in quel momento la rigenerazione è ancora in corso")
gen_end = None
while proc.poll() is None:
    if gen_end is None and not os.path.exists(mark): gen_end = time.time()
    time.sleep(0.02)
t["exit"] = time.time()
gen_end = gen_end or t["exit"]
print(f"   rigenerazione iniziata a 0 s, aggiornamento a {t['update'] - t['rebuild_start']:.1f} s, "
      f"rigenerazione finita a {gen_end - t['rebuild_start']:.1f} s, uscita del programma a {t['exit'] - t['rebuild_start']:.1f} s")
ok(t["exit"] - t["update"] > 3, "il programma non esce 1,5 secondi dopo l'aggiornamento, come prima: aspetta")
ok(gen_end <= t["exit"] and not os.path.exists(mark), "esce solo dopo che la rigenerazione è finita, senza lasciare il segno di lavoro interrotto")
ok(os.path.isfile(f"{W}/sito/public/index.html"), "il sito rigenerato è completo: la home c'è")
ok(open(f"{W}/bin/presstatic", "rb").read() == new_bin and os.path.isfile(f"{W}/bin/presstatic.old"), "il programma nuovo è al suo posto, il vecchio in presstatic.old")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
