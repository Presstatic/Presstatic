import subprocess, json, os, re, urllib.parse
B = "http://127.0.0.1:8092"; D = "/tmp/t7"; os.chdir(D)
def curl(*a, out="/dev/null", w="%{http_code} %{redirect_url}"):
    return subprocess.run(["curl", "-s", "-b", "j", "-c", "j", "-o", out, "-w", w, *a], capture_output=True, text=True).stdout
def msg(r): return urllib.parse.unquote(r.split("msg=", 1)[-1]) if "msg=" in r else r
def page(p): curl(B + p, out="/tmp/pg.html"); return open("/tmp/pg.html").read()
def pub(p): return open(f"{D}/public/{p.strip('/')}/index.html").read()
fails = 0
def check(c, l):
    global fails; fails += 0 if c else 1; print(("  OK  " if c else "  NO  ") + l)
def mocklog(): return [json.loads(l) for l in open("/tmp/mock/services.log")] if os.path.exists("/tmp/mock/services.log") else []
curl("-d", "email=admin@example.com&password=password-lunga-123", B + "/admin/login")
curl("--form-string", "site_name=Cronaca Pontina", "--form-string", "base_url=https://www.cronacapontina.it", "--form-string", "menu=Cronaca", "--form-string", "links_on=on", "--form-string", "links_max=3", "--form-string", "related=on", B + "/admin/settings")

print("== Integrazioni: Google")
sa = open("/tmp/mock/sa.json").read()
curl("--data-urlencode", f"google_sa={sa}", "-d", "google_on=on", B + "/admin/indicizzazione")  # l'indicizzazione ha una pagina sua
r = curl("--data-urlencode", "anthropic_key=sk-ant-TEST", "--data-urlencode", "openai_key=sk-TEST",
         "-d", "ai_text_model=anthropic:claude-sonnet-5", "-d", "ai_image_model=gpt-image-2", "--data-urlencode", "ai_style=Tono sobrio.", B + "/admin/integrations")
check("Integrazioni salvate" in msg(r), "salva chiave Google, chiavi IA, modelli e stile")
h = page("/admin/integrations")
check("sk-ant-TEST" not in h and "sk-TEST" not in h and "PRIVATE KEY" not in h, "chiavi e chiave privata non tornano mai nella pagina")
g = page("/admin/indicizzazione")
check("PRIVATE KEY" not in g and "presstatic@test.iam.gserviceaccount.com" in g and "Come si configura, passo per passo" in g, "Indicizzazione Google: mostra l'account collegato e la guida, mai la chiave privata")
check('href="/admin/indicizzazione"' in h and 'id="google"' not in h, "nella barra laterale c'è «Indicizzazione Google»; in Integrazioni la sezione non è più doppia")
check('name="anthropic_key"' in h and 'name="ai_text_model"' in h and 'name="anthropic_key"' not in g, "le impostazioni dell'IA si modificano da Integrazioni (e non sono finite nella pagina di Google)")
check('name="smtp_host"' in h and 'name="ga_property"' in h, "in Integrazioni restano anche Email e Più letti")
r = curl("-X", "POST", B + "/admin/indicizzazione/prova")
check("Collegamento riuscito" in msg(r), "«Prova il collegamento»: " + msg(r)[:90])
tok = [l for l in mocklog() if l.get("google") == "token"]
check(tok and tok[-1]["valid_signature"] and tok[-1]["scope"] == "https://www.googleapis.com/auth/indexing" and tok[-1]["grant"].endswith("jwt-bearer"), "JWT del service account firmato correttamente (verificato con la chiave pubblica)")
r = curl("--form-string", "title=Maltempo, allerta gialla su tutto il litorale", "--form-string", "body=<p>Testo.</p>", "--form-string", "category=Cronaca", "--form-string", "status=published", B + "/admin/edit/0")
check("Google avvisato (1 indirizzo)" in msg(r), "pubblicazione: " + msg(r)[:80])
pubs = [l for l in mocklog() if l.get("google") == "publish"]
check(pubs and pubs[-1]["type"] == "URL_UPDATED" and pubs[-1]["url"] == "https://www.cronacapontina.it/maltempo-allerta-gialla-su-tutto-il-litorale/" and pubs[-1]["auth"] == "Bearer ya29.TEST", "URL_UPDATED con l'indirizzo giusto")
pid = re.search(r"/admin/edit/(\d+)", r).group(1)
n = len(pubs)
r = curl("--form-string", "title=Maltempo, allerta gialla su tutto il litorale", "--form-string", "body=<p>Testo aggiornato.</p>", "--form-string", "category=Cronaca", "--form-string", "slug=maltempo-allerta-gialla-su-tutto-il-litorale", "--form-string", "status=published", B + f"/admin/edit/{pid}")
check(len([l for l in mocklog() if l.get("google") == "publish"]) == n, "modifica senza «avvisa anche per le modifiche»: nessun invio (quota risparmiata)")
r = curl("--form-string", "title=Maltempo, allerta gialla su tutto il litorale", "--form-string", "body=<p>Testo.</p>", "--form-string", "slug=maltempo-allerta-gialla-su-tutto-il-litorale", "--form-string", "status=draft", B + f"/admin/edit/{pid}")
pubs = [l for l in mocklog() if l.get("google") == "publish"]
check(pubs[-1]["type"] == "URL_DELETED", "ritiro dal sito: URL_DELETED")
check("Inviato" in page("/admin/indicizzazione"), "registro degli ultimi invii nella pagina")

print("== Scrivi con l'IA")
r = curl("--data-urlencode", "topic=Riapre il ponte sul canale", "--data-urlencode", "notes=Il ponte sul canale riapre il 25 settembre dopo otto mesi di lavori, annuncia il Comune.", "-d", "length=medium", "-d", "model=anthropic:claude-sonnet-5", "-d", "image=on", B + "/admin/ai")
check("Bozza scritta con Claude Sonnet 5" in msg(r), "Claude: " + msg(r)[:100])
a = [l for l in mocklog() if "anthropic" in l][-1]
check(a["anthropic"] == "claude-sonnet-5" and a["version"] == "2023-06-01" and a["has_source"] and a["no_nav"], "testo delle fonti passato al modello")
img = [l for l in mocklog() if "image" in l][-1]
check(img["image"] == "gpt-image-2" and img["size"] == "1536x1024" and img["format"] == "jpeg", "immagine generata con GPT Image 2, orizzontale, in JPEG")
aid = re.search(r"/admin/edit/(\d+)", r).group(1)
e = page(f"/admin/edit/{aid}")
check("Latina, il ponte sul canale riapre al traffico" in e and "viabilità" in e and "ponte sul canale" in e and "-1536x1024.jpg" in e and "Bozza" in e, "bozza con titolo, tag, parole chiave, immagine in evidenza; non pubblicata")
r = curl("--data-urlencode", "topic=Riapre il ponte", "-d", "model=openai:gpt-6-sol", B + "/admin/ai")
o = [l for l in mocklog() if "openai" in l][-1]
check("Bozza scritta con GPT-6 Sol" in msg(r) and o["openai"] == "gpt-6-sol" and o["json_mode"] == {"type": "json_object"}, "OpenAI GPT-6 Sol in modalità JSON")
curl("--data-urlencode", "anthropic_key=sk-ant-SBAGLIATA", B + "/admin/integrations")
h = subprocess.run(["curl", "-s", "-b", "j", "--data-urlencode", "topic=Prova errore", "-d", "model=anthropic:claude-opus-5-5", B + "/admin/ai"], capture_output=True, text=True).stdout
check("chiave API non valida" in h and "Prova errore" in h, "chiave sbagliata: errore chiaro e il modulo resta compilato")

print("== Link interni")
curl("--data-urlencode", "anthropic_key=sk-ant-TEST", B + "/admin/integrations")
def post(title, body, kw="", cat="Cronaca", pid=0, slug=""):
    return curl("--form-string", f"title={title}", "--form-string", f"body={body}", "--form-string", f"category={cat}", "--form-string", f"link_keywords={kw}", "--form-string", f"slug={slug}", "--form-string", "status=published", B + f"/admin/edit/{pid}")
post("Il ponte sul canale riapre", "<p>Articolo principale sul ponte.</p>", "ponte sul canale, viabilità Latina")
post("Traffico in città", "<p>Da domani il Ponte sul Canale torna percorribile e la viabilità Latina migliora.</p><h2>Il ponte sul canale</h2><p>Di nuovo ponte sul canale.</p>")
t = pub("/traffico-in-citta/")
body = t.split('<div class="body">')[1].split("</div>")[0]
check(body.count('href="https://www.cronacapontina.it/il-ponte-sul-canale-riapre/"') == 1, "la parola chiave diventa un link, una volta sola per articolo di destinazione")
check('<a href="https://www.cronacapontina.it/il-ponte-sul-canale-riapre/">Ponte sul Canale</a>' in body, "maiuscole del testo rispettate")
check("<h2>Il ponte sul canale</h2>" in body, "nessun link dentro i titoletti")
check("il-ponte-sul-canale-riapre" not in pub("/il-ponte-sul-canale-riapre/").split('<div class="body">')[1].split("</div>")[0], "nessun link dell'articolo verso se stesso")
r = post("Lavori in via Roma", "<p>Chiude via Roma. Intanto il ponte sul canale resta aperto.</p>")
check("il-ponte-sul-canale-riapre" in pub("/lavori-in-via-roma/"), "anche gli articoli nuovi ricevono il link")
main_id = re.search(r'href="/admin/edit/(\d+)">(?:<strong>)?Il ponte sul canale riapre', page("/admin?q=ponte")).group(1)
post("Il ponte sul canale riapre", "<p>Articolo principale sul ponte.</p>", "", pid=main_id, slug="il-ponte-sul-canale-riapre")
check("il-ponte-sul-canale-riapre" not in pub("/traffico-in-citta/").split('<div class="body">')[1].split('</div>')[0], "tolte le parole chiave, i link spariscono anche dagli altri articoli")
print("\nRISULTATO:", "tutto superato" if fails == 0 else f"{fails} controlli non superati")
