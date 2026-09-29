# Commenti statici: spenti non cambiano niente, accesi sono HTML senza JavaScript; moderazione, antispam, risposte.
# Uso: python3 tests/commenti.py http://127.0.0.1:8208 /percorso/del/sito
import subprocess, sys, re, sqlite3, os, urllib.parse, time
B, DIR = sys.argv[1], sys.argv[2]
PUB = f"{DIR}/public"
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
def curl(jar, *a): return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, *a], capture_output=True, text=True).stdout
def form(jar, path, fields):
    args = []
    for k, v in fields: args += ["--form-string", f"{k}={v}"]
    return curl(jar, "-H", "Origin: " + B, "-o", "/dev/null", "-w", "%{redirect_url}", *args, B + path)
def urlenc(jar, path, fields, origin=B):
    args = []
    for k, v in fields: args += ["--data-urlencode", f"{k}={v}"]
    return curl(jar, "-H", "Origin: " + origin, "-w", "\n%{http_code} %{redirect_url}", *args, B + path)
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def page(slug): f = f"{PUB}/{slug}/index.html"; return open(f, encoding="utf-8").read() if os.path.isfile(f) else ""
def setting(k, v):
    with db() as c: c.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (k, v))
A = "/tmp/cm-a.jar"; open(A, "w").close()
curl(A, "-o", "/dev/null", "--data-urlencode", "email=andrea@example.com", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
rebuild = lambda: curl(A, "-H", "Origin: " + B, "-o", "/dev/null", "-X", "POST", B + "/admin/rebuild")
r = form(A, "/admin/edit/0", [("title", "Nuova rotonda in centro"), ("slug", "rotonda"), ("body", "<p>Testo dell'articolo.</p>"), ("category", "Cronaca"), ("status", "published")])
pid = int(re.search(r"/admin/edit/(\d+)", r).group(1))
WWW = "https://www.giornale-di-prova.it"  # il modulo arriva dal sito pubblico, un altro dominio
send = lambda fields: urlenc("/tmp/cm-x.jar", f"/commenti/{pid}", fields, origin=WWW)
base_fields = lambda body, name="Giulia": [("name", name), ("email", "giulia@example.com"), ("body", body), ("consent", "on"), ("website", "")]

print("== 1. Spenti e accesi: cosa cambia nella pagina")
off = page("rotonda")
ok("commenti" not in off.lower().replace("commenti</a>", ""), "spenti: nella pagina non c'è niente dei commenti")
setting("comments_on", "on"); rebuild()
on = page("rotonda")
sec = on.split('<section class="comments"', 1)[-1].split("</section>", 1)[0] if "comments" in on else ""
ok(f'action="http://127.0.0.1:8080/commenti/{pid}"' in on or f'/commenti/{pid}"' in on, "accesi: c'è il modulo, che invia al pannello")
ok("<script" not in sec and on.count("<script") == off.count("<script"), "nessun JavaScript in più: né nel modulo né nella pagina")
print(f"   peso aggiunto alla pagina: {len(on.encode()) - len(off.encode())} byte di HTML e CSS, prima della compressione")

print("== 2. Invio e moderazione")
out = send(base_fields("Finalmente! Il traffico era impossibile. <b>Bravi</b>"))
code, loc = out.strip().rsplit("\n", 1)[-1].split(" ", 1)
ok(code == "303" and loc.endswith("/rotonda/#commento-inviato"), "inviato dal sito pubblico: si torna all'articolo con il messaggio di ringraziamento")
with db() as c: st, iph = c.execute("SELECT status, ip_hash FROM comments ORDER BY id DESC LIMIT 1").fetchone()
ok(st == "pending" and "Finalmente" not in page("rotonda"), "resta in attesa: sul sito non c'è ancora")
ok(len(iph) == 64 and "127.0.0.1" not in iph, "dell'indirizzo IP si salva solo l'impronta")
mod = curl(A, B + "/admin/commenti")
ok("Finalmente! Il traffico era impossibile." in mod and 'class="count">1<' in mod, "compare tra quelli da approvare, con il numero nel menu")
with db() as c: cid = c.execute("SELECT id FROM comments ORDER BY id DESC LIMIT 1").fetchone()[0]
urlenc(A, "/admin/commenti/azione", [("ids", str(cid)), ("action", "approve"), ("s", "pending")])
pg = page("rotonda")
ok("Finalmente! Il traffico era impossibile." in pg and "&lt;b&gt;Bravi&lt;/b&gt;" in pg and "1 commento" in pg, "approvato: è nella pagina, con l'HTML reso innocuo")
urlenc(A, f"/admin/commenti/{cid}/rispondi", [("body", "Grazie Giulia, seguiremo i lavori.")])
pg = page("rotonda")
ok('class="cm-replies"' in pg and "Grazie Giulia" in pg and "cm-badge" in pg, "la risposta della redazione compare sotto, con l'etichetta")

print("== 3. Antispam")
with db() as c: before = c.execute("SELECT COUNT(*) FROM comments").fetchone()[0]
out = send([("name", "Bot"), ("email", "b@b.it"), ("body", "Compra ora"), ("consent", "on"), ("website", "http://spam.example")])
with db() as c: after = c.execute("SELECT COUNT(*) FROM comments").fetchone()[0]
ok(out.rstrip().rsplit("\n", 1)[-1].startswith("303") and after == before, "campo trappola riempito: il programma di spam crede di aver inviato, ma non si salva niente")
send(base_fields("Guarda http://a.it http://b.it http://c.it", "Link"))
with db() as c: ok(c.execute("SELECT status FROM comments WHERE name = 'Link'").fetchone()[0] == "spam", "più di due link: direttamente tra lo spam")
out = send([("name", "Senza"), ("email", "s@s.it"), ("body", "Ciao a tutti"), ("website", "")])
ok(out.rstrip().rsplit("\n", 1)[-1].startswith("400") and "informativa" in out, "senza accettare l'informativa privacy non si invia")
for i in range(4): out = send(base_fields(f"Commento numero {i} per il limite", f"Utente{i}"))
ok("troppi commenti" in out, "dalla stessa connessione al massimo 5 commenti ogni 10 minuti")

print("== 4. Chiusura")
with db() as c: c.execute("DELETE FROM comments WHERE name LIKE 'Utente%'")
form(A, f"/admin/edit/{pid}", [("title", "Nuova rotonda in centro"), ("slug", "rotonda"), ("body", "<p>Testo dell'articolo.</p>"), ("category", "Cronaca"), ("status", "published"), ("comments_field", "1")])
pg = page("rotonda")
ok('<form class="cm-form"' not in pg and "I commenti sono chiusi." in pg and "Finalmente" in pg, "chiusi sull'articolo: niente modulo, i commenti già approvati restano")
ok("chiusi" in send(base_fields("Ancora uno?", "Tardivo")), "e un invio diretto viene rifiutato")
form(A, f"/admin/edit/{pid}", [("title", "Nuova rotonda in centro"), ("slug", "rotonda"), ("body", "<p>Testo dell'articolo.</p>"), ("category", "Cronaca"), ("status", "published"), ("comments_field", "1"), ("comments", "on")])
setting("comments_close_days", "1")
with db() as c: c.execute("UPDATE posts SET published_at = ? WHERE id = ?", (int(time.time()) - 3 * 86400, pid))
rebuild()
ok('<form class="cm-form"' not in page("rotonda"), "chiusi da soli dopo i giorni impostati")
urlenc(A, "/admin/commenti/azione", [("ids", str(cid)), ("action", "delete"), ("s", "approved")])
ok("Finalmente" not in page("rotonda") and "Grazie Giulia" not in page("rotonda"), "eliminato un commento: sparisce dalla pagina con la sua risposta")
setting("comments_on", ""); rebuild()
ok('class="comments"' not in page("rotonda"), "rispenti: la pagina torna come prima")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
