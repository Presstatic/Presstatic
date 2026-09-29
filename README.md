# Presstatic

**Il CMS per giornali online che pubblica pagine HTML statiche.** Veloce come un sito statico, comodo come WordPress:
editor, page builder, newsletter, notifiche push e tutto il resto in un solo programma, gratuito e open source.

- **Velocissimo.** Le pagine sono file HTML già pronti: Lighthouse 100 in prestazioni, accessibilità, best practice e
  SEO, da telefono e da computer. Regge i picchi di traffico senza plugin di cache.
- **Sicuro.** Chi legge non tocca mai il database; il pannello sta su un indirizzo separato, con verifica in due
  passaggi. Gli aggiornamenti sono firmati e verificati prima di essere installati.
- **Tutto incluso.** Nessun plugin da comprare, nessuna versione Pro: paghi solo il server.
- **Un comando per installarlo** su Ubuntu o Debian, processori x86 o ARM, anche più siti sulla stessa VPS.

## Cosa c'è

**Redazione**
- Editor con anteprima su Google, cronologia delle versioni, articoli programmati, note interne e azioni in blocco
- Ruoli (amministratore, redattore, autore), lavoro in contemporanea senza sovrascritture, coautori
- Libreria media con didascalie e crediti, gallerie, immagini WebP ottimizzate e dati GPS rimossi
- **Dirette**: articoli che si aggiornano minuto per minuto, con i dati strutturati delle dirette
- Scrittura assistita con l'**intelligenza artificiale** (Anthropic o OpenAI, con la tua chiave)
- Importazione da WordPress: articoli, pagine, autori, categorie, tag e immagini

**Sito e design**
- Due temi, **Classico** e **Moderno**, con menu a hamburger sul telefono e modalità chiara, scura o automatica
- **Page builder** visuale per home, testata, menu, piè di pagina e pagine, con scrittura direttamente nell'anteprima
- Pagina *Aspetto* con anteprime dei temi, colore, logo e icona del sito

**Lettori**
- **Notifiche push** senza servizi esterni, **newsletter** con doppia conferma e riepiloghi automatici
- **Commenti** moderati e **moduli** personalizzabili (contatti, segnalazioni) con antispam senza captcha
- Condivisione con le icone ufficiali dei social, ricerca interna, «I più letti»

**SEO e crescita**
- Sitemap, sitemap per Google News, dati strutturati, canonical, Open Graph, feed RSS
- Indicizzazione rapida con Google Indexing API e IndexNow, link interni automatici
- Cloudflare integrato: cache delle pagine e svuotamento mirato a ogni pubblicazione

**Pubblicità e privacy**
- Spazi pubblicitari negli articoli, tra le notizie in home e nelle categorie, annuncio fisso sul telefono, `ads.txt`
- **Banner cookie** con blocco preventivo, oppure iubenda, Cookiebot e simili, con **Google Consent Mode v2**

**Gestione**
- Backup automatici sul server e su archivi S3, **ripristino con un clic** dal pannello
- Aggiornamenti firmati dal pannello, più siti sulla stessa VPS, ciascuno con il suo database

## Installazione

Serve un server Linux (VPS o dedicato) con Ubuntu 22.04/24.04 o Debian 12/13, processore x86_64 o ARM64, e un dominio.
Punta tre record DNS (il dominio, `www` e `admin`) all'IP del server, poi:

```
curl -fsSL https://raw.githubusercontent.com/Presstatic/Presstatic/main/deploy/install.sh | sudo bash -s -- miosito.it tua@email.it
```

Alla fine lo script stampa un link: aprilo e completa l'installazione guidata dal browser. Passo passo, con Cloudflare
e le domande frequenti: **[guida d'uso](docs/GUIDA.md)**.

## Aggiornamenti

Dal pannello, pagina *Aggiornamenti* › **Aggiorna ora**. Presstatic scarica la nuova versione, ne verifica la firma e
si riavvia. L'impronta della chiave di firma è nelle [note di ogni release](https://github.com/Presstatic/Presstatic/releases).

## Documentazione

- [Guida d'uso](docs/GUIDA.md): installazione, primi passi, uso quotidiano, domande frequenti
- [Documentazione tecnica](docs/SVILUPPO.md): compilare, web server, Cloudflare, temi, pubblicare una versione
- [Sicurezza](SECURITY.md): come segnalare una vulnerabilità

## Licenza

Presstatic è software libero, distribuito con licenza [GPL-3.0](LICENSE): puoi usarlo gratis anche per siti
commerciali e modificarlo; chi ne distribuisce una versione deve lasciarla aperta con la stessa licenza. I componenti
di terze parti (caratteri, icone, editor, librerie) hanno le loro licenze, elencate nella
[documentazione tecnica](docs/SVILUPPO.md#componenti-di-terze-parti).
