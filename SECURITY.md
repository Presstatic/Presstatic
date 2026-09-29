# Sicurezza

## Segnalare una vulnerabilità

Se trovi una falla, **non aprire una segnalazione pubblica**. Usa la segnalazione privata di GitHub: scheda
**Security** del repository › **Report a vulnerability**. Rispondiamo il prima possibile, correggiamo e pubblichiamo
una versione firmata; chi ha segnalato viene citato nelle note di rilascio, se lo desidera.

Riceve correzioni di sicurezza l'ultima versione pubblicata: i siti si aggiornano dal pannello (*Aggiornamenti*).

## Verificare quello che installi

Ogni release è firmata con una chiave Ed25519. L'installazione e gli aggiornamenti dal pannello controllano la firma
e si fermano se non corrisponde. L'impronta della chiave è nelle note di ogni release e sul sito di Presstatic:
confrontala con quella che mostra l'installazione.

## Cose da sapere

- **Pannello chiuso e nascosto.** Commenti, newsletter e notifiche arrivano dal dominio del sito (`www`), non dal pannello. Dopo l'installazione metti la nuvola arancione anche su `admin`: così l'IP del server non compare nel DNS. Meglio ancora, proteggi `admin` con Cloudflare Access (gratuito fino a 50 utenti) o con un elenco di IP della redazione.
- **Verifica in due passaggi.** In Utenti puoi renderla **obbligatoria per redattori e amministratori**: chi non l'ha attivata viene portato ad attivarla prima di usare il pannello. È consigliato, perché redattori e amministratori possono inserire HTML libero nel sito. Dopo 10 codici sbagliati in 15 minuti il secondo passaggio si blocca per quell'account e il titolare riceve un'email: vuol dire che qualcuno conosce la sua password.
- **Chiave di firma degli aggiornamenti.** `presstatic keygen` stampa anche l'**impronta** della chiave: pubblicala fuori da GitHub (sul sito di Presstatic), perché `install.sh` e la chiave arrivano dallo stesso repository e chi violasse GitHub potrebbe sostituirli entrambi. La stessa impronta compare durante l'installazione, nella pagina Aggiornamenti del pannello e con `presstatic impronta`. Chi installa può farla controllare in automatico: `curl -fsSL …/install.sh | sudo PRESSTATIC_IMPRONTA="xxxx xxxx …" bash` si ferma se non coincide.
- **Aggiornamento automatico.** Il programma può sostituire sé stesso (è così che si aggiorna): chi riuscisse a eseguire codice come utente `presstatic` potrebbe quindi modificarlo. È il compromesso dell'aggiornamento dal pannello; il servizio resta comunque chiuso in `ProtectSystem=strict` senza privilegi di root.
- **Script del pannello.** Ogni pagina del pannello ha una CSP con «nonce»: il browser esegue solo gli script del pannello stesso, quindi un eventuale script infilato da un attaccante non partirebbe.
- **Redattori e HTML.** Redattori e amministratori possono inserire HTML e script negli articoli, come in WordPress: un account di redazione rubato può quindi modificare il sito. Gli autori invece hanno l'HTML ripulito.
- **Backup.** Gli archivi contengono il database con tutti i segreti (chiavi dei servizi, password SMTP, segreti della verifica in due passaggi). Sul server sono leggibili solo dal servizio; su S3 usa un bucket privato con una chiave dedicata, e attiva la cifratura lato server del provider.
- **Variabili `PRESSTATIC_*` di prova** (servizi finti, chiave di firma di test): servono solo ai test automatici e vengono ignorate quando il programma gira come servizio systemd.
