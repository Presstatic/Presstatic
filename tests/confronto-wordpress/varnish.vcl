vcl 4.1;
# WordPress dietro Varnish, configurato bene: pagine in memoria, compresse in gzip una volta sola quando entrano in cache.
backend default { .host = "127.0.0.1"; .port = "8201"; }
sub vcl_recv {
    # pannello, accesso e utenti collegati non passano dalla cache
    if (req.url ~ "^/wp-(admin|login)" || req.http.Cookie ~ "wordpress_logged_in|comment_author") { return (pass); }
    unset req.http.Cookie;
}
sub vcl_backend_response {
    if (beresp.http.Content-Type ~ "text/html|text/css|javascript|xml") { set beresp.do_gzip = true; }
    unset beresp.http.Set-Cookie;
    set beresp.ttl = 1h;
}
