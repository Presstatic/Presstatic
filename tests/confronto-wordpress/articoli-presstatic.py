# Crea un sito Presstatic con 20.000 articoli realistici direttamente nel database.
import sqlite3, random, time, sys
random.seed(7); D = sys.argv[1]; N = int(sys.argv[2])
W = ("comune sindaco lavori strada ponte fiume scuola ospedale piazza mercato calcio squadra partita cultura mostra teatro "
     "ambiente parco raccolta rifiuti regione bilancio consiglio cittadini quartiere stazione treno autobus estate inverno").split()
def text(n): return " ".join(random.choice(W) for _ in range(n))
CATS = ["Cronaca", "Politica", "Economia", "Sport", "Cultura", "Ambiente"]
TAGS = [f"argomento {i}" for i in range(300)]
db = sqlite3.connect(f"{D}/presstatic.db")
db.executemany("INSERT OR IGNORE INTO users(id, name, email, pass, role, bio, photo, slug) VALUES (?,?,?,?,?,?,?,?)",
    [(i, f"Autore {i}", f"autore{i}@x.it", "x", "author", text(18), "", f"autore-{i}") for i in range(2, 12)])
now = int(time.time()); rows = []
for i in range(1, N + 1):
    paras = [f"<p>{text(80)}</p>" for _ in range(6)]
    paras.insert(2, f'<img src="/media/2026/09/foto-{i % 50}-1600x900.jpg" alt="foto">')
    paras.insert(4, f"<h2>{text(5)}</h2>")
    kw = f"parola chiave {i}" if i % 40 == 0 else ""
    rows.append((f"articolo-{i}", text(10).capitalize(), text(22), "".join(paras), random.choice(CATS), f"/media/2026/09/foto-{i % 50}-1600x900.jpg",
                 "NewsArticle", "{}", "published", now - (N - i) * 300, now - (N - i) * 300, random.randint(2, 11), "post", ", ".join(random.sample(TAGS, 3)), 0, kw))
db.executemany("INSERT INTO posts(slug, title, description, body, category, image, schema_type, schema_data, status, published_at, updated_at, author_id, kind, tags, featured, link_keywords) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", rows)
for k, v in [("site_name", "Cronaca Demo"), ("base_url", "http://127.0.0.1:8088"), ("theme", "moderno"), ("menu", "\n".join(CATS)), ("related", "on"), ("links_on", "on")]:
    db.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?, ?)", (k, v))
db.commit(); print("articoli:", db.execute("select count(*) from posts where kind='post'").fetchone()[0])
