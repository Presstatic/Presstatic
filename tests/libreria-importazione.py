# Libreria media (didascalie, crediti, gallerie) e importazione da WordPress.
# Un finto sito WordPress locale serve le immagini; il file di esportazione (WXR) è costruito qui.
# Uso: python3 tests/libreria-importazione.py http://127.0.0.1:8204 /percorso/del/sito
import subprocess, sys, re, time, json, sqlite3, os, io, threading, http.server, urllib.parse
from PIL import Image
B, DIR = sys.argv[1], sys.argv[2]
PUB = f"{DIR}/public"
fails = 0
def ok(cond, label):
    global fails; fails += 0 if cond else 1; print(("  OK  " if cond else "  NO  ") + label)
def curl(jar, *a): return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
def form(jar, path, fields):
    args = []
    for k, v in fields.items(): args += ["--form-string", f"{k}={v}"]
    return curl(jar, "-o", "/dev/null", "-w", "%{redirect_url}", *args, B + path)
def db(): return sqlite3.connect(f"{DIR}/presstatic.db")
def page(path):
    f = f"{PUB}/{path.strip('/')}/index.html"
    return open(f, encoding="utf-8").read() if os.path.isfile(f) else ""
def jpg(color, size=(1400, 900)):
    b = io.BytesIO(); Image.new("RGB", size, color).save(b, "JPEG", quality=85); return b.getvalue()

A = "/tmp/li-a.jar"; open(A, "w").close()
curl(A, "-o", "/dev/null", "--data-urlencode", "email=andrea@example.com", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")

print("== 1. Libreria media")
open("/tmp/li-1.jpg", "wb").write(jpg((200, 40, 40))); open("/tmp/li-2.jpg", "wb").write(jpg((40, 40, 200)))
r = json.loads(curl(A, "-F", "files=@/tmp/li-1.jpg;filename=piazza.jpg", "-F", "files=@/tmp/li-2.jpg;filename=stadio.jpg", B + "/admin/media/upload"))
ok(len(r["items"]) == 2 and not r["errors"], "due immagini caricate insieme dalla libreria")
m1, m2 = r["items"]
ok(m1["thumb"].endswith("-480.webp") and os.path.isfile(PUB + m1["thumb"]), "ognuna ha la sua miniatura WebP")
curl(A, "-o", "/dev/null", "--data-urlencode", "alt=La piazza del paese piena di gente", "--data-urlencode", "caption=La festa di ieri sera", "--data-urlencode", "credit=Mario Rossi", B + f"/admin/media/{m1['id']}")
lst = json.loads(curl(A, B + "/admin/media/list?q=festa"))
ok(lst["total"] == 1 and lst["items"][0]["credit"] == "Mario Rossi", "testo alternativo, didascalia e credito salvati, e la ricerca li trova")

print("== 2. Foto in evidenza con didascalia, foto nel testo, galleria")
body = (f'<p>Primo paragrafo.</p><figure class="ps-figure"><img src="{m2["url"]}" alt="Lo stadio"><figcaption>La curva sud <span class="credit">Foto: Ufficio stampa</span></figcaption></figure>'
        f'<div class="ps-gallery cols-2"><figure class="ps-figure"><img src="{m1["url"]}" alt="Piazza"></figure><figure class="ps-figure"><img src="{m2["url"]}" alt="Stadio"></figure></div><p>Fine.</p>')
form(A, "/admin/edit/0", {"title": "Festa in piazza", "slug": "festa-in-piazza", "body": body, "category": "Cronaca", "status": "published", "image": m1["url"]})
h = page("festa-in-piazza")
ok('class="hero-cap">La festa di ieri sera <span class="credit">Foto: Mario Rossi</span>' in h, "sotto la foto in evidenza compaiono didascalia e credito")
ok('alt="La piazza del paese piena di gente"' in h, "la foto in evidenza usa il testo alternativo della libreria")
ok('<figcaption>La curva sud <span class="credit">Foto: Ufficio stampa</span></figcaption>' in h and "srcset=" in h.split('<figure class="ps-figure"><img', 1)[-1][:700], "la foto nel testo ha didascalia, credito e le misure WebP")
ok('class="ps-gallery cols-2"' in h and h.count('class="ps-figure"') >= 3, "la galleria è nella pagina")
curl(A, "-o", "/dev/null", "--data-urlencode", "alt=La piazza del paese piena di gente", "--data-urlencode", "caption=La festa di sabato sera", "--data-urlencode", "credit=Mario Rossi", B + f"/admin/media/{m1['id']}")
for _ in range(30):
    if "La festa di sabato sera" in page("festa-in-piazza"): break
    time.sleep(0.2)
ok("La festa di sabato sera" in page("festa-in-piazza"), "cambiata la didascalia nella libreria, la pagina dell'articolo si aggiorna da sola")
used = json.loads(curl(A, B + f"/admin/media/{m1['id']}/uso"))["used"]
ok(any(u["title"] == "Festa in piazza" for u in used), "la libreria sa in quali articoli è usata una foto")
r = json.loads(curl(A, "-X", "POST", B + f"/admin/media/{m1['id']}/delete"))
ok("error" in r and os.path.isfile(PUB + m1["url"]), "una foto usata non si può eliminare")
open("/tmp/li-3.jpg", "wb").write(jpg((40, 160, 40)))
m3 = json.loads(curl(A, "-F", "files=@/tmp/li-3.jpg;filename=prato.jpg", B + "/admin/media/upload"))["items"][0]
r = json.loads(curl(A, "-X", "POST", B + f"/admin/media/{m3['id']}/delete"))
base = m3["url"].rsplit("-", 1)[0]
left = [f for f in os.listdir(PUB + os.path.dirname(m3["url"])) if f.startswith(os.path.basename(base) + "-")]
ok(r.get("ok") and not left, "una foto non usata si elimina con tutte le sue misure")

print("== 3. Importazione da WordPress")
WP = "http://127.0.0.1:8305"
files = {"/wp-content/uploads/2024/05/ponte.jpg": jpg((90, 90, 90)), "/wp-content/uploads/2024/05/ponte-1024x683.jpg": jpg((90, 90, 91), (1024, 683)),
         "/wp-content/uploads/2024/05/galleria-a.jpg": jpg((10, 120, 200)), "/wp-content/uploads/2024/05/galleria-b.jpg": jpg((200, 120, 10))}
class Srv(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = files.get(self.path.split("?")[0]); self.send_response(200 if b else 404); self.send_header("Content-Type", "image/jpeg"); self.end_headers(); self.wfile.write(b or b"")
    def log_message(self, *a): pass
threading.Thread(target=http.server.HTTPServer(("127.0.0.1", 8305), Srv).serve_forever, daemon=True).start()
def item(pid, typ, status, title, slug, link, content, date="2024-05-12 08:30:00", creator="mario", cats="", meta="", excerpt="", att=""):
    metas = "".join(f"<wp:postmeta><wp:meta_key><![CDATA[{k}]]></wp:meta_key><wp:meta_value><![CDATA[{v}]]></wp:meta_value></wp:postmeta>" for k, v in meta)
    return f"""<item><title>{title}</title><link>{link}</link><dc:creator><![CDATA[{creator}]]></dc:creator>
<content:encoded><![CDATA[{content}]]></content:encoded><excerpt:encoded><![CDATA[{excerpt}]]></excerpt:encoded>
<wp:post_id>{pid}</wp:post_id><wp:post_date_gmt><![CDATA[{date}]]></wp:post_date_gmt><wp:post_name><![CDATA[{slug}]]></wp:post_name>
<wp:status><![CDATA[{status}]]></wp:status><wp:post_type><![CDATA[{typ}]]></wp:post_type>{att}{cats}{metas}</item>"""
gutenberg = """<!-- wp:paragraph --><p>Il ponte sul canale è stato riaperto stamattina.</p><!-- /wp:paragraph -->
<!-- wp:heading --><h2 class="wp-block-heading">Le novità</h2><!-- /wp:heading -->
<!-- wp:image {"id":100} --><figure class="wp-block-image size-large"><img decoding="async" width="1024" height="683" src="http://127.0.0.1:8305/wp-content/uploads/2024/05/ponte-1024x683.jpg" alt="Il ponte visto dal fiume" class="wp-image-100" srcset="http://127.0.0.1:8305/wp-content/uploads/2024/05/ponte-1024x683.jpg 1024w" sizes="(max-width: 1024px) 100vw" /><figcaption class="wp-element-caption">Il ponte riaperto</figcaption></figure><!-- /wp:image -->
<!-- wp:embed {"url":"https://www.youtube.com/watch?v=dQw4w9WgXcQ"} --><figure class="wp-block-embed is-type-video is-provider-youtube"><div class="wp-block-embed__wrapper">
https://www.youtube.com/watch?v=dQw4w9WgXcQ
</div></figure><!-- /wp:embed -->"""
classic = """Primo paragrafo dell'editor classico.
Seconda riga dello stesso paragrafo.

[caption id="attachment_100" align="aligncenter" width="800"]<img class="size-large wp-image-100" src="/wp-content/uploads/2024/05/ponte.jpg" alt="Il ponte" width="800" height="533" /> Il ponte di sera[/caption]

[gallery ids="100,101"]

[contact-form-7 id="12" title="Contatti"]

Ultimo paragrafo."""
cat = lambda d, n, t: f'<category domain="{d}" nicename="{n}"><![CDATA[{t}]]></category>'
wxr = f"""<?xml version="1.0" encoding="UTF-8" ?>
<rss version="2.0" xmlns:excerpt="http://wordpress.org/export/1.2/excerpt/" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:wp="http://wordpress.org/export/1.2/">
<channel><title>Vecchio sito</title><link>{WP}</link>
<wp:author><wp:author_id>2</wp:author_id><wp:author_login><![CDATA[mario]]></wp:author_login><wp:author_email><![CDATA[mario@example.com]]></wp:author_email><wp:author_display_name><![CDATA[Mario Bianchi]]></wp:author_display_name></wp:author>
<wp:author><wp:author_id>1</wp:author_id><wp:author_login><![CDATA[andrea]]></wp:author_login><wp:author_email><![CDATA[andrea@example.com]]></wp:author_email><wp:author_display_name><![CDATA[Andrea]]></wp:author_display_name></wp:author>
<wp:category><wp:term_id>7</wp:term_id><wp:category_nicename><![CDATA[cronaca]]></wp:category_nicename><wp:cat_name><![CDATA[Cronaca]]></wp:cat_name></wp:category>
<wp:category><wp:term_id>8</wp:term_id><wp:category_nicename><![CDATA[sport]]></wp:category_nicename><wp:cat_name><![CDATA[Sport]]></wp:cat_name></wp:category>
{item(100, "attachment", "inherit", "ponte", "ponte", WP + "/ponte/", "", att=f"<wp:attachment_url>{WP}/wp-content/uploads/2024/05/ponte.jpg</wp:attachment_url>", meta=[("_wp_attachment_image_alt", "Il ponte sul canale")], excerpt="Il ponte al tramonto")}
{item(101, "attachment", "inherit", "galleria b", "galleria-b", WP + "/galleria-b/", "", att=f"<wp:attachment_url>{WP}/wp-content/uploads/2024/05/galleria-b.jpg</wp:attachment_url>")}
{item(10, "post", "publish", "Il ponte &#8220;riaperto&#8221; &amp; festeggiato", "ponte-riaperto", WP + "/2024/05/12/ponte-riaperto/", gutenberg, cats=cat("category", "cronaca", "Cronaca") + cat("category", "sport", "Sport") + cat("post_tag", "viabilita", "Viabilità"), meta=[("_thumbnail_id", "100"), ("_yoast_wpseo_primary_category", "8"), ("_yoast_wpseo_metadesc", "Riaperto il ponte sul canale dopo i lavori.")])}
{item(11, "post", "publish", "Città vecchia", "citt%c3%a0-vecchia", WP + "/citt%c3%a0-vecchia/", classic, creator="andrea", cats=cat("category", "senza-categoria", "Senza categoria"))}
{item(12, "post", "future", "Articolo programmato", "programmato", WP + "/2030/01/01/programmato/", "<p>Uscirà.</p>", date="2030-01-01 09:00:00")}
{item(13, "post", "draft", "Una bozza", "", WP + "/?p=13", "<p>Da finire.</p>", date="0000-00-00 00:00:00")}
{item(14, "post", "trash", "Nel cestino", "cestino", WP + "/cestino/", "<p>Via.</p>")}
{item(15, "page", "publish", "La redazione", "la-redazione", WP + "/la-redazione/", "<p>Chi siamo.</p>")}
</channel></rss>"""
open("/tmp/li-wp.xml", "w").write(wxr)
open("/tmp/li-bad.xml", "w").write("<html><body>non è un export</body></html>")
curl(A, "-o", "/dev/null", "-F", "wxr=@/tmp/li-bad.xml", "-F", "images=on", B + "/admin/importa")
for _ in range(50):
    s = json.loads(curl(A, B + "/admin/importa/stato"))
    if not s["running"]: break
    time.sleep(0.2)
ok(s["errors"] and "WordPress" in s["errors"][0], "un file che non è un'esportazione di WordPress viene rifiutato con una spiegazione")
curl(A, "-o", "/dev/null", "-F", "wxr=@/tmp/li-wp.xml", "-F", "images=on", "-F", "authors=on", B + "/admin/importa")
for _ in range(300):
    s = json.loads(curl(A, B + "/admin/importa/stato"))
    if not s["running"]: break
    time.sleep(0.2)
print(f"   {s['phase']}: {s['posts']} articoli, {s['pages']} pagine, {s['images']} immagini, {s['users']} autori nuovi, {s['redirects']} reindirizzamenti; avvisi: {s['errors']}")
ok(s["finished"] and s["posts"] == 4 and s["pages"] == 1, "4 articoli (pubblicati, programmato, bozza) e 1 pagina; il cestino resta fuori")
ok(s["users"] == 1 and s["images"] == 2, "creato un autore nuovo (l'altro aveva già l'account); scaricate 2 immagini: la versione ridotta nel testo diventa l'originale, senza doppioni")
with db() as c:
    p = dict(zip(["id", "title", "slug", "body", "category", "tags", "image", "description", "author"], c.execute("SELECT p.id, p.title, p.slug, p.body, p.category, p.tags, p.image, p.description, u.name FROM posts p JOIN users u ON u.id = p.author_id WHERE title LIKE 'Il ponte%'").fetchone()))
    c2 = dict(zip(["slug", "body", "category", "author"], c.execute("SELECT p.slug, p.body, p.category, u.name FROM posts p JOIN users u ON u.id = p.author_id WHERE title = 'Città vecchia'").fetchone()))
    stat = dict(c.execute("SELECT title, status FROM posts WHERE title IN ('Articolo programmato', 'Una bozza')").fetchall())
    alt = c.execute("SELECT alt, caption FROM media WHERE url = ?", (p["image"],)).fetchone()
ok(p["title"] == "Il ponte “riaperto” & festeggiato", "i titoli con entità HTML diventano testo normale")
ok(p["category"] == "Sport" and p["tags"] == "Viabilità", "categoria principale di Yoast e tag")
with db() as c: ok(c.execute("SELECT categories FROM posts WHERE id = ?", (p["id"],)).fetchone()[0] == "Cronaca", "l'altra categoria di WordPress diventa categoria aggiuntiva")
ok(p["description"] == "Riaperto il ponte sul canale dopo i lavori.", "la descrizione SEO di Yoast diventa il sommario")
ok(p["author"] == "Mario Bianchi", "l'articolo è firmato dal suo autore")
ok(p["image"].startswith("/media/") and alt == ("Il ponte sul canale", "Il ponte al tramonto"), "foto in evidenza scaricata, con testo alternativo e didascalia di WordPress")
b = p["body"]
ok("wp:" not in b and "srcset" not in b and "wp-image" not in b, "niente commenti dei blocchi né attributi di WordPress nel testo")
ok('<figure class="ps-figure"><img src="/media/' in b and "<figcaption>Il ponte riaperto</figcaption>" in b, "le foto dei blocchi diventano foto con didascalia, scaricate")
ok("youtube-nocookie.com/embed/dQw4w9WgXcQ" in b, "il video di YouTube diventa un video incorporato senza cookie")
cb = c2["body"]
ok("<p>Primo paragrafo dell'editor classico.<br>Seconda riga dello stesso paragrafo.</p>" in cb, "il testo dell'editor classico viene diviso in paragrafi")
ok("<figcaption>Il ponte di sera</figcaption>" in cb and 'class="ps-gallery cols-3"' in cb, "[caption] e [gallery] diventano foto con didascalia e galleria")
ok("contact-form" not in cb and "Ultimo paragrafo." in cb, "gli shortcode sconosciuti vengono tolti, il testo resta")
ok(c2["category"] == "" and c2["author"] == "Andrea Admin", "«Senza categoria» resta vuota; l'autore con lo stesso indirizzo email è l'account esistente")
ok(stat == {"Articolo programmato": "published", "Una bozza": "draft"}, "programmato e bozza mantengono il loro stato")
ok("Il ponte" in page(p["slug"]), "la pagina del nuovo articolo è online")
red = page("2024/05/12/ponte-riaperto")
ok(f'/{p["slug"]}/' in (re.search(r'url=([^"]+)"', red) or [None, ""])[1], "il vecchio indirizzo con la data porta al nuovo")
ok("citta-vecchia" in page("città-vecchia") and c2["slug"] == "citta-vecchia", "anche il vecchio indirizzo con la lettera accentata porta al nuovo")
ok(page("la-redazione") and not page("la-redazione").count("http-equiv"), "la pagina con lo stesso indirizzo resta allo stesso indirizzo")
curl(A, "-o", "/dev/null", "-F", "wxr=@/tmp/li-wp.xml", "-F", "images=on", "-F", "authors=on", B + "/admin/importa")
for _ in range(100):
    s = json.loads(curl(A, B + "/admin/importa/stato"))
    if not s["running"]: break
    time.sleep(0.2)
with db() as c: n = c.execute("SELECT COUNT(*) FROM posts WHERE title LIKE 'Il ponte%'").fetchone()[0]
ok(s["skipped"] == 5 and n == 1, "rifatta con lo stesso file: nessun doppione, i 5 contenuti già importati vengono saltati")
ok("Il ponte" in curl(A, B + "/admin?q=canale"), "gli articoli importati si trovano con la ricerca nel testo")

print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
