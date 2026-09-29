# Redazione al lavoro nello stesso momento.
#  1. Blocco degli articoli: chi apre un articolo già aperto da un altro lo vede, può subentrare, l'altro viene avvisato.
#  2. Conflitto al salvataggio: chi salva sopra il lavoro di un altro non lo sovrascrive; la sua versione va nella cronologia.
#  3. Pubblicazioni simultanee: due articoli pubblicati nello stesso istante finiscono entrambi in home.
#  4. Versione e cronologia salvate insieme all'articolo.
# Uso: python3 tests/redazione-contemporanea.py http://127.0.0.1:8198 /percorso/del/sito
import subprocess, sys, re, json, sqlite3, threading, time, urllib.parse
B, DIR = sys.argv[1], sys.argv[2]
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
def curl(jar, *a):
    return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
def get(jar, path): return curl(jar, B + path)
def post(jar, path, fields=None, form=False):
    args = []
    for k, v in (fields or {}).items(): args += ["--form-string" if form else "--data-urlencode", f"{k}={v}"]
    return curl(jar, "-o", "/dev/null", "-w", "%{redirect_url}", *args, "-X", "POST", B + path)
def msg(r): return urllib.parse.unquote_plus(r.split("msg=", 1)[-1]) if "msg=" in r else r
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def article(jar, title, body, status="published", pid=0, version=None):
    f = {"title": title, "description": "Sommario di prova", "body": body, "category": "Cronaca", "status": status}
    if version is not None: f["version"] = str(version)
    return post(jar, f"/admin/edit/{pid}", f, form=True)

A, G = "/tmp/rc-a.jar", "/tmp/rc-g.jar"
for j in (A, G): open(j, "w").close()
post(A, "/admin/login", {"email": "andrea@example.com", "password": "password-lunga-123"})
post(A, "/admin/users/0", {"name": "Giulia Neri", "email": "giulia@example.com", "password": "password-lunga-123", "role": "editor"}, form=True)
post(G, "/admin/login", {"email": "giulia@example.com", "password": "password-lunga-123"})
r = article(A, "Articolo condiviso", "<p>Prima stesura di Andrea.</p>")
pid = int(re.search(r"/admin/edit/(\d+)", r).group(1))

print("== 1. Blocco degli articoli")
ea = get(A, f"/admin/edit/{pid}")
ok("sta modificando" not in ea, "Andrea apre l'articolo: nessun avviso, l'articolo è suo")
eg = get(G, f"/admin/edit/{pid}")
ok("Andrea Admin sta modificando questo articolo" in eg and "Subentra" in eg, "Giulia lo apre: vede che Andrea lo sta modificando e può subentrare")
lst = get(G, "/admin")
ok("Andrea Admin lo sta modificando" in lst, "nell'elenco articoli Giulia vede chi lo sta modificando")
r = post(G, f"/admin/lock/{pid}/take")
ok(r.endswith(f"/admin/edit/{pid}"), "Giulia preme Subentra e torna nell'editor")
ok("sta modificando" not in get(G, f"/admin/edit/{pid}"), "ora l'articolo è di Giulia: nessun avviso per lei")
ping = json.loads(curl(A, "-X", "POST", B + f"/admin/lock/{pid}"))
ok(ping.get("taken_by") == "Giulia Neri", "al segnale successivo l'editor di Andrea sa che Giulia è subentrata")
ping = json.loads(curl(G, "-X", "POST", B + f"/admin/lock/{pid}"))
ok(ping.get("taken_by") is None, "il segnale di Giulia rinnova il suo blocco")
curl(G, "-X", "POST", B + f"/admin/lock/{pid}/release")
ok("sta modificando" not in get(A, f"/admin/edit/{pid}"), "Giulia chiude l'editor: l'articolo si libera subito")
with db() as c: c.execute("UPDATE post_locks SET at = at - 200 WHERE post_id = ?", (pid,))
ok("sta modificando" not in get(G, f"/admin/edit/{pid}"), "un blocco senza segnale da più di due minuti e mezzo scade da solo")

print("== 2. Conflitto al salvataggio")
v0 = int(re.search(r'name="version" value="(\d+)"', get(A, f"/admin/edit/{pid}")).group(1))
m = msg(article(G, "Articolo condiviso", "<p>Versione di Giulia.</p>", pid=pid, version=v0))
ok(m.startswith("Articolo pubblicato"), "Giulia salva per prima: salvato")
m = msg(article(A, "Articolo condiviso", "<p>Versione di Andrea, scritta intanto.</p>", pid=pid, version=v0))
ok("nel frattempo Giulia Neri ha salvato" in m, "Andrea salva dopo, partendo dalla versione vecchia: avvisato, niente sovrascrittura")
with db() as c:
    body = c.execute("SELECT body FROM posts WHERE id = ?", (pid,)).fetchone()[0]
    top = c.execute("SELECT body FROM revisions WHERE post_id = ? ORDER BY id DESC LIMIT 1", (pid,)).fetchone()[0]
ok("Versione di Giulia" in body, "l'articolo resta quello di Giulia")
ok("Versione di Andrea" in top, "il testo di Andrea è in cima alla cronologia, da confrontare e ripristinare")
v1 = int(re.search(r'name="version" value="(\d+)"', get(A, f"/admin/edit/{pid}")).group(1))
ok(v1 == v0 + 1, "il numero di versione è cresciuto di uno")
ok(msg(article(A, "Articolo condiviso", "<p>Andrea riparte dalla versione di Giulia.</p>", pid=pid, version=v1)).startswith("Articolo pubblicato"), "ricaricato l'articolo, Andrea salva senza problemi")
ok(msg(article(A, "Articolo condiviso", "<p>Salvataggio senza numero di versione.</p>", pid=pid)).startswith("Articolo pubblicato"), "un salvataggio senza numero di versione (strumenti, ripristino) funziona come prima")

print("== 3. Pubblicazioni simultanee")
missing = []
for n in range(10):
    titles = [f"Simultaneo {n} di Andrea", f"Simultaneo {n} di Giulia"]
    ts = [threading.Thread(target=article, args=(j, t, f"<p>{t}</p>")) for j, t in zip((A, G), titles)]
    for t in ts: t.start()
    for t in ts: t.join()
    home = open(f"{DIR}/public/index.html", encoding="utf-8").read()
    missing += [t for t in titles if t not in home]
ok(not missing, f"10 coppie di articoli pubblicati nello stesso istante: tutti in home" + (f" (mancano: {missing})" if missing else ""))

print("== 4. Versione e cronologia insieme all'articolo")
with db() as c:
    before = c.execute("SELECT version, (SELECT COUNT(*) FROM revisions WHERE post_id = ?) FROM posts WHERE id = ?", (pid, pid)).fetchone()
article(A, "Articolo condiviso", "<p>Ultima modifica.</p>", pid=pid)
with db() as c:
    after = c.execute("SELECT version, (SELECT COUNT(*) FROM revisions WHERE post_id = ?) FROM posts WHERE id = ?", (pid, pid)).fetchone()
ok(after[0] == before[0] + 1 and after[1] == before[1] + 1, "un salvataggio aggiorna articolo, versione e cronologia insieme")

print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
