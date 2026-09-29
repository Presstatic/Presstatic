# Uso: python3 tests/importazione-entita.py http://127.0.0.1:PORTA /percorso/del/sito
# Prova apposta per le entità XML dopo l'aggiornamento di quick-xml: il testo importato deve restare identico.
import subprocess, sqlite3, time, sys, urllib.parse
B, DIR = sys.argv[1], sys.argv[2]
def c(*a): return subprocess.run(["curl","-s","-m","20","-b",DIR+"/j","-c",DIR+"/j","-H","Origin: "+B,*a],capture_output=True,text=True).stdout
c("-o","/dev/null","--data-urlencode","email=a@x.it","--data-urlencode","password=password-lunga-123",B+"/admin/login")
wxr='''<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/" xmlns:wp="http://wordpress.org/export/1.2/" xmlns:dc="http://purl.org/dc/elements/1.1/">
<channel><title>Prova</title>
<item><title>Rock &amp; Roll: caff&#233; &lt;forte&gt; &#x263A; &quot;virgolette&quot; &apos;apici&apos; &copy;</title>
<dc:creator>autore</dc:creator>
<content:encoded><![CDATA[<p>Nel CDATA &amp; resta com'è: <b>grassetto</b></p>]]></content:encoded>
<wp:post_id>501</wp:post_id><wp:post_date>2024-05-01 10:00:00</wp:post_date><wp:post_name>entita</wp:post_name>
<wp:status>publish</wp:status><wp:post_type>post</wp:post_type>
<category domain="category" nicename="cultura"><![CDATA[Cultura &amp; Spettacoli]]></category>
<category domain="post_tag" nicename="t"><![CDATA[musica]]></category>
</item></channel></rss>'''
open(DIR+"/e.xml","w").write(wxr)
c("-o","/dev/null","-F","wxr=@"+DIR+"/e.xml;type=text/xml",B+"/admin/importa")
for _ in range(40):
    st=c(B+"/admin/importa/stato")
    if '"running":false' in st: break
    time.sleep(0.5)
db=sqlite3.connect(DIR+"/presstatic.db")
row=db.execute("select title, body, category from posts where slug='entita'").fetchone()
atteso="Rock & Roll: caffé <forte> ☺ \"virgolette\" 'apici' &copy;"
print("titolo importato:", repr(row[0]) if row else None)
print("titolo come atteso:", "SI" if row and row[0]==atteso else f"NO (atteso {atteso!r})")
print("testo dal CDATA intatto:", "SI" if row and "Nel CDATA &amp; resta" in row[1] or (row and "Nel CDATA & resta" in row[1]) else f"NO: {row[1] if row else None!r}")
print("categoria:", repr(row[2]) if row else None)
