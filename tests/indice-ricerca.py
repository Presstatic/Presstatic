# L'indice di ricerca nuovo prende il posto del vecchio senza che /pagefind/ manchi nemmeno per un istante.
# Uso: python3 tests/indice-ricerca.py http://127.0.0.1:8200 /percorso/del/sito   (sito con il programma appena avviato)
import os, sys, time, subprocess
B, DIR = sys.argv[1], sys.argv[2]
entry, d = f"{DIR}/public/pagefind/pagefind-entry.json", f"{DIR}/public/pagefind"
jar = "/tmp/ir.jar"; open(jar, "w").close()
curl = lambda *a: subprocess.run(["curl", "-s", "-b", jar, "-c", jar, "-H", "Origin: " + B, *a], capture_output=True, text=True).stdout
curl("-o", "/dev/null", "--data-urlencode", "email=andrea@example.com", "--data-urlencode", "password=password-lunga-123", B + "/admin/login")
while not os.path.exists(entry): time.sleep(0.5)          # primo indice, 30 secondi dopo l'avvio
first = os.stat(d).st_ino
curl("-o", "/dev/null", "--form-string", "title=Articolo per rifare l'indice", "--form-string", "body=<p>Testo</p>", "--form-string", "status=published", B + "/admin/edit/0")
checks = misses = 0
end = time.time() + 150
while time.time() < end and os.stat(d).st_ino == first if os.path.exists(d) else True:
    checks += 1
    if not os.path.exists(entry): misses += 1
for _ in range(2000): checks += 1; misses += not os.path.exists(entry)
swapped = os.path.exists(d) and os.stat(d).st_ino != first
print(f"  {'OK' if swapped else 'NO'}  l'indice è stato rifatto e sostituito")
print(f"  {'OK' if misses == 0 else 'NO'}  {checks:,} controlli durante la sostituzione: l'indice c'era sempre ({misses} volte mancava)".replace(",", "."))
print("\nRISULTATO:", "tutto superato" if swapped and misses == 0 else "controlli non superati")
