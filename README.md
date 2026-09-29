# Presstatic

> **Stai cercando come installarlo e usarlo?** Leggi la [guida d'uso](docs/GUIDA.md): installazione passo passo, primi passi e domande frequenti. Questo README è la documentazione tecnica.

CMS per siti di notizie scritto in Rust. Quando pubblichi un articolo, Presstatic genera pagine HTML statiche: il sito lo serve Nginx (o Apache) come semplici file, con Cloudflare davanti. Il programma Presstatic serve solo al pannello della redazione, quindi un picco di traffico non lo tocca.

Cosa fa:

- pannello con editor visuale, anteprima su Google, conteggio parole, cronologia delle versioni con ripristino e avviso per le modifiche non salvate;
- redazione con ruoli: autore (scrive e invia in revisione), redattore (approva e pubblica), amministratore (gestisce anche utenti e impostazioni);
- pagine autore con foto e biografia, pagina «La redazione», pagine statiche (Chi siamo, Contatti, Privacy, Cookie policy già pronte come bozze);
- categorie, tag e ricerca interna con Pagefind, incluso nel programma: la ricerca funziona nel browser del lettore, senza nessun server di ricerca e niente da installare;
- redazione al lavoro in contemporanea: chi apre un articolo già aperto da un collega lo vede e può subentrare (come il blocco degli articoli di WordPress); chi salva sopra il lavoro di un altro non lo sovrascrive, la sua versione va nella cronologia; le pubblicazioni sono in coda, così due articoli pubblicati nello stesso istante finiscono entrambi in home; articolo e cronologia si salvano in un'unica transazione;
- reindirizzamenti automatici: quando cambi l'indirizzo di un articolo online, il vecchio porta subito al nuovo (Google lo tratta come permanente, come un 301); quando elimini un articolo online scegli dove portare chi arriva al suo indirizzo: un altro articolo o pagina, la home, oppure «pagina non trovata»;
- accesso sicuro: recupero della password via email (link valido un'ora, una volta sola), verifica in due passaggi con le app di autenticazione e 10 codici di recupero, email di prova e guida alla configurazione SMTP nel pannello (Integrazioni > Email);
- ricerca nel titolo e nel testo degli articoli dal pannello, senza accenti e con parole scritte a metà (indice SQLite FTS5);
- libreria media: tutte le immagini con ricerca, caricamento trascinando i file, testo alternativo, didascalia e credito fotografico; foto con didascalia e gallerie nell'editor; didascalia e credito sotto la foto in evidenza;
- importazione da WordPress dal file di esportazione (WXR): articoli, pagine, bozze, programmati, autori, categoria principale e descrizione di Yoast, tag, immagini scaricate e convertite, editor a blocchi e classico, shortcode [caption] e [gallery], video, reindirizzamenti dai vecchi indirizzi; rifarla non crea doppioni;
- note della redazione sugli articoli (mai sul sito; l'autore riceve un'email) e azioni in blocco dall'elenco: pubblica, riporta in bozza, metti o togli dall'evidenza, cambia categoria, aggiungi un tag, elimina, con un solo aggiornamento del sito;
- categorie ad albero (Sport > Calcio: gli articoli delle sottocategorie compaiono anche nella madre), categoria principale più categorie aggiuntive, descrizione della categoria sul sito, rinomina ed eliminazione con reindirizzamento; coautori nella firma, nei dati per Google e nelle pagine autore;
- commenti statici, attivabili dalle Impostazioni: i commenti approvati sono scritti nella pagina dell'articolo e il modulo è HTML senza JavaScript (Lighthouse 100 in tutte le categorie, spenti e accesi); moderazione con risposte della redazione, antispam senza servizi esterni, chiusura per articolo o dopo N giorni, avviso via email;
- «I più letti» in home con le visite di Google Analytics 4 (Data API, stesso service account dell'indicizzazione): aggiornati ogni 30 minuti rigenerando solo la home, senza contatori né richieste in più da chi legge (Lighthouse 100 spenti e accesi);
- newsletter: modulo di iscrizione HTML in fondo agli articoli (solo se attiva), doppia conferma, riepilogo automatico dei nuovi articoli ogni giorno o settimana all'ora scelta, disiscrizione con un clic (List-Unsubscribe), invio con una sola connessione SMTP e avanzamento nel pannello, esportazione CSV degli iscritti;
- pulsanti di condivisione con i colori dei social (WhatsApp, Facebook, X, Telegram, LinkedIn, Email), testo scuro o chiaro scelto per un contrasto di almeno 4,5:1, loghi ufficiali caricabili dalle Impostazioni; semplici link, senza script dei social;
- notifiche push dei browser senza servizi esterni: chiavi VAPID generate dal pannello, messaggi cifrati secondo lo standard Web Push (RFC 8291), pulsante sotto gli articoli con lo script scaricato solo al clic, avviso automatico alla prima pubblicazione di ogni articolo, invio a mano, iscrizioni scadute tolte da sole;
- backup giornalieri: copia coerente del database, immagini, temi e file caricati in un archivio .tar.gz, inviato a un archivio compatibile S3 (Wasabi, Backblaze B2, Hetzner Object Storage, Amazon S3, Cloudflare R2) con firma AWS versione 4 e caricamento a pezzi per gli archivi grandi; copie da conservare nell'archivio e sul server; ripristino con `presstatic ripristina <archivio>`;
- page builder per la home (Pannello > Costruisci la home): sezioni, colonne e widget con trascinamento, anteprima vera nel tema, impostazioni per computer, tablet e telefono, annulla/ripeti, modelli di partenza, bozza e pubblicazione; widget di base (titolo, testo, immagine, pulsante, separatore, spazio, video, elenco) e da testata (apertura, griglia ed elenco di articoli per categoria, ultime notizie, più letti, newsletter, pubblicità, HTML); la home pubblicata è HTML e CSS statici, senza script dell'editor;
- testata, menu e piè di pagina costruiti con lo stesso editor (Costruisci il sito > Testata e menu / Piè di pagina), validi su tutte le pagine: widget logo, menu (dal menu del sito, dalle sezioni o scritto a mano; sul telefono diventa un pulsante «Menu» fatto solo con HTML), ricerca, social con i loro colori, sezioni, copyright e note legali; ritorno alla versione del tema con un clic;
- page builder anche per le pagine singole (per esempio «Chi siamo»), con modello di partenza; widget avanzati senza JavaScript: schede (anche da tastiera), fisarmonica, carosello di articoli, galleria, numero in evidenza, citazione, scheda con foto e pulsante;
- «I miei blocchi»: una sezione composta si salva con un nome (★ nella barra dell'editor) e si inserisce come copia indipendente in qualsiasi pagina costruita;
- indicizzazione rapida (Pannello > Indicizzazione rapida): Google Indexing API e IndexNow (Bing, Yandex, Seznam, Naver e gli altri motori che aderiscono), senza account; chiave e file di verifica creati da soli, invii a pubblicazione, modifica, cambio di indirizzo ed eliminazione, registro degli ultimi invii;
- scrittura assistita con l'intelligenza artificiale (Anthropic Claude o OpenAI GPT): dalle fonti di una notizia una bozza già formattata e ottimizzata per la SEO, con l'immagine in evidenza generata;
- link interni automatici: le parole chiave di un articolo diventano link negli altri articoli;
- Google Indexing API: Google viene avvisato appena un articolo esce, cambia o viene tolto;
- immagini come Ghost: versioni WebP a più larghezze con srcset, foto del telefono raddrizzate, dati EXIF (compresa la posizione GPS) rimossi;
- due temi con 100/100 su PageSpeed (Lighthouse) da computer e da mobile in tutte e quattro le categorie: Classico (prima pagina da quotidiano a tre colonne, sezioni in coppia, carattere Newsreader) e Moderno (testata ampia con logo in evidenza, apertura con la linea del tempo delle ultime notizie, sezioni affiancate, tema scuro automatico, carattere Schibsted Grotesk). Entrambi adattano ogni blocco al numero di articoli disponibili, senza lasciare spazi vuoti. Entrambi con menu, sezioni in home, articoli in evidenza scelti dalla redazione, piè di pagina a colonne, profili social, note legali, logo, icona, colore, articoli correlati (disattivabili), condivisione, tempo di lettura, barra di lettura e pagina 404;
- feed RSS, `robots.txt`, `ads.txt`, sitemap indice con un file per mese (con immagini) e sitemap Google News delle ultime 48 ore;
- SEO: canonical, Open Graph, Twitter card, `max-image-preview:large` per Discover;
- dati strutturati: NewsArticle, Article, BlogPosting, Recipe, Review, VideoObject a scelta per ogni articolo, più BreadcrumbList, WebSite con ricerca, Organization con i profili social, ProfilePage per gli autori;
- script di tracciamento nell'intestazione e subito dopo `<body>`; pubblicità nell'intestazione, sotto il titolo, dentro il testo dopo il paragrafo N (anche ripetuto) e a fine articolo;
- Cloudflare via API: regola di cache per l'HTML, Tiered Cache Smart, svuotamento mirato della cache a ogni pubblicazione.

Prestazioni misurate su un solo core, con 20.000 articoli, 10 autori, 300 tag, articoli correlati e link interni attivi: rigenerazione completa in circa 4 secondi (5,6 la prima volta, quando tutti i file sono nuovi), pubblicazione di un articolo in circa mezzo secondo. La generazione usa tutti i core del server, quindi su un VPS con più core è più rapida. L'indice di ricerca di 20.000 articoli si crea in circa 35 secondi, in background, al massimo ogni 2 minuti.

PageSpeed: misurato con Lighthouse 13 su home, articolo e categoria di entrambi i temi, serviti da Nginx: 100 in prestazioni, accessibilità, best practice e SEO, sia da mobile sia da computer. Pubblicità e script esterni aggiunti dalle impostazioni abbassano il punteggio in proporzione al loro peso.

## 1. Compilare

Serve Rust 1.80 o successivo (<https://rustup.rs>) e un compilatore C (per le librerie WebP e crittografiche): su Ubuntu `sudo apt install build-essential`.

```sh
cargo build --release
```

Il programma è `target/release/presstatic`, un solo file. Il binario già pronto nella consegna gira su Linux x86_64 con glibc 2.34 o successiva: Ubuntu 22.04 e 24.04, Debian 12 e 13, Rocky/Alma 9.

## 2. Installare sul server

Serve un server Ubuntu 22.04 o 24.04, oppure Debian 12 o 13 (va bene un VPS da pochi euro al mese), e un dominio. Presstatic non gira su un hosting condiviso solo PHP. Non c'è nessun database da creare o collegare: Presstatic salva tutto in un file (`presstatic.db`) che crea da solo.

**Installazione automatica (consigliata)**

1. Nel DNS del dominio crea tre record A verso l'IP del server: `miosito.it`, `www.miosito.it` e `admin.miosito.it`. Se usi Cloudflare, durante l'installazione lasciali con la nuvola grigia (solo DNS).
2. Sul server (anche dalla console nel browser del provider) lancia un solo comando, sostituendo dominio ed email:

   ```sh
   curl -fsSL https://raw.githubusercontent.com/UTENTE/presstatic/main/deploy/install.sh | sudo bash -s -- miosito.it tua@email.it
   ```

   `UTENTE/presstatic` è il repository GitHub di chi distribuisce Presstatic (vedi «Pubblicare Presstatic»). Lo script scarica l'ultima versione dalle release, ne verifica la firma, controlla il DNS, installa Nginx e Certbot, crea l'utente e il servizio, configura il sito su `www` e il pannello su `admin`, e ottiene il certificato HTTPS gratuito di Let's Encrypt. In alternativa, se hai già i file sul server, entra nella cartella con `presstatic` e `install.sh` e lancia `sudo bash install.sh miosito.it tua@email.it`.
3. Alla fine lo script stampa un indirizzo come `https://admin.miosito.it/admin/setup?token=…`. Aprilo e segui l'installazione guidata: nome e indirizzo del sito, account amministratore, tema, colore, logo, sezioni e, se vuoi, Cloudflare e le chiavi dell'intelligenza artificiale. Con «Installa e metti online il sito» Presstatic genera le pagine e ti porta nel pannello, già collegato.
4. Se usi Cloudflare, ora puoi mettere la nuvola arancione su `www` **e anche su `admin`** e scegliere SSL/TLS **Full (strict)**: il certificato di Let's Encrypt va bene. Poi lancia `sudo bash /var/www/presstatic/solo-cloudflare.sh` (l'installazione lo mette lì): da quel momento il sito pubblico risponde solo attraverso Cloudflare, e chi scopre l'indirizzo IP del server non può aggirarlo. Rilancialo una volta al mese per aggiornare l'elenco degli indirizzi di Cloudflare; `--togli` lo disattiva.

Il programma viene messo in `/var/www/presstatic/bin/`, di proprietà del servizio: è quello che permette al pannello di aggiornarsi da solo. Per aggiornare a mano rilancia lo stesso comando: articoli, utenti e impostazioni restano.

Il codice nell'indirizzo dell'installazione guidata impedisce che qualcun altro configuri il sito al posto tuo: lo conosce solo chi ha accesso al server (è anche nel file `/var/www/presstatic/setup-token.txt`, leggibile solo dal servizio). Finita l'installazione, la pagina non è più raggiungibile.

**Installazione manuale**

```sh
sudo useradd --system --home /var/www/presstatic presstatic
sudo mkdir -p /var/www/presstatic
sudo mkdir -p /var/www/presstatic/bin && sudo cp presstatic /var/www/presstatic/bin/ && sudo chmod 755 /var/www/presstatic/bin/presstatic
sudo chown -R presstatic: /var/www/presstatic
sudo cp deploy/presstatic.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now presstatic
```

Al primo avvio Presstatic scrive nel log del servizio (`journalctl -u presstatic`) e nel file `setup-token.txt` il codice dell'installazione guidata: apri `/admin/setup?token=…` sul dominio del pannello. In alternativa puoi creare l'amministratore dalla riga di comando con `cd /var/www/presstatic && sudo -u presstatic bin/presstatic adduser`. Poi configura il web server come descritto nel punto 3.

Variabili facoltative: `PRESSTATIC_ADDR` (predefinito `127.0.0.1:8080`), `PRESSTATIC_DB` (`presstatic.db`), `PRESSTATIC_PUBLIC` (`public`).

## 3. Web server

**Nginx**: copia `deploy/nginx.conf` in `/etc/nginx/sites-enabled/`, sostituisci `miosito.it` con il tuo dominio, poi `nginx -t && systemctl reload nginx`. Il sito pubblico (`www.miosito.it`) è servito direttamente dalla cartella `public/`; il pannello sta su `admin.miosito.it` in HTTPS. La configurazione è provata con Nginx 1.24.

**Apache**: per il sito pubblico usa `deploy/apache-sito.conf` (servono `mod_rewrite`, `mod_headers`, `mod_mime`): invia ai browser le versioni già compresse delle pagine (`.br` e `.gz`) che Presstatic prepara, invece di comprimere a ogni visita; nelle prove, circa 21.000 richieste al secondo su un core invece di circa 1.100. Copia anche `deploy/.htaccess` dentro `public/`. Per il pannello usa `deploy/apache-admin.conf` (servono `mod_proxy`, `mod_proxy_http`, `mod_headers`, `mod_ssl`). Con i file statici Nginx resta circa 4 volte più veloce di Apache ed è il server consigliato.

Il pannello deve stare dietro HTTPS: il cookie di accesso diventa `Secure` solo così. Se puoi, limita il dominio del pannello agli IP della redazione (righe `allow`/`deny` nel file Nginx).

Per provare in locale senza web server: avvia `presstatic`, apri <http://127.0.0.1:8080/admin>. Finché l'indirizzo del sito nelle impostazioni è `http://127.0.0.1:8080`, anche il sito si vede lì.

## 4. Cloudflare

1. Il dominio deve essere su Cloudflare con la nuvola arancione attiva sul record `www`. In SSL/TLS scegli la modalità **Full (strict)** e crea un Origin Certificate da installare sul server (i percorsi sono già nel file Nginx).
2. Crea un token in *My Profile > API Tokens > Create Token > Custom token* con questi permessi, limitato alla tua zona:
   - Zone > Cache Rules > Edit
   - Zone > Zone Settings > Edit
   - Zone > Cache Purge > Purge

   Se la configurazione risponde con un errore di autorizzazione, aggiungi anche Account > Account Rulesets > Edit e Account > Account Filter Lists > Edit, che la documentazione Cloudflare elenca per le Cache Rules.
3. Nel pannello, *Impostazioni*: inserisci l'indirizzo del sito (per esempio `https://www.miosito.it`), lo Zone ID (pagina Panoramica del dominio) e il token, salva, poi premi **Configura Cloudflare**.

Cosa succede:

- Tiered Cache e topologia Smart vengono attivati;
- viene creata, o aggiornata se esiste già, la regola «Presstatic: cache HTML»: tutto l'HTML del dominio diventa idoneo alla cache con Edge TTL di 2 ore, il minimo del piano Free. Le altre regole di cache che hai non vengono toccate;
- a ogni pubblicazione, modifica, ritiro o eliminazione Presstatic svuota la cache dell'articolo e delle prime 3 pagine di home, categoria, tag e autore in cui compare, più feed e sitemap. Salvare le impostazioni o premere «Rigenera il sito» svuota tutta la cache.

Le pagine di archivio più profonde (dalla quarta in poi) si aggiornano su Cloudflare entro le 2 ore della Edge TTL. Il browser dei lettori tiene l'HTML per 60 secondi (intestazione impostata da Nginx o Apache).

Il token resta sul server: il pannello non lo rimostra mai. Per cambiarlo scrivi quello nuovo; se lasci il campo vuoto resta quello salvato.

## 5. Uso quotidiano

**La redazione.** In *Utenti* l'amministratore crea gli account e sceglie il ruolo:

- **Autore**: scrive i propri articoli e li invia in revisione; non pubblica, non modifica articoli altrui né quelli già pubblicati. Dal suo testo vengono tolti script e codici incorporati (restano formattazione, immagini e video di YouTube e Vimeo).
- **Redattore**: modifica e pubblica tutti gli articoli e le pagine, approva il lavoro degli autori, può incorporare qualsiasi codice. Accanto ad «Articoli» vede quanti articoli aspettano la revisione.
- **Amministratore**: in più gestisce utenti, impostazioni, tema e Cloudflare.
- **Disattivato**: non accede più (anche se era collegato), i suoi articoli restano online.

Ognuno, da *Il mio profilo*, cambia nome, foto, biografia e password. Nome, foto e biografia compaiono nella firma, nel riquadro sotto gli articoli e nella pagina autore: aiutano Google a capire chi scrive.

**Scrivere.**

- **Nuovo articolo**: titolo, sommario, testo, categoria, tag (separati da virgola), immagine in evidenza. Il redattore sceglie anche l'autore.
- **Editor**: titoletti, grassetto, corsivo, link, elenchi, citazione, immagine (con descrizione per Google), video di YouTube o Vimeo e il pulsante `</>` per incorporare post di Instagram, X, TikTok, Facebook o una mappa. Il testo incollato da Word o Google Docs perde colori e caratteri strani.
- **Anteprima su Google**: mostra come apparirà il risultato e avvisa se titolo o sommario sono troppo lunghi o troppo corti.
- **Pubblicare**: il redattore preme *Pubblica*; l'autore preme *Invia in revisione*. Con una data futura l'articolo esce da solo a quell'ora. *Ritira dal sito* lo riporta in bozza. Ctrl+S (⌘+S su Mac) salva senza cambiare lo stato.
- **Cronologia**: nella colonna a destra c'è l'elenco delle versioni salvate (fino a 50 per articolo). Ogni versione si apre e si può ripristinare; quella attuale resta comunque nella cronologia.
- **Elenco articoli**: schede Tutti, Pubblicati, In revisione, Programmati e Bozze, con ricerca per titolo. Gli autori vedono solo i propri.

**Scrivere con l'intelligenza artificiale.** *Scrivi con l'IA*: scrivi di cosa parla la notizia, incolla gli indirizzi delle fonti (fino a sei) o il loro testo, scegli modello e lunghezza. Presstatic legge le pagine (solo l'articolo, senza menu e pubblicità) e il modello prepara una **bozza** con titolo, sommario, titoletti, categoria, tag, parole chiave per i link interni e le fonti in fondo; se vuoi, anche l'immagine in evidenza. Nulla viene pubblicato: rileggi e controlla fatti, nomi e citazioni. Le immagini generate sono illustrazioni, non foto: non usarle per mostrare un fatto reale.

**Link interni.** Nel campo *Parole chiave per i link interni* di un articolo scrivi le espressioni per cui gli altri articoli dovrebbero rimandare a lui (per esempio «ponte sul canale»). Quando compaiono nel testo di altri articoli diventano link, anche negli articoli già pubblicati: una volta sola per articolo di destinazione, al massimo 3 link per articolo (si cambia nelle Impostazioni), mai nei titoletti. Se togli le parole chiave, i link spariscono.

**In evidenza.** Il redattore può spuntare *In evidenza in home*: l'articolo resta tra le notizie di apertura anche quando ne escono di più recenti.

**Pagine.** In *Pagine* trovi Chi siamo, Contatti, Privacy policy e Cookie policy, create come bozze: completale e pubblicale. Le pagine stanno nella sitemap ma non in home, feed e liste.

**Immagini.** Carica pure le foto originali: Presstatic crea versioni WebP a 480, 800, 1200 e 1600 pixel e una copia JPEG (o PNG per la grafica) fino a 2400 pixel, che usa per Google e per i social. Ogni lettore scarica solo la versione adatta al suo schermo. GIF e AVIF restano come sono.

**Ricerca.** La pagina `/cerca/` e l'icona della lente nella testata. L'indice si aggiorna da solo, al massimo ogni 2 minuti dopo una pubblicazione. Non c'è niente da installare.

**Integrazioni** (solo amministratori).

- *Google Indexing API*: nella pagina c'è la guida passo per passo (progetto su Google Cloud, API da abilitare, account di servizio, chiave JSON, permessi da Proprietario in Search Console). Poi incolli la chiave, spunti «Avvisa Google a ogni pubblicazione» e premi *Prova il collegamento*. Le modifiche agli articoli già pubblicati si inviano solo se attivi l'opzione apposita, per non consumare la quota di Google (circa 200 indirizzi al giorno). In fondo alla pagina c'è il registro degli ultimi invii. Google dichiara l'Indexing API per offerte di lavoro e dirette video: per le notizie non garantisce nulla.
- *Intelligenza artificiale*: chiavi API di Anthropic e OpenAI, modello predefinito per i testi e per le immagini, stile della testata. Modelli già pronti: Claude Opus 5.5, Fable 5.1, Sonnet 5 e Haiku 4.5; GPT-6 Astra, GPT-6 Sol, GPT-6 Luna e GPT-5.6 Terra; GPT Image 2 per le immagini. Quando esce un modello nuovo lo aggiungi nel campo *Altri modelli* (`openai:nome` o `anthropic:nome`), senza aggiornare Presstatic. I costi li addebita il fornitore sul tuo account. Per impostazione predefinita «Scrivi con l'IA» è riservata a redattori e amministratori (si può aprire anche agli autori), con al massimo 30 bozze al giorno per utente e due in contemporanea.

Chiavi e token non tornano mai nel browser: per cambiarli scrivi quelli nuovi, se lasci il campo vuoto restano quelli salvati.

**File** (solo amministratori). Un file manager per le cartelle `public` (il sito generato, con le immagini caricate) e `themes` (i temi personalizzati): carichi file, anche più di uno o uno ZIP da estrarre sul posto, crei cartelle, scarichi ed elimini. Non si può uscire da quelle due cartelle, e dagli ZIP vengono saltati i file con percorsi sospetti. Le pagine HTML del sito vengono rigenerate a ogni pubblicazione, quindi le modifiche a quei file durano fino alla pubblicazione successiva; per i contenuti tuoi usa Pagine.

**Aggiornamenti** (solo amministratori). Presstatic controlla ogni 12 ore le release del repository GitHub indicato nel file `REPO` e, se c'è una versione più recente, mostra un avviso nel menu con le note di rilascio. Con «Aggiorna ora» scarica il nuovo programma, ne verifica la firma con la chiave pubblica compilata dentro (file `PUBLIC_KEY`), lo prova, lo sostituisce e si riavvia: articoli, immagini, utenti e impostazioni restano al loro posto. La versione precedente resta in `bin/presstatic.old` per un eventuale ripristino. Un aggiornamento con firma diversa viene rifiutato: anche chi entrasse nel repository non potrebbe spingere codice ai siti.

**Impostazioni.** Tema, colore, articoli correlati (sì o no), link interni automatici, menu, piè di pagina, logo, icona, script e pubblicità. Quando salvi, Presstatic aggiorna subito tutte le pagine e svuota la cache di Cloudflare.

- **Piè di pagina**: nelle *Colonne di link* una riga che inizia con `#` apre una colonna e ne è il titolo; sotto, un link per riga (`Chi siamo | /chi-siamo/`, oppure il nome di una categoria). Nei *Profili social* basta un indirizzo per riga: il nome del social si riconosce da solo (per un nome diverso: `Newsletter | https://…`). Le *Note legali* vanno una per riga.
- **Menu**: una voce per riga, stessa sintassi dei link del piè di pagina.

## 6. Personalizzare

**Temi**: i due temi sono dentro il programma. Per modificarne uno crea accanto al database la cartella `themes/classico/` (o `themes/moderno/`) e copiaci solo il file da cambiare, preso da `templates/`. Per un tema nuovo crea `themes/nome-tema/`: compare da solo tra i temi nelle impostazioni e prende da Classico i file che non ha. Dopo aver modificato i file premi *Rigenera il sito*. I modelli usano la sintassi Jinja; `templates/seo.html` contiene l'intestazione comune a tutti i temi (SEO, social, script) e conviene includerlo anche nei temi nuovi, così ogni impostazione del pannello continua a funzionare. `page.html` (pagine statiche) e `search.html` (ricerca) sono comuni a tutti i temi.

Variabili disponibili nei modelli: `site` (tutte le impostazioni, per esempio `site.site_name`, `site.logo`, `site.accent`), `menu`, `footer` (`columns`, `social`, `legal`), `meta`, `canonical`, `theme`. Negli articoli anche `post`, `body`, `image_attrs`, `author`, `tags`, `related`, `reading`, `updated`, `facts`; nelle liste `posts` (ognuno con `img` per srcset, `time`, `author`), `heading`, `label`, `author`, `people`, `page`, `prev_url`, `next_url`, `notfound`; nella prima pagina della home anche `front` (`top`, `latest`, `sections`, `others`, `dateline`, `updated`). `templates/enhance.html` aggiunge a entrambi i temi la data di oggi e gli orari relativi («2 ore fa»).

**Nuovi tipi di schema**: copia `schemas.json` accanto al database e aggiungi un tipo. Ogni campo ha `key` (proprietà schema.org, anche annidata come `itemReviewed.name`), `label` (testo nel pannello), `kind` facoltativo (`lines`, `steps`, `minutes`, `number`, `url`) e `show` facoltativo (etichetta visibile nella pagina). Riavvia Presstatic.

## 7. Pubblicare Presstatic (per chi lo distribuisce)

**Due architetture.** Ogni release contiene `presstatic-linux-x86_64` (Intel, AMD) e `presstatic-linux-aarch64` (ARM64),
più `presstatic-src.tar.gz` per i processori senza programma pronto (l'installazione lo compila sul server), ognuno con la
sua firma `.sig`. Il modo più semplice: il flusso `.github/workflows/release.yml` compila tutto a ogni tag `vX.Y.Z` su
macchine Ubuntu 22.04 x86 e ARM di GitHub (gratuite per i repository pubblici) e, con il segreto
`PRESSTATIC_SIGNING_KEY` (il file `firma.key` in base64), lo firma e prepara la release in bozza. In alternativa, da un
computer x86: `RUSTC_BOOTSTRAP=1 cargo build --release --target aarch64-unknown-linux-gnu -Zbuild-std=std,panic_abort`
con `gcc-aarch64-linux-gnu` come linker.

Una volta sola:

1. Crea un repository pubblico su GitHub (per esempio `tuonome/presstatic`) e scrivi `tuonome/presstatic` nel file `REPO`.
2. Crea la coppia di chiavi per firmare le versioni: `target/release/presstatic keygen ~/.presstatic-signing.key`. La chiave privata resta solo sul tuo computer (fanne una copia al sicuro: senza, non potrai più pubblicare aggiornamenti che i siti accettano). Il comando stampa la chiave pubblica in due formati: incolla il primo nel file `PUBLIC_KEY` e il blocco PEM nella variabile `PUBKEY_PEM` di `deploy/install.sh`; sostituisci anche `UTENTE/presstatic` in `REPO` dentro lo script e nel README.
3. Installa la CLI di GitHub (`gh`) e autenticati con `gh auth login`.

Per ogni versione:

```sh
deploy/release.sh 1.0.1 "Cosa cambia in questa versione"
```

Lo script aggiorna il numero di versione, compila, firma il programma, crea il tag e la release su GitHub con i due file `presstatic-linux-x86_64` e `presstatic-linux-x86_64.sig`. Entro 12 ore tutti i siti installati mostrano «Aggiornamento disponibile»; con «Controlla ora» subito. Il comando di installazione per i nuovi siti è quello del punto 2.

## 8. Backup

Bastano due cose: il database e la cartella `public/media/`. Il database va copiato con `sqlite3 presstatic.db "VACUUM INTO '/percorso/backup.db'"`, anche mentre il sito è in uso: fa una copia coerente in un colpo solo (nelle prove, circa 250 MB in meno di un secondo). Non copiare il file a mano mentre il servizio è acceso, perché l'ultima parte dei dati può trovarsi ancora nel file `presstatic.db-wal`. Tutto il resto, compreso l'indice di ricerca, si ricrea con *Rigenera il sito*.

## 9. Misurare le prestazioni del database

`python3 tests/benchmark-sqlite.py` prova SQLite con le stesse query di Presstatic, su database di prova (quello del sito non viene mai toccato): latenza di ogni operazione, crescita con l'archivio, curva di carico con il punto di saturazione, scritture continue, velocità del disco, arresto brusco e backup a caldo. Alla fine salva un rapporto con i grafici in `benchmark-sqlite.html`. `--rapido` dura circa 3 minuti; `--dimensioni 10000,100000,1000000` prova archivi più grandi; `--reale /var/www/presstatic/presstatic.db` misura anche una copia del database vero.

## Limiti di questa versione

- Non ci sono commenti, newsletter né una libreria dei media: ogni immagine si carica dove serve.
- Le sessioni del pannello sono in memoria: dopo un riavvio bisogna rifare l'accesso.
- Nessuna notifica email: il redattore vede gli articoli da rivedere quando apre il pannello.
- La scrittura con l'IA e la Google Indexing API sono state provate con servizi simulati che rispondono come quelli veri: al primo uso con le chiavi reali conviene fare una prova.
- I link interni si aggiungono solo agli articoli che contengono le parole chiave; per applicarli a tutto l'archivio dopo averli attivati, premi *Rigenera il sito*.
- Gli articoli correlati di una pagina si aggiornano quando l'articolo viene salvato di nuovo o quando rigeneri il sito.
- Ogni sitemap mensile può contenere fino a 50.000 articoli, il limite di Google.
- Prima di vendere Presstatic ad altri serve un audit di sicurezza esterno e una procedura per segnalare e correggere le vulnerabilità (obblighi del Cyber Resilience Act per il software distribuito).

## Componenti di terze parti

- Editor visuale: [Quill](https://quilljs.com) 2.0.3, licenza BSD-3-Clause (`assets/editor/LICENSE-quill.txt`).
- Font: Newsreader (Production Type) e Schibsted Grotesk (Schibsted), licenza SIL Open Font License 1.1 (`assets/fonts/`).
- Immagini: le librerie Rust `image` (MIT/Apache-2.0) e `webp` con libwebp (BSD).
- Pulizia dell'HTML degli autori: `ammonia` (MIT/Apache-2.0).
- Ricerca: [Pagefind](https://pagefind.app), licenza MIT, incluso come libreria.
- Link interni: `aho-corasick` (MIT/Unlicense). Firma dei token Google: `ring` (licenze ISC/MIT/OpenSSL).

## Sicurezza: cose da sapere

- **Pannello chiuso e nascosto.** Commenti, newsletter e notifiche arrivano dal dominio del sito (`www`), non dal pannello. Dopo l'installazione metti la nuvola arancione anche su `admin`: così l'IP del server non compare nel DNS. Meglio ancora, proteggi `admin` con Cloudflare Access (gratuito fino a 50 utenti) o con un elenco di IP della redazione.
- **Verifica in due passaggi.** In Utenti puoi renderla **obbligatoria per redattori e amministratori**: chi non l'ha attivata viene portato ad attivarla prima di usare il pannello. È consigliato, perché redattori e amministratori possono inserire HTML libero nel sito. Dopo 10 codici sbagliati in 15 minuti il secondo passaggio si blocca per quell'account e il titolare riceve un'email: vuol dire che qualcuno conosce la sua password.
- **Chiave di firma degli aggiornamenti.** `presstatic keygen` stampa anche l'**impronta** della chiave: pubblicala fuori da GitHub (sul sito di Presstatic), perché `install.sh` e la chiave arrivano dallo stesso repository e chi violasse GitHub potrebbe sostituirli entrambi. La stessa impronta compare durante l'installazione, nella pagina Aggiornamenti del pannello e con `presstatic impronta`. Chi installa può farla controllare in automatico: `curl -fsSL …/install.sh | sudo PRESSTATIC_IMPRONTA="xxxx xxxx …" bash` si ferma se non coincide.
- **Aggiornamento automatico.** Il programma può sostituire sé stesso (è così che si aggiorna): chi riuscisse a eseguire codice come utente `presstatic` potrebbe quindi modificarlo. È il compromesso dell'aggiornamento dal pannello; il servizio resta comunque chiuso in `ProtectSystem=strict` senza privilegi di root.
- **Script del pannello.** Ogni pagina del pannello ha una CSP con «nonce»: il browser esegue solo gli script del pannello stesso, quindi un eventuale script infilato da un attaccante non partirebbe.
- **Redattori e HTML.** Redattori e amministratori possono inserire HTML e script negli articoli, come in WordPress: un account di redazione rubato può quindi modificare il sito. Gli autori invece hanno l'HTML ripulito.
- **Backup.** Gli archivi contengono il database con tutti i segreti (chiavi dei servizi, password SMTP, segreti della verifica in due passaggi). Sul server sono leggibili solo dal servizio; su S3 usa un bucket privato con una chiave dedicata, e attiva la cifratura lato server del provider.
- **Variabili `PRESSTATIC_*` di prova** (servizi finti, chiave di firma di test): servono solo ai test automatici e vengono ignorate quando il programma gira come servizio systemd.

