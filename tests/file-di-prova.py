# File di prova per i test: foto con orientamento EXIF, ritratto, logo e icona.
from PIL import Image
import sys, os
d = sys.argv[1]; os.makedirs(d, exist_ok=True)
img = Image.new("RGB", (3000, 2000), (120, 150, 200))
exif = Image.Exif(); exif[0x0112] = 6   # Orientation = ruotata di 90° (foto del telefono)
exif[0x8825] = {1: "N", 2: (41.0, 30.0, 0.0)}  # dati GPS, che il CMS deve togliere
img.save(f"{d}/grande.jpg", quality=92, exif=exif)
Image.new("RGB", (800, 800), (200, 120, 90)).save(f"{d}/volto.jpg", quality=90)
Image.new("RGB", (1600, 900), (60, 130, 90)).save(f"{d}/foto1.jpg", quality=90)
Image.new("RGBA", (600, 200), (20, 20, 20, 0)).save(f"{d}/logo.png")
Image.new("RGBA", (256, 256), (180, 20, 40, 255)).save(f"{d}/icona.png")
print("file di prova creati in", d)
