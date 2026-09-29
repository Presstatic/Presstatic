# Verifica dei punti del rapporto di Fable. Stampa VULNERABILE o CORRETTO per ciascuno.
import subprocess, os, re, json, time, threading, urllib.parse, html.parser
B = "http://127.0.0.1:8085"; D = "/tmp/fc"; os.chdir(D)
def curl(jar, *a, out="/dev/null", w="%{http_code} %{redirect_url}"):
    return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-o", out, "-w", w, *a], capture_output=True, text=True).stdout
def msg(r): return urllib.parse.unquote(r.split("msg=", 1)[-1]) if "msg=" in r else r
pid = [p for p in os.popen("pgrep -x presstatic").read().split() if os.readlink(f"/proc/{p}/cwd") == D][0]
def rss(): return int(open(f"/proc/{pid}/status").read().split("VmRSS:")[1].split()[0]) // 1024
def verdict(vuln, label, extra=""): print(("  VULNERABILE  " if vuln else "  CORRETTO     ") + label + (f"  ({extra})" if extra else ""))
curl("a.jar", "-d", "email=admin@x.it&password=password-lunga-123", B + "/admin/login")
curl("a.jar", "--form-string", "name=Autore", "--form-string", "email=autore@x.it", "--form-string", "role=author", "--form-string", "password=password-lunga-123", B + "/admin/users/0")
curl("s.jar", "-d", "email=autore@x.it&password=password-lunga-123", B + "/admin/login")

# 1) tentativi di accesso in parallelo
out = []
def attempt(ip, pw, store=True):
    r = subprocess.run(["curl", "-s", "-o", "/dev/null", "-w", "%{redirect_url}", "-H", f"X-Forwarded-For: {ip}", "-H", f"X-Real-IP: {ip}", "-d", f"email=admin@x.it&password={pw}", B + "/admin/login"], capture_output=True, text=True).stdout
    if store: out.append(r)
    return r
ts = [threading.Thread(target=attempt, args=("203.0.113.7", f"sbagliata-{i}")) for i in range(60)]
[t.start() for t in ts]; [t.join() for t in ts]
checked = sum("non%20corrette" in r for r in out)
verdict(checked > 10, "1) limite di accesso aggirabile in parallelo", f"{checked} password verificate su 60 tentativi dallo stesso IP")
# 6) il blocco per account impedisce l'accesso al proprietario da un altro indirizzo
for ip in ["198.51.100.1", "198.51.100.2", "198.51.100.3"]:
    ts = [threading.Thread(target=attempt, args=(ip, f"x-{i}")) for i in range(10)]; [t.start() for t in ts]; [t.join() for t in ts]
r = attempt("192.0.2.200", "password-lunga-123", False)
verdict(not r.endswith("/admin"), "6) dopo gli errori di altri, il proprietario non entra con la password giusta", msg(r)[:60] if not r.endswith("/admin") else "entra")

# 5) email enormi al login
before = rss()
big = "email=" + "a" * 5_000_000 + "%40x.it&password=x"
open("big.txt", "w").write(big)
ts = [threading.Thread(target=lambda i=i: subprocess.run(["curl", "-s", "-o", "/dev/null", "-H", f"X-Forwarded-For: 10.9.{i}.1", "-H", f"X-Real-IP: 10.9.{i}.1", "--data-binary", "@big.txt", B + "/admin/login"])) for i in range(10)]
[t.start() for t in ts]; [t.join() for t in ts]; time.sleep(1)
verdict(rss() - before > 25, "5) memoria trattenuta da email enormi", f"{before} MB -> {rss()} MB con 10 email da 5 MB")

# 7) temi: chiavi segrete e file fuori dalla cartella
curl("a.jar", "--data-urlencode", "openai_key=sk-openai-SEGRETA", B + "/admin/integrations")
os.makedirs("themes/prova", exist_ok=True)
open("themes/prova/list.html", "w").write('{% extends "classico/base.html" %}{% block main %}CHIAVE=[{{ site.openai_key }}] FILE=[{% include "../../../../etc/hostname" ignore missing %}]{% endblock %}')
settings = ["--form-string", "site_name=Prova", "--form-string", "base_url=http://127.0.0.1:8085", "--form-string", "theme=prova", "--form-string", "ad_inarticle=<div>ANNUNCIO</div>", "--form-string", "ad_paragraph=2", "--form-string", "related=on", "--form-string", "links_on=on"]
curl("a.jar", *settings, B + "/admin/settings")
home = open(f"{D}/public/index.html").read()
key = re.search(r"CHIAVE=\[(.*?)\]", home); f = re.search(r"FILE=\[(.*?)\]", home, re.S)
verdict(bool(key and key.group(1)), "7a) un tema legge le chiavi API", f"CHIAVE=[{key.group(1) if key else '?'}]")
verdict(bool(f and f.group(1).strip()), "7b) un tema include file del server", f"FILE=[{f.group(1).strip() if f else '?'}]")
curl("a.jar", *[x.replace("theme=prova", "theme=classico") for x in settings], B + "/admin/settings")

# 3) iframe di un sito qualsiasi nel video
video = ["--form-string", "title=Video", "--form-string", "body=<p>x</p>", "--form-string", "schema_type=VideoObject", "--form-string", "sd_embedUrl=https://attaccante.example/finto-player.html"]
r = curl("s.jar", *video, "--form-string", "status=published", B + "/admin/edit/0"); vid = re.search(r"edit/(\d+)", r).group(1)
curl("a.jar", *video, "--form-string", "slug=video", "--form-string", "status=published", B + f"/admin/edit/{vid}")
page = open(f"{D}/public/video/index.html").read()
verdict("attaccante.example/finto-player" in page.split("<iframe")[1] if "<iframe" in page else False, "3) iframe di un sito qualsiasi tramite il link del video")

# nuova) script tramite alt e inserimento dell'annuncio
class Scripts(html.parser.HTMLParser):
    def __init__(s): super().__init__(); s.inside = False; s.found = []
    def handle_starttag(s, t, a): s.inside = t == "script"
    def handle_data(s, d):
        if s.inside and "alert(1)" in d: s.found.append(d)
    def handle_endtag(s, t): s.inside = False
curl("a.jar", "--form-string", "title=Alt", "--form-string", 'body=<p>a</p><img src="/media/x.jpg" alt="x</p><script>alert(1)</script>"><p>b</p><p>c</p>', "--form-string", "slug=alt", "--form-string", "status=published", B + "/admin/edit/0")
p = Scripts(); p.feed(open(f"{D}/public/alt/index.html").read())
verdict(bool(p.found), "nuova) script eseguibile creato dall'inserimento dell'annuncio dentro l'attributo alt")

# 2) IA usabile dagli autori
h = subprocess.run(["curl", "-s", "-b", "s.jar", "-o", "/dev/null", "-w", "%{http_code}", B + "/admin/ai"], capture_output=True, text=True).stdout
verdict(h == "200", "2b) un autore usa l'IA senza limiti", f"HTTP {h}")

# 4) permessi del database
mode = oct(os.stat(f"{D}/presstatic.db").st_mode & 0o777)
verdict(mode.endswith("44"), "4) database leggibile da tutti gli utenti del server", mode)

print("-- controlli aggiuntivi sulle correzioni")
# parole chiave limitate
r = curl("a.jar", "--form-string", "title=Chiavi", "--form-string", "body=<p>x</p>", "--form-string", "link_keywords=" + ", ".join(f"parola{i}" for i in range(50)), "--form-string", "status=draft", B + "/admin/edit/0")
kid = re.search(r"edit/(\d+)", r).group(1)
import sqlite3; db = sqlite3.connect(f"{D}/presstatic.db")
n = len(db.execute("SELECT link_keywords FROM posts WHERE id=?", (kid,)).fetchone()[0].split(","))
verdict(n > 10, "parole chiave per i link interni illimitate", f"{n} salvate su 50")
# GIF oltre 5 MB
open("grande.gif", "wb").write(b"GIF89a" + b"\0" * 6_000_000)
out = subprocess.run(["curl", "-s", "-b", "a.jar", "-F", "file=@grande.gif", B + "/admin/upload"], capture_output=True, text=True).stdout
verdict("error" not in out, "GIF da 6 MB accettata", out[:80])
# testo oltre 2 MB
r = curl("s.jar", "--form-string", "title=Lungo", "--form-string", "body=<p>" + "a" * 2_100_000 + "</p>", "--form-string", "status=draft", B + "/admin/edit/0")
verdict("troppo lungo" not in msg(r), "testo da 2,1 MB accettato", msg(r)[:60])
# collegamento a /dev/zero nella cartella pubblica
os.symlink("/dev/zero", f"{D}/public/zero")
t0 = time.time(); code = subprocess.run(["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", "--max-time", "5", B + "/zero"], capture_output=True, text=True).stdout
alive = subprocess.run(["curl", "-s", "-o", "/dev/null", "-w", "%{http_code}", B + "/admin/login"], capture_output=True, text=True).stdout
verdict(code != "404" or alive != "200", "/dev/zero letto dal servizio", f"risposta {code} in {time.time()-t0:.2f} s, servizio {'attivo' if alive == '200' else 'CADUTO'}")
# cookie __Host- in HTTPS
h = subprocess.run(["curl", "-s", "-D", "-", "-o", "/dev/null", "-H", "X-Forwarded-Proto: https", "-H", "X-Real-IP: 192.0.2.50", "-d", "email=autore@x.it&password=password-lunga-123", B + "/admin/login"], capture_output=True, text=True).stdout
verdict("__Host-ps=" not in h, "cookie di sessione senza prefisso __Host- in HTTPS")
# cambio password: serve quella attuale, e chiude le altre sessioni
curl("s2.jar", "-H", "X-Real-IP: 192.0.2.51", "-d", "email=autore@x.it&password=password-lunga-123", B + "/admin/login")
r = curl("s.jar", "--form-string", "name=Autore", "--form-string", "email=autore@x.it", "--form-string", "password=nuova-password-123", "--form-string", "current_password=sbagliata-12345", B + "/admin/users/2")
verdict("attuale non" not in msg(r), "cambio password senza conoscere quella attuale", msg(r)[:60])
curl("s.jar", "--form-string", "name=Autore", "--form-string", "email=autore@x.it", "--form-string", "password=nuova-password-123", "--form-string", "current_password=password-lunga-123", B + "/admin/users/2")
other = curl("s2.jar", B + "/admin")
verdict(other.startswith("200"), "dopo il cambio password l'altra sessione resta aperta", other[:40])
# video: YouTube ammesso
r = curl("a.jar", "--form-string", "title=Video YT", "--form-string", "body=<p>x</p>", "--form-string", "schema_type=VideoObject", "--form-string", "sd_embedUrl=https://www.youtube-nocookie.com/embed/abcdefghijk", "--form-string", "slug=video-yt", "--form-string", "status=published", B + "/admin/edit/0")
pg = open(f"{D}/public/video-yt/index.html").read()
verdict(not ('src="https://www.youtube-nocookie.com/embed/abcdefghijk"' in pg and 'sandbox="allow-scripts' in pg), "video YouTube non più incorporabile o senza sandbox")
