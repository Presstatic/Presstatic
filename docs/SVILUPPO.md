# Documentazione tecnica

Per installare e usare Presstatic basta la [guida d'uso](GUIDA.md). Questa pagina è per chi vuole compilarlo,
personalizzarlo a fondo, contribuire o pubblicarne una versione.

**Come funziona.** Presstatic è un solo programma scritto in Rust, con il database SQLite in un file. Quando la
redazione pubblica, genera le pagine del sito come file HTML (già compressi) che Nginx consegna ai lettori, con
Cloudflare davanti se c'è. Il programma serve il pannello su `admin.dominio` e riceve solo commenti, iscrizioni e moduli
dal sito: chi legge non tocca mai il database.

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
   curl -fsSL https://raw.githubusercontent.com/Presstatic/Presstatic/main/deploy/install.sh | sudo bash -s -- miosito.it tua@email.it
   ```

   `Presstatic/Presstatic` è il repository GitHub di chi distribuisce Presstatic (vedi «Pubblicare una nuova versione»). Lo script scarica l'ultima versione dalle release, ne verifica la firma, controlla il DNS, installa Nginx e Certbot, crea l'utente e il servizio, configura il sito su `www` e il pannello su `admin`, e ottiene il certificato HTTPS gratuito di Let's Encrypt. In alternativa, se hai già i file sul server, entra nella cartella con `presstatic` e `install.sh` e lancia `sudo bash install.sh miosito.it tua@email.it`.
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

## 7. Pubblicare una nuova versione

Le release si compilano da sole con GitHub Actions (`.github/workflows/release.yml`) a ogni tag `vX.Y.Z`: il programma
per x86_64 e per ARM64 (aarch64) su macchine Ubuntu 22.04 vere, il codice sorgente `presstatic-src.tar.gz` per i
processori senza programma pronto, e la firma `.sig` di ogni file con la chiave nel segreto `PRESSTATIC_SIGNING_KEY`.
Il risultato è una release in bozza da controllare e pubblicare.

**Una volta sola:**

1. `presstatic keygen firma.key` su una macchina Linux: stampa la chiave pubblica (per il file `PUBLIC_KEY`),
   l'impronta (da pubblicare sul sito di Presstatic) e la stessa chiave in PEM (per `PUBKEY_PEM` in `deploy/install.sh`).
2. `REPO` e la riga `REPO=` di `deploy/install.sh` con il repository delle release (qui `Presstatic/Presstatic`).
3. Il segreto `PRESSTATIC_SIGNING_KEY` nel repository: il file `firma.key` in base64. La chiave privata resta solo lì e
   in una copia sicura: se si perde, i siti installati non accettano più aggiornamenti.

**A ogni versione:**

1. Il numero di versione in `Cargo.toml` e, per lo stesso pacchetto, in `Cargo.lock` (la compilazione usa `--locked`).
2. Commit, poi `git tag vX.Y.Z` e `git push origin vX.Y.Z`: il tag deve coincidere con la versione del programma.
3. Quando le tre fasi sono verdi, controlla nella bozza i sei file (due programmi, il sorgente e le loro firme) e
   pubblica. I siti vedono l'aggiornamento entro 12 ore (o con «Controlla ora») e lo installano da *Aggiornamenti*
   dopo aver verificato la firma.

Compilare ARM64 da un computer x86, senza GitHub: `RUSTC_BOOTSTRAP=1 cargo build --release --target
aarch64-unknown-linux-gnu -Zbuild-std=std,panic_abort` con `gcc-aarch64-linux-gnu` come linker e i sorgenti della
libreria standard di Rust.

## 8. Backup

Bastano due cose: il database e la cartella `public/media/`. Il database va copiato con `sqlite3 presstatic.db "VACUUM INTO '/percorso/backup.db'"`, anche mentre il sito è in uso: fa una copia coerente in un colpo solo (nelle prove, circa 250 MB in meno di un secondo). Non copiare il file a mano mentre il servizio è acceso, perché l'ultima parte dei dati può trovarsi ancora nel file `presstatic.db-wal`. Tutto il resto, compreso l'indice di ricerca, si ricrea con *Rigenera il sito*.

## 9. Misurare le prestazioni del database

`python3 tests/benchmark-sqlite.py` prova SQLite con le stesse query di Presstatic, su database di prova (quello del sito non viene mai toccato): latenza di ogni operazione, crescita con l'archivio, curva di carico con il punto di saturazione, scritture continue, velocità del disco, arresto brusco e backup a caldo. Alla fine salva un rapporto con i grafici in `benchmark-sqlite.html`. `--rapido` dura circa 3 minuti; `--dimensioni 10000,100000,1000000` prova archivi più grandi; `--reale /var/www/presstatic/presstatic.db` misura anche una copia del database vero.

## Limiti noti

- Sistemi supportati dall'installazione: Ubuntu 22.04 e successive, Debian 12 e successive, processori x86_64 e ARM64
  (glibc 2.34 o più recente). Le distribuzioni della famiglia Red Hat non sono ancora supportate dallo script.
- Con Apache la configurazione (`deploy/apache-sito.conf`) va adattata a mano; l'installazione automatica usa Nginx.
- Le sessioni del pannello sono in memoria: dopo un riavvio del programma bisogna rifare l'accesso.
- La scrittura con l'IA e la Google Indexing API sono state provate con servizi simulati che rispondono come quelli
  veri: al primo uso con le chiavi reali conviene fare una prova.
- I link interni automatici si aggiungono agli articoli salvati dopo averli attivati; per applicarli a tutto l'archivio
  premi *Rigenera il sito*. Lo stesso vale per gli articoli correlati.
- Ogni sitemap mensile può contenere fino a 50.000 articoli, il limite di Google.

## Componenti di terze parti

- Editor visuale: [Quill](https://quilljs.com) 2.0.3, licenza BSD-3-Clause (`assets/editor/LICENSE-quill.txt`).
- Font: Newsreader (Production Type) e Schibsted Grotesk (Schibsted), licenza SIL Open Font License 1.1 (`assets/fonts/`).
- Immagini: le librerie Rust `image` (MIT/Apache-2.0) e `webp` con libwebp (BSD).
- Pulizia dell'HTML degli autori: `ammonia` (MIT/Apache-2.0).
- Ricerca: [Pagefind](https://pagefind.app), licenza MIT, incluso come libreria.
- Link interni: `aho-corasick` (MIT/Unlicense). Firma dei token Google: `ring` (licenze ISC/MIT/OpenSSL).
