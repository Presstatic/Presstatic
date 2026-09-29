# Guida di Presstatic

Presstatic è un CMS per giornali online. La redazione scrive nel pannello; a ogni pubblicazione Presstatic prepara le
pagine del sito come file HTML già pronti, che il server (e Cloudflare, se lo usi) consegna ai lettori senza calcoli.
Le pagine restano velocissime anche con molti lettori insieme, e il database non è mai sulla strada di chi legge.

Tutte le funzioni sono incluse e gratuite: non ci sono versioni Pro né plugin da comprare. Paghi solo quello che scegli
tu fuori da Presstatic: il server e, se la usi, la chiave dell'intelligenza artificiale.

**Indice:** [Cosa serve](#cosa-serve) · [Installazione](#installazione) · [Primi passi](#primi-passi) ·
[Uso quotidiano](#uso-quotidiano) · [Domande frequenti](#domande-frequenti)

---

## Cosa serve

- **Un server Linux** (una VPS o un server dedicato vanno benissimo): Ubuntu 22.04 o 24.04, oppure Debian 12 o 13, con
  accesso SSH. Processore **x86_64** (Intel, AMD) oppure **ARM64** (Ampere, AWS Graviton, Hetzner CAX, Oracle Ampere):
  l'installazione riconosce il processore e scarica il programma giusto. Per altri processori compila Presstatic sul
  server dal codice sorgente firmato (qualche minuto in più, una volta sola).
- **Un dominio**, per esempio `miosito.it`.
- **Cloudflare gratuito**, consigliato: nasconde l'indirizzo del server e consegna le pagine da tutto il mondo.

Non serve un database da creare: i dati stanno in un file dentro la cartella del sito.

## Installazione

### 1. Crea tre record DNS

Nel pannello DNS del dominio (su Cloudflare o dal tuo registrar) crea tre record **A** verso l'indirizzo IP del server:

| Tipo | Nome | Valore |
|---|---|---|
| A | `miosito.it` (o `@`) | IP del server |
| A | `www` | IP del server |
| A | `admin` | IP del server |

Se usi Cloudflare, **lascia per ora la nuvola grigia** («Solo DNS») su tutti e tre: durante l'installazione Let's
Encrypt deve raggiungere direttamente il server per rilasciare il certificato HTTPS.

### 2. Entra nel server

Con SSH dal tuo computer (su Windows funziona anche dalla PowerShell), oppure con la console web del provider:

```
ssh utente@IP-DEL-SERVER
```

### 3. Lancia l'installazione

Con un solo comando, che scarica Presstatic, ne verifica la firma e prepara tutto:

```
curl -fsSL https://raw.githubusercontent.com/[UTENTE]/presstatic/main/deploy/install.sh | sudo bash -s -- miosito.it tua@email.it
```

Oppure, se hai lo zip `presstatic-linux-x86_64.zip` sul tuo computer, copialo sul server ed eseguilo da lì
(da Windows, nella PowerShell):

```
scp "$env:USERPROFILE\Downloads\presstatic-linux-x86_64.zip" utente@IP-DEL-SERVER:~/
ssh -t utente@IP-DEL-SERVER "python3 -m zipfile -e presstatic-linux-x86_64.zip . && chmod +x presstatic-linux-x86_64/presstatic && sudo bash presstatic-linux-x86_64/install.sh miosito.it tua@email.it"
```

L'email serve a Let's Encrypt per gli avvisi sui certificati. Durante l'installazione compare anche l'**impronta della
chiave** con cui è firmato Presstatic: se vuoi, confrontala con quella pubblicata sul sito del progetto.

### 4. Apri il link e completa l'installazione guidata

Alla fine lo script stampa un link come `https://admin.miosito.it/admin/setup?token=…`. Aprilo nel browser: scegli
nome e indirizzo del sito, il tuo account, il tema, il colore, il logo e le sezioni del giornale. Premi **«Installa e
metti online il sito»**: Presstatic genera le pagine e ti porta nel pannello, già collegato.

### 5. Accendi Cloudflare (se lo usi)

Metti la **nuvola arancione** su tutti e tre i record, imposta SSL/TLS su **«Full (strict)»**, poi sul server:

```
sudo bash /var/www/presstatic/solo-cloudflare.sh
```

Da quel momento il sito accetta solo il traffico che passa da Cloudflare: chi scopre l'indirizzo del server non può
aggirarlo.

### Più siti sulla stessa VPS

Lancia di nuovo l'installazione con un altro dominio: Presstatic crea un secondo sito indipendente, con la sua
cartella (`/var/www/presstatic-altrosito-it`), il suo database e il suo pannello. Lanciarla con un dominio già
installato, invece, lo **aggiorna** lasciando intatti articoli, utenti e impostazioni.

### Aggiornare

Dal pannello: **Aggiornamenti › Aggiorna ora**. Presstatic scarica la nuova versione, controlla la firma e si riavvia.
In alternativa, rilancia l'installazione con lo stesso dominio.

---

## Primi passi

1. **Sezioni e menu.** Le sezioni scelte nell'installazione guidata (Cronaca, Politica…) sono già il menu, le
   categorie e i blocchi della home. Per aggiungerne o toglierne: **Impostazioni › Menu**. Descrizioni e sottosezioni:
   **Categorie**.
2. **Il primo articolo.** **Nuovo articolo**: titolo, sommario, testo, foto, sezione. L'anteprima su Google ti avvisa
   se titolo e sommario sono troppo lunghi. Puoi pubblicare subito o programmare l'uscita.
3. **L'aspetto.** Due temi inclusi, **Classico** (magazine luminoso) e **Moderno** (magazine digitale con un colore
   per ogni sezione), entrambi con menu a hamburger sul telefono e tema scuro automatico. Tema, colore principale e
   logo si cambiano in **Impostazioni**.
4. **La home su misura.** **Costruisci il sito**: componi home, testata, menu, piè di pagina e pagine trascinando
   sezioni e widget, con l'anteprima vera del tuo tema su computer, tablet e telefono. Il sito cambia solo quando
   premi **Pubblica**.
5. **La redazione.** **Utenti**: aggiungi redattori (possono pubblicare e inserire codice HTML) e autori (scrivono, la
   redazione pubblica). Un account si può disattivare senza cancellarlo. Consigliato: rendi obbligatoria la **verifica in due passaggi** per redattori e
   amministratori.
6. **Email.** In **Integrazioni** collega un server di posta (SMTP): serve per recuperare la password, per la
   newsletter e per ricevere avvisi e messaggi dei moduli.

## Uso quotidiano

### Scrivere e pubblicare
- **Note interne** sugli articoli, con avviso all'autore; **azioni in blocco** su più articoli; se due persone aprono
  lo stesso articolo, la seconda viene avvisata e nessuna modifica va persa.
- **Cronologia** di ogni articolo, con il ripristino di una versione precedente.
- **Libreria media** con didascalie, crediti e ricerca; **gallerie** di foto negli articoli.
- **Scrivi con l'IA**: bozze, titoli e sommari con la tua chiave OpenAI o Anthropic.

### I lettori
- **Commenti**: moderati dalla redazione, con antispam, senza JavaScript per chi legge.
- **Newsletter**: iscrizione con doppia conferma, riepilogo automatico giornaliero o settimanale.
- **Notifiche push**: avvisi sul telefono o sul computer a ogni nuovo articolo, per un numero illimitato di iscritti,
  inviate dal tuo server senza servizi esterni.
- **Condivisione**: pulsanti con le icone ufficiali di WhatsApp, Facebook, X, Telegram, LinkedIn, Threads, Bluesky,
  Reddit, email e «Copia link». Si scelgono in **Impostazioni › Condivisione**.
- **Moduli**: contatti, segnalazioni, richieste. Crei i campi in **Moduli**, li inserisci con il widget «Modulo» del
  page builder o scrivendo `[modulo 3]` in una pagina; i messaggi arrivano nel pannello e per email.

### Cookie e consenso
In **Impostazioni › Cookie e consenso** scegli come raccogliere il consenso ai cookie:
- **Banner di Presstatic**: i codici esterni (Analytics, pixel, pubblicità) partono solo dopo il sì del lettore, per
  categoria. Senza servizi esterni.
- **Piattaforma esterna** (iubenda, Cookiebot o altre): incolli il loro script, che va per primo nella pagina, e attivi
  nella piattaforma il blocco automatico.
- **Google Consent Mode v2**, per Google Analytics e Google Ads.

Se usi la pubblicità di Google (AdSense, Ad Manager) nello Spazio economico europeo, Google richiede una piattaforma
certificata: scegli la modalità «piattaforma esterna».

### Pubblicità
In **Impostazioni › Pubblicità** incolli gli script della rete pubblicitaria e gli annunci per tre spazi: in alto
nell'articolo, nel testo (dopo il paragrafo che scegli) e in fondo. Il file `ads.txt` si scrive dallo stesso riquadro.

### Google e motori di ricerca
Sitemap, sitemap per Google News, dati strutturati e link canonici sono automatici. In **Indicizzazione rapida**
colleghi Google (Indexing API) e IndexNow (Bing e altri motori) per far conoscere subito ogni articolo nuovo.

### Backup e ripristino
In **Backup** imposti il backup automatico ogni giorno, sul server e su un archivio S3 (Wasabi, Backblaze, Hetzner,
Amazon S3, Cloudflare R2). Per tornare indietro: **Ripristina** accanto a una copia sul server, oppure carica un
archivio scaricato dall'S3. Prima di ogni ripristino Presstatic salva una copia dello stato attuale, quindi un
ripristino sbagliato si annulla allo stesso modo.

---

## Domande frequenti

**Serve un database come MySQL?**
No. Presstatic usa SQLite: un file nella cartella del sito, già incluso nei backup.

**Posso usare Apache invece di Nginx?**
L'installazione configura Nginx da sola. Con Apache c'è il file `apache-sito.conf` da adattare a mano: servono i moduli
`proxy` e `proxy_http`, e per un secondo sito sulla stessa VPS la porta va cambiata (8081, 8082…).

**Cloudflare è obbligatorio?**
No, ma è consigliato: nasconde l'indirizzo del server, consegna le pagine da tutto il mondo e regge i picchi di
traffico. Presstatic lo configura da solo quando inserisci il token in Impostazioni.

**Posso importare il mio sito WordPress?**
Sì: **Importa da WordPress**, con il file di esportazione (Strumenti › Esporta di WordPress). Arrivano articoli,
pagine, categorie, autori e immagini.

**Ho dimenticato la password.**
Nella pagina di accesso usa «Password dimenticata»: funziona se l'email (SMTP) è configurata. Un amministratore può
anche reimpostarla da **Utenti**.

**Ho perso il telefono con la verifica in due passaggi.**
Usa uno dei codici di recupero che hai salvato quando l'hai attivata. In alternativa, un altro amministratore può
azzerarla dalla tua scheda in **Utenti**.

**Il tema scuro si può spegnere?**
Oggi segue le impostazioni del dispositivo del lettore. Una pagina «Aspetto» con la scelta tra chiaro, scuro e
automatico è in preparazione.

**Posso modificare un tema o crearne uno?**
Sì: i temi sono modelli HTML con il loro stile. Le istruzioni sono nel README, sezione «Personalizzare».

**Dove stanno i miei dati sul server?**
Nella cartella del sito, di solito `/var/www/presstatic`: il database `presstatic.db`, le immagini in `public/media`,
le copie di backup in `backups`.

**Come segnalo un problema o chiedo una funzione?**
Apri una segnalazione nella pagina GitHub del progetto.
