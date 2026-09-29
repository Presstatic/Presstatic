import subprocess, os, io, zipfile, urllib.parse, re
B = "http://127.0.0.1:8083"; D = "/tmp/fm"; os.chdir(D); fails = 0
def check(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l)
def curl(jar, *a, out="/dev/null", w="%{http_code} %{redirect_url}"):
    return subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-o", out, "-w", w, *a], capture_output=True, text=True).stdout
def msg(r): return urllib.parse.unquote(r.split("msg=", 1)[-1]) if "msg=" in r else r
curl("a.jar", "-d", "email=admin@x.it&password=password-lunga-123", B + "/admin/login")
curl("a.jar", "--form-string", "name=Autore", "--form-string", "email=autore@x.it", "--form-string", "role=author", "--form-string", "password=password-lunga-123", B + "/admin/users/0")
curl("s.jar", "-d", "email=autore@x.it&password=password-lunga-123", B + "/admin/login")
print("File manager")
check(curl("s.jar", B + "/admin/files?path=public").startswith("403"), "un autore non entra nel file manager")
check(curl("a.jar", B + "/admin/files?path=public").startswith("200"), "l'amministratore vede la cartella public")
r = curl("a.jar", "-F", "action=mkdir", "-F", "path=public", "-F", "name=documenti", B + "/admin/files")
check(os.path.isdir("public/documenti") and "creata" in msg(r), "nuova cartella")
open("regolamento.pdf", "wb").write(b"%PDF-1.4 prova"); open("foto.txt", "w").write("ciao")
r = curl("a.jar", "-F", "action=upload", "-F", "path=public/documenti", "-F", "files=@regolamento.pdf", "-F", "files=@foto.txt", B + "/admin/files")
check(os.path.exists("public/documenti/regolamento.pdf") and os.path.exists("public/documenti/foto.txt"), "caricamento di due file insieme: " + msg(r)[:60])
h = subprocess.run(["curl", "-s", "-b", "a.jar", "-D", "-", "-o", "/tmp/fm/scaricato.pdf", B + "/admin/files/download?path=public/documenti/regolamento.pdf"], capture_output=True, text=True).stdout.lower()
check("attachment" in h and "application/pdf" in h and open("scaricato.pdf", "rb").read() == b"%PDF-1.4 prova", "download del file con il tipo giusto")
# ZIP con un file normale, uno che tenta di uscire dalla cartella e un link simbolico
buf = io.BytesIO()
with zipfile.ZipFile(buf, "w") as z:
    z.writestr("tema-mio/base.html", "<html>tema</html>"); z.writestr("tema-mio/css/stile.css", "body{}")
    z.writestr("../../../../tmp/fm/EVASO.txt", "male"); z.writestr("/etc/EVASO2.txt", "male")
    info = zipfile.ZipInfo("link"); info.external_attr = (0o120777 << 16); z.writestr(info, "/etc/passwd")
open("tema.zip", "wb").write(buf.getvalue())
curl("a.jar", "-F", "action=upload", "-F", "path=themes", "-F", "files=@tema.zip", B + "/admin/files")
r = curl("a.jar", "-F", "action=extract", "-F", "path=themes/tema.zip", B + "/admin/files")
check(os.path.exists("themes/tema-mio/base.html") and os.path.exists("themes/tema-mio/css/stile.css"), "ZIP estratto nella cartella giusta: " + msg(r)[:60])
check(not os.path.exists("/tmp/fm/EVASO.txt") and not os.path.exists("/etc/EVASO2.txt") and not os.path.lexists("themes/link"), "voci dello ZIP con «..», percorsi assoluti e link simbolici saltate")
for p in ["public/../presstatic.db", "../etc/passwd", "presstatic.db", "themes/../../fm/presstatic.db", "public/..%2Fpresstatic.db"]:
    r = curl("a.jar", B + "/admin/files?path=" + urllib.parse.quote(p, safe=""))
    h = subprocess.run(["curl", "-s", "-b", "a.jar", "-o", "/dev/null", "-w", "%{http_code} %{redirect_url}", B + "/admin/files/download?path=" + urllib.parse.quote(p, safe="")], capture_output=True, text=True).stdout
    check("Errore" in msg(r) and ("Errore" in msg(h) or h.startswith("303")), f"rifiutato il percorso {p}")
os.symlink("/etc", "public/documenti/etc-link")
r = curl("a.jar", B + "/admin/files?path=public/documenti/etc-link")
check("Errore" in msg(r), "un link simbolico che porta fuori dalla cartella non viene seguito")
check("Errore" in msg(curl("a.jar", "-F", "action=delete", "-F", "path=public", B + "/admin/files")), "la cartella principale non si elimina")
r = curl("a.jar", "-F", "action=delete", "-F", "path=public/documenti", B + "/admin/files")
check(not os.path.exists("public/documenti") and "Eliminato" in msg(r), "eliminazione di una cartella con il contenuto")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
