# Confronto di carico: WordPress e Presstatic

Stessi contenuti (2.000 articoli da circa 5 KB in 6 categorie), stesso server, cinque configurazioni:

| Porta | Configurazione |
|---|---|
| 8201 | WordPress + Nginx + PHP-FPM, senza cache |
| 8202 | WordPress + Nginx con cache di pagina (fastcgi_cache) |
| 8203 | WordPress + Apache (MPM event) + PHP-FPM, senza cache |
| 8204 | Presstatic, pagine statiche servite da Nginx |
| 8205 | Presstatic, pagine statiche servite da Apache |

## Preparazione (Ubuntu 24.04)

1. `apt install nginx apache2 php-fpm php-mysql php-xml php-mbstring php-curl php-gd php-intl mariadb-server wrk`
2. WordPress in `/var/www/wp` (per esempio dal mirror ufficiale su GitHub), database `wp` su MariaDB, poi con WP-CLI:
   `wp core install --url=http://wp.local ...` e `wp rewrite structure '/%postname%/'`.
3. Articoli di WordPress: `wp eval-file articoli-wordpress.php`.
4. Sito Presstatic in `/tmp/bench-ps`: `presstatic adduser`, poi `python3 articoli-presstatic.py /tmp/bench-ps 2000`, avvio e *Rigenera il sito*.
5. Configurazioni: `nginx.conf` in `/etc/nginx/sites-enabled/`, `apache.conf` in `/etc/apache2/sites-enabled/` (con `Listen 8203` e `Listen 8205`, moduli `mpm_event proxy_fcgi rewrite`, `a2enconf php8.3-fpm`) e il `.htaccess` standard di WordPress.

Nota: sul server con la cache di pagina serve `tcp_nopush off;`. Con l'impostazione predefinita di Ubuntu (`tcp_nopush on`) le risposte dalla cache subiscono un ritardo di circa 40 ms per richiesta e WordPress risulterebbe ingiustamente lento.

## Prova

    python3 carico.py 50 10s home,articolo risultati.jsonl              # due modalità: come un browser e senza compressione
    python3 carico.py 10,200 8s home risultati.jsonl                    # home, 10 e 200 connessioni
    python3 carico.py 50 10s home risultati.jsonl "browser (br, gzip)"  # una modalità sola

«Come un browser» chiede le pagine accettando brotli e gzip, come fanno tutti i browser e Cloudflare: è la prova realistica.
«Senza compressione» chiede la pagina intera, come nelle prime misure. `wrk` gira con una sola thread: su una macchina
con pochi core rende di più, perché ruba meno processore al server. Nginx è regolato come in `install.sh` (connessioni
riusate, cache dei file, `index.html` cercato per primo, registro delle visite spento) e Apache allo stesso modo
(`apache-regolazioni.conf`). Il sito Presstatic su Apache usa `deploy/apache-sito.conf`, che invia le pagine già compresse.

Tra una prova su WordPress senza cache e la successiva, `riavvia-php.sh` svuota la coda di PHP-FPM: altrimenti le richieste rimaste in attesa continuerebbero a occupare il processore durante la prova seguente.

`risultati-regolati.jsonl` contiene le ultime misure (server regolati, registro spento); `risultati-server-di-prova.jsonl` quelle della prima serie. Entrambe sul server di prova (1 core, 4 GB, rete interna, senza HTTPS e senza Cloudflare): contano i rapporti tra le configurazioni, non i valori assoluti.

## Compressione spenta, gzip e brotli

    python3 carico-compressione.py 50 10s home,articolo risultati.jsonl "spenta|gzip|brotli"
    python3 carico-compressione.py 10,200 8s home risultati.jsonl "spenta|gzip|brotli"

Per WordPress servono gzip e brotli al volo: in Nginx il modulo `libnginx-mod-http-brotli-filter` (`brotli on;`), in Apache
`mod_brotli` e `mod_deflate` (`AddOutputFilterByType BROTLI_COMPRESS;DEFLATE text/html ...`). Presstatic invia le versioni
già compresse. I risultati sul server di prova sono in `risultati-compressione.jsonl`.

## WordPress con Varnish

Varnish 7.1 davanti a WordPress (porta 8210, backend Nginx senza cache sulla 8201), configurato come in `varnish.vcl`:
utenti collegati e pannello esclusi, pagine HTML compresse in gzip una volta sola quando entrano in cache, durata un'ora.

    varnishd -a 127.0.0.1:8210 -f varnish.vcl -s malloc,512m -n /tmp/varnish
    python3 carico-varnish.py risultati.jsonl caldo         # pagine in cache, home e articolo, gzip e browser
    python3 carico-varnish.py risultati.jsonl connessioni   # home con 10 e 200 connessioni
    python3 carico-varnish.py risultati.jsonl freddo        # archivio: 2.000 articoli diversi (archivio-freddo.lua)

Le prove dell'archivio vanno fatte a processore libero, una alla volta: dopo una prova su WordPress, PHP e Varnish
continuano per qualche secondo a smaltire le richieste rimaste in coda. I risultati sono in `risultati-varnish.jsonl`.
