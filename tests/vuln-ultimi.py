import subprocess, os, re, io, sqlite3, urllib.parse, threading, struct
B="http://127.0.0.1:8171"; D="/tmp/lt"; os.chdir(D); fails=0
def curl(jar,*a,w="%{http_code} %{redirect_url}"):
    return subprocess.run(["curl","-s","-b",jar,"-c",jar,"-o","/dev/null","-w",w,*a],capture_output=True,text=True).stdout
def msg(r): return urllib.parse.unquote(r.split("msg=",1)[-1]) if "msg=" in r else r
def ok(c,l):
    global fails; fails+=0 if c else 1; print(("  OK  " if c else "  NO  ")+l)
def login(jar, email, pw, ip):
    return subprocess.run(["curl","-s","-c",jar,"-o","/dev/null","-w","%{redirect_url}","-H",f"X-Real-IP: {ip}","-d",f"email={email}&password={pw}",B+"/admin/login"],capture_output=True,text=True).stdout
login("a.jar","admin@x.it","password-lunga-123","10.9.9.9")
curl("a.jar","--form-string","name=Collega","--form-string","email=collega@x.it","--form-string","role=editor","--form-string","password=password-lunga-123",B+"/admin/users/0")

print("V9 — l'errore di uno non blocca i colleghi dallo stesso indirizzo")
ts=[threading.Thread(target=login,args=("x.jar","admin@x.it",f"sbagliata-{i}","203.0.113.50")) for i in range(12)]; [t.start() for t in ts]; [t.join() for t in ts]
ok("Troppi" in msg(login("x.jar","admin@x.it","password-lunga-123","203.0.113.50")), "la coppia indirizzo+account con 10 errori è bloccata")
ok(login("c.jar","collega@x.it","password-lunga-123","203.0.113.50").endswith("/admin"), "un collega dallo stesso indirizzo entra normalmente")
ts=[threading.Thread(target=login,args=("y.jar",f"u{i}@x.it","sbagliata-123","203.0.113.60")) for i in range(52)]; [t.start() for t in ts]; [t.join() for t in ts]
ok("Troppi" in msg(login("y.jar","collega@x.it","password-lunga-123","203.0.113.60")), "50 errori su account diversi bloccano quell'indirizzo")

print("V13 — AVIF rifiutato, GIF ripulita dai metadati")
avif = b"\0\0\0\x1cftypavif" + b"\0"*100
open("f.avif","wb").write(avif)
out = subprocess.run(["curl","-s","-b","a.jar","-F","file=@f.avif",B+"/admin/upload"],capture_output=True,text=True).stdout
ok("AVIF" in out and "error" in out, "AVIF rifiutato con spiegazione")
from PIL import Image
from PIL import ImageDraw
frames=[]
for i in range(3):
    f=Image.new("RGB",(40,40),(255,255,255)); ImageDraw.Draw(f).rectangle((i*10,i*10,i*10+12,i*10+12),fill=(200,30,30)); frames.append(f.convert("P"))
buf=io.BytesIO(); frames[0].save(buf,"GIF",save_all=True,append_images=frames[1:],loop=0,duration=100,comment=b"SEGRETO-COMMENTO",optimize=False,disposal=2)
g=bytearray(buf.getvalue())
xmp=b"\x21\xff\x0bXMP DataXMP"+b"\x0f"+b"GPS-41.9N-12.5E"+b"\x00"
g=g[:-1]+xmp+b"\x3b"  # estensione XMP con metadati prima della fine
open("anim.gif","wb").write(bytes(g))
out = subprocess.run(["curl","-s","-b","a.jar","-F","file=@anim.gif",B+"/admin/upload"],capture_output=True,text=True).stdout
url = re.search(r'"url":"([^"]+)"',out)
if url:
    data=open("public"+url.group(1),"rb").read()
    ok(b"SEGRETO-COMMENTO" not in data and b"GPS-41" not in data, "commento e metadati XMP tolti")
    im=Image.open("public"+url.group(1)); n=getattr(im,"n_frames",1)
    ok(n==3, f"la GIF resta animata ({n} fotogrammi) e leggibile")
else:
    ok(False, "GIF caricata: "+out[:80])

print("V19 — la parola chiave resta all'articolo che l'ha usata per primo")
curl("a.jar","-X","POST",B+"/admin/rebuild")
r1=curl("a.jar","--form-string","title=Primo ponte","--form-string","slug=primo-ponte","--form-string","body=<p>x</p>","--form-string","link_keywords=ponte sul fiume","--form-string","status=published","--form-string","published_at=2026-01-01T10:00",B+"/admin/edit/0")
r2=curl("a.jar","--form-string","title=Secondo ponte","--form-string","slug=secondo-ponte","--form-string","body=<p>y</p>","--form-string","link_keywords=ponte sul fiume","--form-string","status=published",B+"/admin/edit/0")
r3=curl("a.jar","--form-string","title=Terzo","--form-string","slug=terzo","--form-string","body=<p>Oggi parliamo del ponte sul fiume in città.</p>","--form-string","status=published",B+"/admin/edit/0")
page=open("public/terzo/index.html").read()
ok("/primo-ponte/" in page and "/secondo-ponte/\">ponte" not in page, "il link va al primo articolo che ha usato la parola chiave")

print("V22 — niente link dentro un'entità HTML")
curl("a.jar","--form-string","title=Amp","--form-string","slug=amp-art","--form-string","body=<p>x</p>","--form-string","link_keywords=amp","--form-string","status=published",B+"/admin/edit/0")
curl("a.jar","--form-string","title=Entita","--form-string","slug=entita","--form-string","body=<p>Tom &amp; Jerry e amp da solo.</p>","--form-string","status=published",B+"/admin/edit/0")
page=open("public/entita/index.html").read()
ok("&amp; Jerry" in page, "l'entità &amp; resta intatta")

print("V30 — Zone ID di Cloudflare non valido rifiutato")
r=curl("a.jar","--form-string","site_name=Prova","--form-string","base_url=http://127.0.0.1:8171","--form-string","theme=classico","--form-string","cf_zone=../../accounts",B+"/admin/settings")
ok("Zone ID" in msg(r), msg(r)[:70])

print("\nRISULTATO:", "tutto superato" if fails==0 else f"{fails} controlli non superati")
