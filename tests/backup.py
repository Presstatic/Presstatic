# Backup su archivio compatibile S3. Il finto archivio verifica la firma di ogni richiesta con botocore (la libreria
# ufficiale di Amazon) e rifiuta quelle sbagliate; poi si scarica il backup, lo si apre e lo si ripristina davvero.
# Uso: python3 tests/backup.py http://127.0.0.1:8214 /percorso/del/sito /percorso/del/programma
import subprocess, sys, re, json, sqlite3, os, time, io, tarfile, hashlib, threading, http.server, urllib.parse, shutil, random
from botocore.auth import S3SigV4Auth
from botocore.awsrequest import AWSRequest
from botocore.credentials import Credentials
B, DIR, BIN = sys.argv[1], sys.argv[2], sys.argv[3]
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
AK, SK, BUCKET = "PRESSTATICTEST", "segreto-di-prova-1234567890", "presstatic-test"
objects, uploads, log = {}, {}, {"verified": 0, "bad": 0, "ops": []}
class S3(http.server.BaseHTTPRequestHandler):
    def reply(self, code, body=b"", headers=None):
        self.send_response(code)
        for k, v in (headers or {}).items(): self.send_header(k, v)
        self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
    def handle_any(self, method):
        body = self.rfile.read(int(self.headers.get("Content-Length", 0) or 0))
        m = re.match(r"AWS4-HMAC-SHA256 Credential=([^/]+)/(\d+)/([^/]+)/s3/aws4_request, SignedHeaders=([^,]+), Signature=(\w+)", self.headers.get("Authorization", ""))
        good = False
        if m and m.group(1) == AK:
            req = AWSRequest(method=method, url=f"http://{self.headers['Host']}{self.path}", data=body, headers={h: self.headers[h] for h in m.group(4).split(";")})
            req.context["timestamp"] = self.headers["x-amz-date"]
            signer = S3SigV4Auth(Credentials(AK, SK), "s3", m.group(3))
            good = signer.signature(signer.string_to_sign(req, signer.canonical_request(req)), req) == m.group(5) and hashlib.sha256(body).hexdigest() == self.headers["x-amz-content-sha256"]
        if not good:
            log["bad"] += 1; return self.reply(403, b"<Error><Code>SignatureDoesNotMatch</Code></Error>")
        log["verified"] += 1
        u = urllib.parse.urlparse(self.path); q = urllib.parse.parse_qs(u.query, keep_blank_values=True)
        parts = urllib.parse.unquote(u.path).lstrip("/").split("/", 1)
        if parts[0] != BUCKET: return self.reply(404, b"<Error><Code>NoSuchBucket</Code></Error>")
        key = parts[1] if len(parts) > 1 else ""
        if method == "PUT" and "partNumber" in q:
            uploads[q["uploadId"][0]][int(q["partNumber"][0])] = body; log["ops"].append("parte")
            return self.reply(200, headers={"ETag": '"' + hashlib.md5(body).hexdigest() + '"'})
        if method == "PUT": objects[key] = body; log["ops"].append("put"); return self.reply(200, headers={"ETag": '"x"'})
        if method == "POST" and "uploads" in q:
            uid = f"up{len(uploads) + 1}"; uploads[uid] = {}; log["ops"].append("inizio a pezzi")
            return self.reply(200, f"<InitiateMultipartUploadResult><UploadId>{uid}</UploadId></InitiateMultipartUploadResult>".encode())
        if method == "POST" and "uploadId" in q:
            p = uploads.pop(q["uploadId"][0]); nums = [int(x) for x in re.findall(rb"<PartNumber>(\d+)</PartNumber>", body)]
            objects[key] = b"".join(p[n] for n in nums); log["ops"].append("fine a pezzi")
            return self.reply(200, b"<CompleteMultipartUploadResult><Key>" + key.encode() + b"</Key></CompleteMultipartUploadResult>")
        if method == "DELETE": objects.pop(key, None); log["ops"].append("cancella"); return self.reply(204)
        if method == "GET":
            pre = q.get("prefix", [""])[0]
            xml = "".join(f"<Contents><Key>{k}</Key></Contents>" for k in sorted(objects) if k.startswith(pre))
            return self.reply(200, f"<ListBucketResult>{xml}</ListBucketResult>".encode())
        self.reply(400)
    def do_PUT(self): self.handle_any("PUT")
    def do_POST(self): self.handle_any("POST")
    def do_GET(self): self.handle_any("GET")
    def do_DELETE(self): self.handle_any("DELETE")
    def log_message(self, *a): pass
threading.Thread(target=http.server.ThreadingHTTPServer(("127.0.0.1", 8320), S3).serve_forever, daemon=True).start()
def curl(jar, *a): return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, *a], capture_output=True, text=True).stdout
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def setting(k, v):
    with db() as c: c.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (k, v))
A = "/tmp/bk-a.jar"; open(A, "w").close()
curl(A, "-o", "/dev/null", "--data-urlencode", "email=andrea@example.com", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
adm = lambda path, *a: curl(A, "-H", "Origin: " + B, *a, B + path)
msg = lambda r: urllib.parse.unquote_plus(r.split("msg=", 1)[-1])
from PIL import Image
Image.new("RGB", (1300, 800), (180, 40, 40)).save("/tmp/bk-foto.jpg")
img = json.loads(adm("/admin/media/upload", "-F", "files=@/tmp/bk-foto.jpg;filename=foto.jpg"))["items"][0]["url"]
adm("/admin/edit/0", "-o", "/dev/null", "--form-string", "title=Articolo da salvare", "--form-string", "slug=da-salvare", "--form-string", "body=<p>Testo importante.</p>", "--form-string", "status=published", "--form-string", f"image={img}")
os.makedirs(f"{DIR}/themes/mio-tema", exist_ok=True); open(f"{DIR}/themes/mio-tema/post.html", "w").write("tema personalizzato")
open(f"{DIR}/public/listino-prezzi.pdf", "wb").write(b"%PDF-1.4 documento caricato dal file manager")
for k, v in [("s3_on", "on"), ("s3_endpoint", "http://127.0.0.1:8320"), ("s3_region", "eu-central-1"), ("s3_bucket", BUCKET), ("s3_key", AK), ("s3_secret", "sbagliata"), ("s3_prefix", "miosito"), ("backup_keep", "3"), ("backup_local_keep", "2")]: setting(k, v)
print("== 1. Collegamento")
ok("rifiuta la firma" in msg(adm("/admin/backup/prova-s3", "-o", "/dev/null", "-w", "%{redirect_url}", "-X", "POST")), "con la chiave segreta sbagliata: «l'archivio rifiuta la firma»")
setting("s3_secret", SK); log["bad"] = 0
ok("Collegamento riuscito" in msg(adm("/admin/backup/prova-s3", "-o", "/dev/null", "-w", "%{redirect_url}", "-X", "POST")) and log["bad"] == 0, "con le chiavi giuste: collegamento riuscito, firme verificate da botocore")
def run_backup():
    adm("/admin/backup/esegui", "-o", "/dev/null", "-X", "POST")
    for _ in range(300):
        s = json.loads(adm("/admin/backup/stato"))
        if not s["running"] and s["last"]: return s["last"]
        time.sleep(0.2)
print("== 2. Backup")
last = run_backup()
keys = sorted(k for k in objects if k.startswith("miosito/presstatic-backup-"))
ok("Backup completato" in last and len(keys) == 1, f"backup completato e caricato nell'archivio ({last[:90]}…)")
t = tarfile.open(fileobj=io.BytesIO(objects[keys[0]]), mode="r:gz"); names = t.getnames()
ok("presstatic.db" in names and any(n.startswith("public/media/") for n in names) and "themes/mio-tema/post.html" in names and "public/listino-prezzi.pdf" in names, "dentro: database, immagini, temi personalizzati e file caricati")
ok(not any(n in names for n in ["public/index.html", "public/da-salvare/index.html"]) and not any(n.startswith("public/category") for n in names), "fuori: pagine e elenchi, che si rigenerano (archivio più piccolo)")
t.extract("presstatic.db", "/tmp/bk-estratto"); c = sqlite3.connect("/tmp/bk-estratto/presstatic.db")
ok(c.execute("SELECT COUNT(*) FROM posts WHERE slug = 'da-salvare'").fetchone()[0] == 1 and c.execute("PRAGMA integrity_check").fetchone()[0] == "ok", "il database nel backup è integro e contiene l'articolo")
print("== 3. Archivio grande: caricamento a pezzi")
big = random.randbytes(45 * 1024 * 1024); open(f"{DIR}/public/media/video-grande.bin", "wb").write(big)
log["ops"].clear(); time.sleep(1.1); last = run_backup()
ok("inizio a pezzi" in log["ops"] and log["ops"].count("parte") >= 2 and "fine a pezzi" in log["ops"], f"archivio di oltre 32 MB: caricato in {log['ops'].count('parte')} pezzi")
newest = sorted(k for k in objects if k.startswith("miosito/presstatic-backup-"))[-1]
t = tarfile.open(fileobj=io.BytesIO(objects[newest]), mode="r:gz")
ok(hashlib.sha256(t.extractfile("public/media/video-grande.bin").read()).digest() == hashlib.sha256(big).digest(), "ricomposto dall'archivio, il file grande è identico byte per byte")
os.remove(f"{DIR}/public/media/video-grande.bin")
print("== 4. Copie da conservare")
for _ in range(3): time.sleep(1.1); run_backup()
keys = sorted(k for k in objects if k.startswith("miosito/presstatic-backup-"))
ok(len(keys) == 3, f"dopo 5 backup, nell'archivio restano le ultime 3 copie ({len(keys)})")
local = sorted(f for f in os.listdir(f"{DIR}/backups") if f.startswith("presstatic-backup-"))
ok(len(local) == 2, "sul server restano le ultime 2 copie")
dl = subprocess.run(["curl", "-s", "-b", A, B + f"/admin/backup/scarica/{local[-1]}"], capture_output=True).stdout
ok(dl == open(f"{DIR}/backups/{local[-1]}", "rb").read(), "una copia sul server si scarica dal pannello, identica")
ok("404" == subprocess.run(["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", "-b", A, B + "/admin/backup/scarica/..%2Fpresstatic.db"], capture_output=True, text=True).stdout, "e non si può scaricare nient'altro dalla stessa rotta")
ok(log["bad"] == 0, f"tutte le {log['verified']} richieste avevano una firma valida")
print("== 5. Ripristino in una cartella nuova")
R = "/tmp/bk-ripristino"; shutil.rmtree(R, ignore_errors=True); os.makedirs(R)
shutil.copy(f"{DIR}/backups/{local[-1]}", R)
out = subprocess.run([BIN, "ripristina", local[-1]], cwd=R, capture_output=True, text=True)
ok(out.returncode == 0 and os.path.exists(f"{R}/presstatic.db") and os.path.exists(f"{R}/themes/mio-tema/post.html") and os.path.exists(f"{R}/public/listino-prezzi.pdf"), "«presstatic ripristina» rimette database, temi e file")
srv = subprocess.Popen([BIN], cwd=R, env=dict(os.environ, PRESSTATIC_ADDR="127.0.0.1:8215"), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
for _ in range(100):
    if os.path.exists(f"{R}/public/da-salvare/index.html") and not os.path.exists(f"{R}/.generazione-in-corso"): break
    time.sleep(0.2)
ok(os.path.exists(f"{R}/public/da-salvare/index.html") and os.path.exists(f"{R}/public" + img), "al primo avvio il sito si rigenera da solo: l'articolo è online, con la sua foto")
srv.kill()
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
