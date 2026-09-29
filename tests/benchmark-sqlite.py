#!/usr/bin/env python3
"""Benchmark di SQLite con le query reali di Presstatic.

Non tocca mai il database del sito: lavora su database di prova creati in una cartella temporanea
(con --reale ne usa una COPIA). Usa solo Python 3 standard, niente da installare.

    python3 benchmark-sqlite.py                      # prova completa, circa 6-8 minuti
    python3 benchmark-sqlite.py --rapido             # prova veloce, circa 2 minuti
    python3 benchmark-sqlite.py --dimensioni 10000,100000,1000000 --passo 8
    python3 benchmark-sqlite.py --reale /var/www/presstatic/presstatic.db

Alla fine stampa le tabelle e salva un rapporto con i grafici in benchmark-sqlite.html.
"""
import argparse, json, multiprocessing as mp, os, random, shutil, signal, sqlite3, statistics, subprocess, sys, tempfile, threading, time

COLS = ("p.id, p.slug, p.title, p.description, p.body, p.category, p.image, p.schema_type, p.schema_data, p.status, p.published_at, "
        "p.updated_at, p.kind, p.tags, COALESCE(p.author_id, 0), COALESCE(u.name, ''), COALESCE(u.slug, ''), COALESCE(u.bio, ''), "
        "COALESCE(u.photo, ''), p.featured, p.link_keywords")
NOBODY = COLS.replace("p.body", "''")
BASE = "FROM posts p LEFT JOIN users u ON u.id = p.author_id"
SCHEMA = """
PRAGMA journal_mode=WAL;
CREATE TABLE users(id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT UNIQUE NOT NULL, pass TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT 'author', bio TEXT NOT NULL DEFAULT '', photo TEXT NOT NULL DEFAULT '', slug TEXT NOT NULL DEFAULT '');
CREATE TABLE posts(id INTEGER PRIMARY KEY, slug TEXT UNIQUE NOT NULL, title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
  body TEXT NOT NULL DEFAULT '', category TEXT NOT NULL DEFAULT '', image TEXT NOT NULL DEFAULT '', schema_type TEXT NOT NULL DEFAULT 'NewsArticle',
  schema_data TEXT NOT NULL DEFAULT '{}', status TEXT NOT NULL DEFAULT 'draft', published_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
  author_id INTEGER, kind TEXT NOT NULL DEFAULT 'post', tags TEXT NOT NULL DEFAULT '', featured INTEGER NOT NULL DEFAULT 0, link_keywords TEXT NOT NULL DEFAULT '');
CREATE INDEX posts_pub ON posts(status, published_at);
CREATE INDEX posts_cat ON posts(category, published_at);
CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE revisions(id INTEGER PRIMARY KEY, post_id INTEGER NOT NULL, user_id INTEGER, title TEXT NOT NULL, description TEXT NOT NULL, body TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX revisions_post ON revisions(post_id, id);
"""
WORDS = ("comune sindaco lavori strada ponte fiume scuola ospedale piazza mercato calcio squadra partita cultura mostra teatro ambiente "
         "parco raccolta rifiuti regione bilancio consiglio cittadini quartiere stazione treno autobus estate inverno").split()
CATS = ["Cronaca", "Politica", "Economia", "Sport", "Cultura", "Ambiente"]
NOW = int(time.time())


def text(rnd, n): return " ".join(rnd.choice(WORDS) for _ in range(n))


def body(rnd):
    p = [f"<p>{text(rnd, 80)}</p>" for _ in range(6)]
    p.insert(2, '<img src="/media/2026/09/foto-1600x900.jpg" alt="">')
    return "".join(p)  # circa 5 KB, come un articolo vero


def connect(path, timeout=5.0):
    c = sqlite3.connect(path, timeout=timeout, check_same_thread=False)  # 5 s di attesa, come Presstatic
    return c


def build(path, n):
    rnd = random.Random(1)
    c = sqlite3.connect(path)
    c.executescript(SCHEMA)
    c.executemany("INSERT INTO users(id, name, email, pass, bio, slug) VALUES (?,?,?,?,?,?)",
                  [(i, f"Autore {i}", f"a{i}@x.it", "x", text(rnd, 18), f"autore-{i}") for i in range(1, 11)])
    c.executemany("INSERT INTO settings VALUES (?,?)", [(f"chiave{i}", text(rnd, 4)) for i in range(40)])
    bodies = [body(rnd) for _ in range(200)]  # 200 testi diversi riusati: generazione veloce anche per milioni di righe
    chunk = []
    for i in range(1, n + 1):
        chunk.append((f"articolo-{i}", text(rnd, 10), text(rnd, 22), bodies[i % 200], CATS[i % 6], f"/media/2026/09/foto-{i % 50}-1600x900.jpg",
                       "published", NOW - (n - i) * 60, NOW - (n - i) * 60, i % 10 + 1, "tag1, tag2, tag3", f"parola chiave {i}" if i % 40 == 0 else ""))
        if len(chunk) == 5000 or i == n:
            c.executemany("INSERT INTO posts(slug, title, description, body, category, image, status, published_at, updated_at, author_id, tags, link_keywords) "
                          "VALUES (?,?,?,?,?,?,?,?,?,?,?,?)", chunk)
            chunk = []
    c.commit()
    c.close()


# ---------------------------------------------------------------- operazioni (le stesse query di Presstatic)
def op_articolo(c, n, rnd): c.execute(f"SELECT {COLS} {BASE} WHERE p.id = ?", (rnd.randint(1, n),)).fetchall()
def op_home(c, n, rnd): c.execute(f"SELECT {NOBODY} {BASE} WHERE p.kind = 'post' AND p.status = 'published' AND p.published_at <= ? ORDER BY p.published_at DESC LIMIT 20", (NOW,)).fetchall()
def op_categoria(c, n, rnd): c.execute(f"SELECT {NOBODY} {BASE} WHERE p.kind = 'post' AND p.status = 'published' AND p.published_at <= ? AND p.category = ? ORDER BY p.published_at DESC LIMIT 20", (NOW, rnd.choice(CATS))).fetchall()
def op_correlati(c, n, rnd): c.execute(f"SELECT {NOBODY} {BASE} WHERE p.kind = 'post' AND p.status = 'published' AND p.published_at <= ? AND p.category = ? AND p.id <> ? ORDER BY p.published_at DESC LIMIT 3", (NOW, rnd.choice(CATS), rnd.randint(1, n))).fetchall()
def op_pannello(c, n, rnd): c.execute(f"SELECT {NOBODY} {BASE} WHERE p.kind = 'post' AND p.title LIKE ? ORDER BY p.published_at DESC LIMIT 300", ("%",)).fetchall()
def op_link(c, n, rnd): c.execute(f"SELECT {COLS} {BASE} WHERE p.id <> 0 AND p.kind = 'post' AND p.status = 'published' AND p.published_at <= ? AND (p.body LIKE ? OR p.body LIKE ?) LIMIT 1000", (NOW, "%ponte sul fiume%", "%parola rara%")).fetchall()
def op_rigenerazione(c, n, rnd):
    last = 0
    while True:
        b = c.execute(f"SELECT {COLS} {BASE} WHERE +p.status = 'published' AND +p.published_at <= ? AND p.id > ? ORDER BY p.id LIMIT 500", (NOW, last)).fetchall()
        if not b: break
        last = b[-1][0]
def op_salva(c, n, rnd):
    i = rnd.randint(1, n)
    c.execute("UPDATE posts SET title = ?, description = ?, updated_at = ? WHERE id = ?", (f"Titolo {rnd.random()}", "sommario", NOW, i))
    c.execute("INSERT INTO revisions(post_id, user_id, title, description, body, created_at) VALUES (?, 1, ?, 'sommario', ?, ?)", (i, "titolo", "<p>testo</p>" * 300, NOW))
    c.execute("DELETE FROM revisions WHERE post_id = ? AND id NOT IN (SELECT id FROM revisions WHERE post_id = ? ORDER BY id DESC LIMIT 50)", (i, i))
    c.commit()

READS = [("Articolo singolo", op_articolo), ("Home (ultimi 20)", op_home), ("Categoria (20)", op_categoria), ("Correlati (3)", op_correlati),
         ("Elenco del pannello (300)", op_pannello), ("Ricerca parole chiave (tutti i testi)", op_link)]
# Carico misto della curva: le letture frequenti. Le operazioni pesanti e rare (elenco del pannello, ricerca nei testi,
# rigenerazione) si misurano a parte, con una stima della loro crescita: altrimenti una sola di esse domina tutta la curva.
MIX_READS = [op_articolo, op_articolo, op_home, op_categoria, op_correlati, op_correlati]
HEAVY = ["Elenco del pannello (300)", "Ricerca parole chiave (tutti i testi)", "Lettura completa per la rigenerazione"]


def pct(values, p):
    if not values: return 0.0
    s = sorted(values); k = min(len(s) - 1, int(round(p / 100 * (len(s) - 1))))
    return s[k]


def ms(x): return f"{x * 1000:.2f} ms" if x < 1 else f"{x:.2f} s"


# ---------------------------------------------------------------- 1) latenza di ogni operazione
def latency(path, n, budget):
    c = connect(path); rnd = random.Random(2); out = {}
    for name, fn in READS + [("Salvataggio con cronologia", op_salva)]:
        lat, t_end = [], time.perf_counter() + budget
        while time.perf_counter() < t_end or len(lat) < 5:
            t = time.perf_counter(); fn(c, n, rnd); lat.append(time.perf_counter() - t)
            if len(lat) >= 20000: break
        out[name] = {"p50": pct(lat, 50), "p95": pct(lat, 95), "p99": pct(lat, 99), "n": len(lat)}
    t = time.perf_counter(); op_rigenerazione(c, n, rnd); out["Lettura completa per la rigenerazione"] = {"p50": time.perf_counter() - t, "p95": 0, "p99": 0, "n": 1}
    c.close(); return out


# ---------------------------------------------------------------- 2) curva di carico
def worker(path, n, secs, write_share, seed, q):
    c = connect(path); rnd = random.Random(seed); lat, errors, count = [], 0, 0
    t_end = time.perf_counter() + secs
    while time.perf_counter() < t_end:
        fn = op_salva if rnd.random() < write_share else rnd.choice(MIX_READS)
        t = time.perf_counter()
        try:
            fn(c, n, rnd); count += 1
        except sqlite3.OperationalError:
            errors += 1
            try: c.rollback()
            except Exception: pass
        lat.append(time.perf_counter() - t)
    q.put((count, errors, lat[::max(1, len(lat) // 5000)]))


def level_separate(path, n, users, secs, write_share):
    """Ogni utente ha la sua connessione (processi separati): la capacità di SQLite in sé."""
    q = mp.Queue(); ps = [mp.Process(target=worker, args=(path, n, secs, write_share, 100 + k, q)) for k in range(users)]
    [p.start() for p in ps]; res = [q.get() for _ in ps]; [p.join() for p in ps]
    count = sum(r[0] for r in res); errors = sum(r[1] for r in res); lat = [x for r in res for x in r[2]]
    return count / secs, errors, lat


def level_single(path, n, users, secs, write_share):
    """Una sola connessione condivisa con un lucchetto: come fa il pannello di Presstatic."""
    c = connect(path); lock = threading.Lock(); stats = {"count": 0, "errors": 0}; lat = []; t_end = time.perf_counter() + secs
    def run(seed):
        rnd = random.Random(seed)
        while time.perf_counter() < t_end:
            fn = op_salva if rnd.random() < write_share else rnd.choice(MIX_READS)
            t = time.perf_counter()
            with lock:
                try: fn(c, n, rnd); stats["count"] += 1
                except sqlite3.OperationalError: stats["errors"] += 1
            lat.append(time.perf_counter() - t)
    th = [threading.Thread(target=run, args=(200 + k,)) for k in range(users)]
    [t.start() for t in th]; [t.join() for t in th]; c.close()
    return stats["count"] / secs, stats["errors"], lat


class Sampler:
    """Processore, attesa del disco e memoria durante un test (da /proc, solo Linux)."""
    def __init__(self): self.cpu, self.iow, self.stop = [], [], False
    def _read(self):
        v = [int(x) for x in open("/proc/stat").readline().split()[1:8]]; return sum(v), v[3], v[4]
    def run(self):
        a = self._read()
        while not self.stop:
            time.sleep(0.5); b = self._read(); tot = b[0] - a[0] or 1
            self.cpu.append(100 * (1 - (b[1] - a[1] + b[2] - a[2]) / tot)); self.iow.append(100 * (b[2] - a[2]) / tot); a = b
    def __enter__(self): self.t = threading.Thread(target=self.run); self.t.start(); return self
    def __exit__(self, *e): self.stop = True; self.t.join()


def curve(path, n, levels, secs, write_share, mode):
    rows = []; fn = level_separate if mode == "separate" else level_single
    for users in levels:
        with Sampler() as s:
            ops, err, lat = fn(path, n, users, secs, write_share)
        rows.append({"users": users, "ops": ops, "p50": pct(lat, 50), "p99": pct(lat, 99), "errors": err,
                     "cpu": statistics.mean(s.cpu) if s.cpu else 0, "iowait": statistics.mean(s.iow) if s.iow else 0})
        print(f"    {users:>3} utenti: {ops:>9,.0f} operazioni/s   p50 {ms(rows[-1]['p50']):>10}   p99 {ms(rows[-1]['p99']):>10}   errori {err}   CPU {rows[-1]['cpu']:.0f}%", flush=True)
    return rows


def knee(rows):
    """Primo livello in cui la capacità smette di crescere (meno del 10% in più) mentre la latenza al 99° percentile raddoppia."""
    for a, b in zip(rows, rows[1:]):
        if b["ops"] < a["ops"] * 1.10 and b["p99"] > rows[0]["p99"] * 2:
            return a
    return None


# ---------------------------------------------------------------- 3) scritture continue e file WAL
def sustained(path, n, secs):
    stop = time.perf_counter() + secs; wal = path + "-wal"; sizes, lat = [], []
    q = mp.Queue(); readers = [mp.Process(target=worker, args=(path, n, secs, 0.0, 300 + k, q)) for k in range(2)]
    [r.start() for r in readers]
    c = connect(path); rnd = random.Random(9)
    while time.perf_counter() < stop:
        t = time.perf_counter(); op_salva(c, n, rnd); lat.append(time.perf_counter() - t)
        if len(lat) % 200 == 0: sizes.append(os.path.getsize(wal) if os.path.exists(wal) else 0)
    [q.get() for _ in readers]; [r.join() for r in readers]
    c.execute("PRAGMA wal_checkpoint(TRUNCATE)"); c.close()  # riporta il file WAL a zero dopo la prova
    med = pct(lat, 50)
    return {"saves": len(lat), "per_s": len(lat) / secs, "p50": med, "p99": pct(lat, 99), "p999": pct(lat, 99.9), "max": max(lat),
            "spikes": sum(1 for x in lat if x > med * 10), "wal_max": max(sizes or [0])}


# ---------------------------------------------------------------- 4) disco
def disk(folder, rounds=300):
    f = os.path.join(folder, "fsync.tmp"); lat = []
    with open(f, "wb") as h:
        for _ in range(rounds):
            h.write(os.urandom(4096)); h.flush(); t = time.perf_counter(); os.fsync(h.fileno()); lat.append(time.perf_counter() - t)
    os.remove(f); return {"p50": pct(lat, 50), "p99": pct(lat, 99), "per_s": 1 / statistics.mean(lat)}


# ---------------------------------------------------------------- 5) arresto brusco durante i salvataggi
CRASH_CHILD = r"""
import sqlite3, sys
c = sqlite3.connect(sys.argv[1], timeout=5); i = 0
while True:
    i += 1
    c.execute("INSERT INTO settings(key, value) VALUES (?, 'x') ON CONFLICT(key) DO UPDATE SET value = excluded.value", (f"crash-{i}",))
    c.commit()
    open(sys.argv[2], "w").write(str(i))
"""
def crash(path, rounds):
    ok = 0; notes = []
    for r in range(rounds):
        mark = path + ".last"; p = subprocess.Popen([sys.executable, "-c", CRASH_CHILD, path, mark])
        time.sleep(random.uniform(0.3, 1.2)); p.send_signal(signal.SIGKILL); p.wait()
        last = int(open(mark).read() or 0) if os.path.exists(mark) else 0
        c = sqlite3.connect(path); integ = c.execute("PRAGMA integrity_check").fetchone()[0]
        found = c.execute("SELECT 1 FROM settings WHERE key = ?", (f"crash-{last}",)).fetchone() is not None if last else True
        c.execute("DELETE FROM settings WHERE key LIKE 'crash-%'"); c.commit(); c.close()
        good = integ == "ok" and found; ok += good
        if not good: notes.append(f"giro {r + 1}: integrità {integ}, ultimo salvataggio confermato {'presente' if found else 'MANCANTE'}")
    return ok, rounds, notes


# ---------------------------------------------------------------- 6) backup a caldo durante i salvataggi
def backup(path, n, folder, rate=50):
    """Backup con VACUUM INTO (copia coerente in un colpo solo) mentre una redazione molto attiva salva 50 volte al secondo."""
    c = connect(path); c.execute("PRAGMA wal_checkpoint(TRUNCATE)"); c.close()
    stop = threading.Event(); lat_before, lat_during, phase = [], [], {"during": False}
    def writer():
        w = connect(path); rnd = random.Random(5)
        while not stop.is_set():
            t = time.perf_counter(); op_salva(w, n, rnd); (lat_during if phase["during"] else lat_before).append(time.perf_counter() - t)
            time.sleep(max(0, 1 / rate - (time.perf_counter() - t)))
        w.close()
    th = threading.Thread(target=writer); th.start(); time.sleep(2)
    dest = os.path.join(folder, "backup.db"); src = connect(path)
    phase["during"] = True; t = time.perf_counter(); src.execute("VACUUM INTO ?", (dest,)); took = time.perf_counter() - t
    time.sleep(0.3); phase["during"] = False; time.sleep(0.5); stop.set(); th.join(); src.close()
    ok = sqlite3.connect(dest).execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    size = os.path.getsize(dest); os.remove(dest)
    return {"took": took, "mb": size / 1048576, "ok": ok, "before_p50": pct(lat_before, 50), "during_p50": pct(lat_during, 50), "during_p99": pct(lat_during, 99)}


# ---------------------------------------------------------------- rapporto HTML con grafici (SVG, nessuna libreria)
def svg_chart(title, series, xs, ylabel, fmt):
    W, H, L, B = 640, 300, 64, 40
    ymax = max([max(s["ys"]) for s in series] + [1e-9]) * 1.1
    def X(i): return L + i * (W - L - 20) / max(1, len(xs) - 1)
    def Y(v): return H - B - v / ymax * (H - B - 20)
    out = [f'<svg viewBox="0 0 {W} {H}" role="img" aria-label="{title}"><text x="{L}" y="14" font-weight="700">{title}</text>']
    for k in range(5):
        v = ymax * k / 4; out.append(f'<line x1="{L}" x2="{W-20}" y1="{Y(v):.1f}" y2="{Y(v):.1f}" stroke="#e3e6eb"/><text x="{L-6}" y="{Y(v)+4:.1f}" text-anchor="end" font-size="11">{fmt(v)}</text>')
    for i, x in enumerate(xs): out.append(f'<text x="{X(i):.1f}" y="{H-B+18}" text-anchor="middle" font-size="11">{x}</text>')
    out.append(f'<text x="{(W+L)/2}" y="{H-4}" text-anchor="middle" font-size="11">{ylabel}</text>')
    for s in series:
        pts = " ".join(f"{X(i):.1f},{Y(v):.1f}" for i, v in enumerate(s["ys"]))
        out.append(f'<polyline fill="none" stroke="{s["color"]}" stroke-width="2.5" points="{pts}"/>')
        out += [f'<circle cx="{X(i):.1f}" cy="{Y(v):.1f}" r="3.5" fill="{s["color"]}"/>' for i, v in enumerate(s["ys"])]
    legend = "".join(f'<tspan fill="{s["color"]}" font-weight="700">■ {s["name"]}</tspan>   ' for s in series)
    out.append(f'<text x="{L}" y="32" font-size="12">{legend}</text></svg>')
    return "".join(out)


def report(res, file):
    h = ['<!doctype html><html lang="it"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">',
         '<title>Benchmark SQLite di Presstatic</title><style>body{font:16px/1.5 system-ui,sans-serif;max-width:960px;margin:2rem auto;padding:0 1rem;color:#15171c}',
         'table{border-collapse:collapse;width:100%;margin:1rem 0 2rem;font-size:.9rem}td,th{border-bottom:1px solid #e3e6eb;padding:.4rem;text-align:right}td:first-child,th:first-child{text-align:left}',
         'svg{width:100%;height:auto;margin:1rem 0;font-family:system-ui}.ok{color:#0a7a3a}.ko{color:#b3122e}</style>',
         f'<h1>Benchmark SQLite di Presstatic</h1><p>Server: {os.cpu_count()} core · SQLite {sqlite3.sqlite_version} · {time.strftime("%d/%m/%Y %H:%M")}</p>']
    for size, lat in res["latency"].items():
        h.append(f"<h2>Latenza con {size:,} articoli</h2><table><tr><th>Operazione</th><th>mediana</th><th>95%</th><th>99%</th></tr>")
        h += [f"<tr><td>{k}</td><td>{ms(v['p50'])}</td><td>{ms(v['p95']) if v['p95'] else '—'}</td><td>{ms(v['p99']) if v['p99'] else '—'}</td></tr>" for k, v in lat.items()]
        h.append("</table>")
    if res.get("growth"):
        h.append("<h2>Stima per archivi più grandi</h2><table><tr><th>Operazione</th><th>100.000 articoli</th><th>1.000.000</th></tr>")
        h += [f"<tr><td>{k}</td><td>~{ms(v[100000])}</td><td>~{ms(v[1000000])}</td></tr>" for k, v in res["growth"].items()]
        h.append("</table>")
    for mode, label in (("single", "una connessione, come il pannello"), ("separate", "connessioni separate, SQLite da solo")):
        rows = res["curve"][mode]; xs = [r["users"] for r in rows]
        h.append(f"<h2>Curva di carico: {label}</h2>")
        h.append(svg_chart("Operazioni al secondo", [{"name": "operazioni/s", "ys": [r["ops"] for r in rows], "color": "#1f4fd8"}], xs, "utenti contemporanei", lambda v: f"{v:,.0f}"))
        h.append(svg_chart("Latenza", [{"name": "99%", "ys": [r["p99"] * 1000 for r in rows], "color": "#b3122e"}, {"name": "mediana", "ys": [r["p50"] * 1000 for r in rows], "color": "#0f766e"}], xs, "utenti contemporanei", lambda v: f"{v:.1f} ms"))
        k = knee(rows)
        h.append(f"<p><strong>{'Saturazione da ' + str(k['users']) + ' utenti contemporanei: circa ' + format(int(k['ops']), ',') + ' operazioni al secondo.' if k else 'Nessuna saturazione nei livelli provati.'}</strong></p>")
    s, d, cr, b = res["sustained"], res["disk"], res["crash"], res["backup"]
    h.append(f"<h2>Scritture continue</h2><p>{s['saves']:,} salvataggi ({s['per_s']:,.0f} al secondo) con 2 lettori in parallelo: mediana {ms(s['p50'])}, 99% {ms(s['p99'])}, 99,9% {ms(s['p999'])}, massimo {ms(s['max'])}; picchi oltre 10 volte la mediana: {s['spikes']}; file WAL al massimo {s['wal_max']/1048576:.1f} MB.</p>")
    h.append(f"<h2>Disco</h2><p>Conferma di scrittura (fsync): mediana {ms(d['p50'])}, 99% {ms(d['p99'])}, circa {d['per_s']:,.0f} al secondo.</p>")
    h.append(f"<h2>Arresto brusco</h2><p class=\"{'ok' if cr[0] == cr[1] else 'ko'}\">{cr[0]} prove su {cr[1]}: database integro e nessun salvataggio confermato perso.</p>")
    h.append(f"<h2>Backup a caldo</h2><p>{b['mb']:.0f} MB copiati con VACUUM INTO in {ms(b['took'])} mentre la redazione salvava 50 volte al secondo; copia {'integra' if b['ok'] else 'danneggiata'}; salvataggi: mediana {ms(b['before_p50'])} prima e {ms(b['during_p50'])} durante (99%: {ms(b['during_p99'])}).</p>")
    open(file, "w").write("".join(h))


def main():
    ap = argparse.ArgumentParser(description="Benchmark di SQLite con le query reali di Presstatic")
    ap.add_argument("--dimensioni", default="10000,50000", help="numeri di articoli da provare, separati da virgole")
    ap.add_argument("--livelli", default="1,2,4,8,16,32,64", help="utenti contemporanei nella curva di carico")
    ap.add_argument("--passo", type=float, default=5, help="secondi per ogni livello della curva")
    ap.add_argument("--scritture", type=float, default=0.05, help="quota di salvataggi nel carico misto (0.05 = 5%%)")
    ap.add_argument("--reale", help="percorso di un presstatic.db vero: ne viene usata una copia, solo per le latenze")
    ap.add_argument("--rapido", action="store_true", help="prova veloce: archivi più piccoli e livelli più brevi")
    ap.add_argument("--rapporto", default="benchmark-sqlite.html")
    a = ap.parse_args()
    if a.rapido: a.dimensioni, a.passo = "5000,20000", 2.5
    sizes = [int(x) for x in a.dimensioni.split(",")]; levels = [int(x) for x in a.livelli.split(",")]
    folder = tempfile.mkdtemp(prefix="presstatic-bench-"); res = {"latency": {}, "curve": {}, "growth": {}}
    need = max(sizes) * 5000 * 3 + 800 * 1048576  # database più grande, sua crescita durante le scritture, backup
    free = shutil.disk_usage(folder).free
    if free < need:
        print(f"Spazio libero insufficiente in {folder}: servono circa {need/1073741824:.1f} GB, ci sono {free/1073741824:.1f} GB. Usa archivi più piccoli con --dimensioni."); return
    print(f"Benchmark SQLite {sqlite3.sqlite_version} su {os.cpu_count()} core, cartella di lavoro {folder}\n")
    try:
        for n in sizes:
            path = os.path.join(folder, f"prova-{n}.db"); t = time.perf_counter(); build(path, n)
            print(f"1) Latenza con {n:,} articoli (database di {os.path.getsize(path)/1048576:.0f} MB, creato in {time.perf_counter()-t:.1f} s)")
            lat = latency(path, n, 1.0 if a.rapido else 2.0); res["latency"][n] = lat
            for k, v in lat.items(): print(f"    {k:<42} mediana {ms(v['p50']):>10}" + (f"   95% {ms(v['p95']):>10}   99% {ms(v['p99']):>10}" if v["p95"] else ""))
        if len(sizes) > 1:
            n1, n2 = sizes[-2], sizes[-1]
            print("\n1c) Stima per archivi più grandi (le operazioni che scorrono tutto crescono in proporzione)")
            for k in HEAVY:
                per_row = (res["latency"][n2][k]["p50"] - res["latency"][n1][k]["p50"]) / (n2 - n1)
                est = {m: max(0.0, res["latency"][n2][k]["p50"] + per_row * (m - n2)) for m in (100000, 1000000)}
                res["growth"][k] = est
                print(f"    {k:<42} 100.000 articoli: ~{ms(est[100000]):>9}   1.000.000: ~{ms(est[1000000]):>9}")
        if a.reale:
            copy = os.path.join(folder, "copia-reale.db"); c = sqlite3.connect(a.reale); d = sqlite3.connect(copy); c.backup(d); c.close(); d.close()
            n = sqlite3.connect(copy).execute("SELECT MAX(id) FROM posts").fetchone()[0] or 1
            print(f"\n1b) Latenza sulla copia del database reale ({n:,} articoli)")
            lat = latency(copy, n, 2.0); res["latency"][f"reale-{n}"] = lat
            for k, v in lat.items(): print(f"    {k:<42} mediana {ms(v['p50']):>10}")
        n = sizes[-1]; path = os.path.join(folder, f"prova-{n}.db")
        print(f"\n2) Curva di carico con {n:,} articoli, {int(a.scritture*100)}% salvataggi e {100-int(a.scritture*100)}% letture, {a.passo:g} s per livello")
        for mode, label in (("single", "una connessione con lucchetto, come il pannello di Presstatic"), ("separate", "una connessione per utente: SQLite da solo")):
            print(f"  {label}:"); res["curve"][mode] = curve(path, n, levels, a.passo, a.scritture, mode)
            k = knee(res["curve"][mode])
            print(f"    -> {'saturazione da ' + str(k['users']) + ' utenti: circa ' + format(int(k['ops']), ',') + ' operazioni al secondo' if k else 'nessuna saturazione nei livelli provati'}")
        print(f"\n3) Scritture continue ({int(a.passo*3)} s, con 2 lettori in parallelo)")
        s = sustained(path, n, a.passo * 3); res["sustained"] = s
        print(f"    {s['saves']:,} salvataggi ({s['per_s']:,.0f}/s): mediana {ms(s['p50'])}, 99% {ms(s['p99'])}, 99,9% {ms(s['p999'])}, massimo {ms(s['max'])}; picchi {s['spikes']}; WAL max {s['wal_max']/1048576:.1f} MB")
        print("\n4) Disco: tempo di conferma di una scrittura (fsync)")
        d = disk(folder); res["disk"] = d; print(f"    mediana {ms(d['p50'])}, 99% {ms(d['p99'])}, circa {d['per_s']:,.0f} conferme al secondo")
        print("\n5) Arresto brusco durante i salvataggi")
        cr = crash(path, 5); res["crash"] = cr; print(f"    {cr[0]} prove su {cr[1]}: database integro e nessun salvataggio confermato perso" + ("".join("\n    " + x for x in cr[2])))
        print("\n6) Backup a caldo (VACUUM INTO) mentre la redazione salva 50 volte al secondo")
        b = backup(path, n, folder); res["backup"] = b
        print(f"    {b['mb']:.0f} MB copiati in {ms(b['took'])}, copia {'integra' if b['ok'] else 'DANNEGGIATA'}; salvataggi (50 al secondo): mediana {ms(b['before_p50'])} prima, {ms(b['during_p50'])} durante (99%: {ms(b['during_p99'])})")
        report(res, a.rapporto); print(f"\nRapporto con i grafici: {os.path.abspath(a.rapporto)}")
        json.dump(res, open(a.rapporto.replace(".html", ".json"), "w"), default=str)
    finally:
        shutil.rmtree(folder, ignore_errors=True)


if __name__ == "__main__":
    main()
