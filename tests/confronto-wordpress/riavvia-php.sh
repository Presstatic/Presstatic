#!/bin/bash
# Riavvia PHP-FPM svuotando la coda delle richieste rimaste, e aspetta che WordPress risponda di nuovo.
# Si cerca il processo per nome esatto (php-fpm8.3), mai per riga di comando.
pkill -x php-fpm8.3 2>/dev/null
for i in $(seq 1 50); do pgrep -x php-fpm8.3 >/dev/null || break; sleep 0.1; done
pkill -9 -x php-fpm8.3 2>/dev/null
mkdir -p /run/php; rm -f /run/php/php8.3-fpm.sock /run/php/php8.3-fpm.pid
php-fpm8.3 -D -y /etc/php/8.3/fpm/php-fpm.conf
for i in $(seq 1 50); do [ -S /run/php/php8.3-fpm.sock ] && break; sleep 0.1; done
for u in / /articolo-1000/; do for p in 8201 8203; do curl -s -o /dev/null -w "%{http_code} " -H 'Host: wp.local' http://127.0.0.1:$p$u; done; done
echo
