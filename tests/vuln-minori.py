import subprocess, os, io, zipfile, urllib.parse, re, sqlite3, json
B="http://127.0.0.1:8161"; D="/tmp/mn"; os.chdir(D); fails=0
def curl(jar,*a,w="%{http_code} %{redirect_url}"):
    return subprocess.run(["curl","-s","-b",jar,"-c",jar,"-o","/dev/null","-w",w,*a],capture_output=True,text=True).stdout
def get(jar,path):
    return subprocess.run(["curl","-s","-b",jar,B+path],capture_output=True,text=True).stdout
def msg(r): return urllib.parse.unquote(r.split("msg=",1)[-1]) if "msg=" in r else r
def ok(c,l):
    global fails; fails+=0 if c else 1; print(("  OK  " if c else "  NO  ")+l)
curl("a.jar","-d","email=admin@x.it&password=password-lunga-123",B+"/admin/login")
curl("a.jar","-X","POST",B+"/admin/rebuild")
curl("a.jar","--form-string","name=Autore","--form-string","email=autore@x.it","--form-string","role=author","--form-string","password=password-lunga-123",B+"/admin/users/0")
curl("s.jar","-H","X-Real-IP: 10.1.0.1","-d","email=autore@x.it&password=password-lunga-123",B+"/admin/login")

print("Anteprima ricerca — /cerca/ esiste dopo la pubblicazione")
ok(os.path.exists("public/cerca/index.html"), "la pagina /cerca/ è stata generata")

print("V26 — categoria senza lettere latine non crea /category//")
r = curl("a.jar","--form-string","title=Test cat","--form-string","body=<p>x</p>","--form-string","category=中文","--form-string","status=published",B+"/admin/edit/0")
ok(not os.path.exists("public/category") or not any(d=="" for d in os.listdir("public/category")), "nessuna cartella categoria vuota")
ok("/category//" not in get("a.jar","/"), "nessun link /category// nella home")

print("V18 — un autore non sceglie la data di pubblicazione")
r = curl("s.jar","--form-string","title=Datato","--form-string","body=<p>x</p>","--form-string","published_at=2020-01-01T00:00","--form-string","status=pending",B+"/admin/edit/0")
pid = re.search(r"edit/(\d+)",r).group(1)
at = sqlite3.connect("presstatic.db").execute("select published_at from posts where id=?",(pid,)).fetchone()[0]
ok(at > 1735689600, f"la data del 2020 dell'autore è stata ignorata (timestamp {at})")

print("V20 — immagine in evidenza esterna rifiutata")
r = curl("s.jar","--form-string","title=Img esterna","--form-string","body=<p>x</p>","--form-string","image=https://tracker.example/pixel.jpg","--form-string","status=pending",B+"/admin/edit/0")
pid = re.search(r"edit/(\d+)",r).group(1)
img = sqlite3.connect("presstatic.db").execute("select image from posts where id=?",(pid,)).fetchone()[0]
ok(not img.startswith("http"), f"immagine esterna scartata (salvato: '{img}')")

print("V17 — cambio email senza password attuale rifiutato")
r = curl("s.jar","--form-string","name=Autore","--form-string","email=nuova@x.it",B+"/admin/users/2")
ok("attuale non" in msg(r) or "password attuale" in msg(r), "serve la password attuale per cambiare email: "+msg(r)[:50])

print("V14 — upload non segue un link simbolico")
os.makedirs("public/link-test",exist_ok=True)
open("/tmp/segreto.txt","w").write("SEGRETO")
if os.path.lexists("public/link-test/x.txt"): os.remove("public/link-test/x.txt")
os.symlink("/tmp/segreto.txt","public/link-test/x.txt")
open("x.txt","w").write("contenuto nuovo")
curl("a.jar","-F","action=upload","-F","path=public/link-test","-F","files=@x.txt",B+"/admin/files")
ok(open("/tmp/segreto.txt").read()=="SEGRETO", "il file fuori dalla cartella non è stato sovrascritto")

print("\nRISULTATO:", "tutto superato" if fails==0 else f"{fails} controlli non superati")
