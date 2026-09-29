//! Newsletter: iscrizione dal sito con doppia conferma (GDPR), disiscrizione con un clic, riepilogo automatico dei nuovi
//! articoli (ogni giorno o ogni settimana, all'ora scelta) inviato con il server SMTP configurato in Integrazioni.
//! Sul sito c'è solo un modulo HTML (niente JavaScript), e solo se la newsletter è attiva.
use crate::{mail, now, opt, site, App, R, Settings};
use rusqlite::params;
use serde::Serialize;
use std::sync::{LazyLock, Mutex};

pub fn on(st: &Settings) -> bool { st.get("newsletter_on").is_some_and(|v| v == "on") }

#[derive(Default, Clone, Serialize)]
pub struct Sending { pub running: bool, pub total: usize, pub sent: usize, pub failed: usize, pub last: String }
pub static SENDING: LazyLock<Mutex<Sending>> = LazyLock::new(Default::default);

/// Modulo di iscrizione in fondo agli articoli: HTML e un po' di CSS, niente JavaScript.
pub fn form_html(st: &Settings) -> String {
    if !on(st) || !mail::configured(st) { return String::new() }
    let action = site::esc(&format!("{}/newsletter/iscriviti", site::base(st)));
    let privacy = site::esc(&format!("{}/privacy/", site::base(st)));
    let title = site::esc(opt(st, "newsletter_title", "La newsletter"));
    let pitch = site::esc(opt(st, "newsletter_pitch", "Le notizie più importanti, direttamente nella tua casella. Gratis, ti cancelli quando vuoi."));
    format!("<aside class=\"nl\" data-pagefind-ignore><style>.nl{{margin:2.5rem 0;padding:1.4rem 1.5rem;border-radius:10px;background:var(--soft,#f3f5f9);border:1px solid var(--line,#e3e6ee)}}\
.nl,.nl h2{{color:var(--ink,#1d2433)}}.nl a{{color:inherit;text-decoration:underline}}.nl h2{{margin:0 0 .3rem;font-size:1.2rem}}.nl p{{margin:0 0 .9rem}}.nl form{{display:flex;flex-wrap:wrap;gap:.6rem}}.nl input[type=email]{{flex:1;min-width:12rem;font:inherit;padding:.65rem .8rem;border:1px solid var(--line,#ccc);border-radius:6px}}\
.nl button{{font:inherit;font-weight:700;padding:.65rem 1.1rem;border:0;border-radius:6px;background:var(--accent,#1e6bff);color:#fff;cursor:pointer}}.nl label.nl-c{{flex-basis:100%;display:flex;gap:.5rem;font-size:.85rem;align-items:flex-start}}\
.nl .nl-hp{{position:absolute;left:-9999px}}</style><h2>{title}</h2><p>{pitch}</p>\
<form method=\"post\" action=\"{action}\"><input type=\"email\" name=\"email\" required maxlength=\"254\" placeholder=\"La tua email\" aria-label=\"La tua email\" autocomplete=\"email\">\
<input class=\"nl-hp\" name=\"website\" tabindex=\"-1\" autocomplete=\"off\" aria-hidden=\"true\"><button type=\"submit\">Iscriviti</button>\
<label class=\"nl-c\"><input type=\"checkbox\" name=\"consent\" required><span>Ho letto l'<a href=\"{privacy}\">informativa sulla privacy</a>: la mia email serve solo per la newsletter.</span></label></form></aside>")
}

/// Pagina semplice per le risposte al lettore (iscrizione, conferma, disiscrizione).
pub fn page(st: &Settings, title: &str, text: &str) -> String {
    let site = site::esc(opt(st, "site_name", "Il sito"));
    format!("<!doctype html><html lang=\"it\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"robots\" content=\"noindex\"><title>{} | {site}</title>\
<body style=\"font-family:system-ui,sans-serif;max-width:34rem;margin:4rem auto;padding:0 1.2rem;line-height:1.55;color:#1d2433\"><p style=\"font-weight:700;color:#67728c\">{site}</p><h1 style=\"font-size:1.6rem\">{}</h1><p>{}</p>\
<p><a href=\"{}/\" style=\"font-weight:700\">Torna al sito</a></p></body></html>", site::esc(title), site::esc(title), site::esc(text), site::esc(&site::base(st)))
}

/// Iscrizione: resta «in attesa» finché il lettore non conferma dal link nell'email (doppia conferma).
pub fn subscribe(app: &App, email: &str, consent: bool, trap: &str, ip: &str) -> R<()> {
    let st = app.settings();
    if !on(&st) { return Err("la newsletter non è attiva".into()) }
    if !trap.is_empty() { return Ok(()) } // campo trappola: un programma di spam, si fa finta di niente
    let email = email.trim().to_lowercase();
    // Solo i caratteri degli indirizzi reali: niente | ! ( ' = e simili, che servirebbero solo a costruire formule o comandi.
    let valid = |e: &str| -> bool {
        let Some((local, domain)) = e.split_once('@') else { return false };
        !local.is_empty() && local.len() <= 64 && !local.starts_with(['.', '-', '+']) && !domain.contains('@')
            && local.chars().all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c))
            && domain.contains('.') && !domain.starts_with(['.', '-']) && !domain.ends_with(['.', '-'])
            && domain.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
    };
    if email.len() > 254 || !valid(&email) { return Err("l'indirizzo email non sembra valido".into()) }
    if !consent { return Err("per iscriverti devi accettare l'informativa sulla privacy".into()) }
    let who = crate::ip_hash(app, ip);
    let token = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let recent: i64 = db.query_row("SELECT COUNT(*) FROM subscribers WHERE ip_hash = ?1 AND created_at > ?2", params![who, now() - 600], |r| r.get(0)).unwrap_or(0);
        if recent >= 5 { return Err("troppe iscrizioni da questa connessione: riprova tra qualche minuto".into()) }
        // Tetto complessivo: niente raffiche di email di conferma da tanti indirizzi diversi (reputazione del server di posta).
        let hour: i64 = db.query_row("SELECT COUNT(*) FROM subscribers WHERE status = 'pending' AND created_at > ?1", [now() - 3600], |r| r.get(0)).unwrap_or(0);
        if hour >= 200 { return Err("in questo momento arrivano troppe iscrizioni: riprova tra un po'".into()) }
        match db.query_row("SELECT status, token FROM subscribers WHERE email = ?1", [&email], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
            Ok((status, _)) if status == "confirmed" => return Ok(()), // già iscritto: nessuna email in più (e nessun indizio su chi è iscritto)
            // Stesso indirizzo in attesa di conferma: al massimo un'email ogni 12 ore, da qualunque IP arrivi la richiesta.
            // Chiunque può scrivere un indirizzo altrui nel modulo: così non si può inondare di conferme una casella.
            Ok((status, _)) if status == "pending" && db.query_row("SELECT created_at FROM subscribers WHERE email = ?1", [&email], |r| r.get::<_, i64>(0)).unwrap_or(0) > now() - 12 * 3600 => return Ok(()),
            Ok((_, token)) => { db.execute("UPDATE subscribers SET status = 'pending', created_at = ?2, ip_hash = ?3 WHERE email = ?1", params![email, now(), who]).map_err(|e| e.to_string())?; token }
            Err(_) => {
                let token = crate::twofa::token();
                db.execute("INSERT INTO subscribers(email, status, token, ip_hash, created_at) VALUES (?1, 'pending', ?2, ?3, ?4)", params![email, token, who, now()]).map_err(|e| e.to_string())?;
                token
            }
        }
    };
    let link = format!("{}/newsletter/conferma/{token}", site::base(&st));
    let site_name = opt(&st, "site_name", "Il sito").to_string();
    let html = mail::layout(&site_name, "Conferma l'iscrizione", &[&format!("Grazie per esserti iscritto alla newsletter di {site_name}."), "Manca un passo: premi il pulsante per confermare che l'indirizzo è tuo."], Some(("Confermo l'iscrizione", &link)), "Se non hai chiesto tu l'iscrizione, ignora questa email: non riceverai niente.");
    let text = format!("Per confermare l'iscrizione alla newsletter di {site_name} apri questo link:\n{link}\n\nSe non l'hai chiesta tu, ignora questa email.");
    mail::send(&st, &email, &format!("Conferma l'iscrizione a {site_name}"), &text, &html)
}

pub fn confirm(app: &App, token: &str) -> R<()> {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let n = db.execute("UPDATE subscribers SET status = 'confirmed', confirmed_at = ?2 WHERE token = ?1 AND status IN ('pending', 'confirmed')", params![token, now()]).map_err(|e| e.to_string())?;
    if n == 0 { return Err("il link non è valido o l'iscrizione è stata annullata".into()) }
    Ok(())
}

pub fn unsubscribe(app: &App, token: &str) -> R<()> {
    let n = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("UPDATE subscribers SET status = 'unsubscribed' WHERE token = ?1", [token]).map_err(|e| e.to_string())?;
    if n == 0 { return Err("il link non è valido".into()) }
    Ok(())
}

/// Nuovi articoli dall'ultimo invio (al massimo 15), dal più recente.
fn digest_posts(app: &App, since: i64) -> Vec<site::Post> {
    site::query(&app.db.lock().unwrap_or_else(|e| e.into_inner()), false, "WHERE p.kind = 'post' AND p.status = 'published' AND p.published_at > ?1 AND p.published_at <= ?2 ORDER BY p.published_at DESC LIMIT 15", &[&since, &now()])
}

/// Invia il riepilogo dei nuovi articoli agli iscritti confermati, riusando la stessa connessione SMTP.
pub fn send_digest(app: &App) -> R<String> {
    let st = app.settings();
    if !on(&st) { return Err("la newsletter non è attiva".into()) }
    let since: i64 = opt(&st, "newsletter_last", "0").parse().unwrap_or(0).max(now() - 8 * 86_400);
    let posts = digest_posts(app, since);
    if posts.is_empty() { return Ok("Nessun articolo nuovo dall'ultimo invio: niente da mandare.".into()) }
    let subs: Vec<(String, String)> = app.db.lock().unwrap_or_else(|e| e.into_inner())
        .prepare("SELECT email, token FROM subscribers WHERE status = 'confirmed' ORDER BY id").and_then(|mut q| q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|r| r.filter_map(Result::ok).collect())).unwrap_or_default();
    if subs.is_empty() { return Ok("Non ci sono ancora iscritti confermati.".into()) }
    {
        let mut s = SENDING.lock().unwrap_or_else(|e| e.into_inner());
        if s.running { return Err("un invio è già in corso".into()) }
        *s = Sending { running: true, total: subs.len(), ..Default::default() };
    }
    let site_name = opt(&st, "site_name", "Il sito").to_string();
    let subject = format!("{}: {}", opt(&st, "newsletter_title", &site_name), posts[0].title);
    let esc = site::esc;
    let cards: String = posts.iter().map(|p| {
        let url = site::post_url(&st, &p.slug);
        let img = if p.image.is_empty() { String::new() } else { format!("<a href=\"{}\"><img src=\"{}\" alt=\"\" width=\"512\" style=\"display:block;width:100%;height:auto;border-radius:10px;margin:0 0 10px\"></a>", esc(&url), esc(&format!("{}{}", site::base(&st), p.image))) };
        format!("<div style=\"margin:0 0 26px\">{img}<p style=\"margin:0 0 4px;font-size:12px;font-weight:700;color:#1e6bff;text-transform:uppercase\">{}</p><h2 style=\"margin:0 0 6px;font-size:19px;line-height:1.3\"><a href=\"{}\" style=\"color:#0b1638;text-decoration:none\">{}</a></h2><p style=\"margin:0;font-size:15px;line-height:1.55;color:#33415c\">{}</p></div>",
            esc(&p.category), esc(&url), esc(&p.title), esc(&p.description))
    }).collect();
    let text_list: String = posts.iter().map(|p| format!("- {}\n  {}\n", p.title, site::post_url(&st, &p.slug))).collect();
    let sender = match mail::sender(&st) { Ok(s) => s, Err(e) => { SENDING.lock().unwrap_or_else(|e| e.into_inner()).running = false; return Err(e) } };
    let (mut sent, mut failed, mut first_err) = (0, 0, String::new());
    for (email, token) in &subs {
        let unsub = format!("{}/newsletter/disiscrivi/{token}", site::base(&st));
        let html = format!("<!doctype html><html lang=\"it\"><body style=\"margin:0;background:#f4f6fb;font-family:Arial,Helvetica,sans-serif\"><div style=\"max-width:600px;margin:0 auto;padding:26px 16px\">\
<div style=\"background:#0a1233;border-radius:16px 16px 0 0;padding:18px 24px;color:#fff;font-weight:700;font-size:18px\">{}</div><div style=\"background:#fff;border:1px solid #e3e8f2;border-top:0;border-radius:0 0 16px 16px;padding:26px 24px\">{cards}</div>\
<p style=\"font-size:12px;color:#67728c;text-align:center;line-height:1.5;margin:18px 0 0\">Ricevi questa email perché ti sei iscritto alla newsletter di {}.<br><a href=\"{}\" style=\"color:#67728c\">Annulla l'iscrizione</a></p></div></body></html>",
            esc(&site_name), esc(&site_name), esc(&unsub));
        let text = format!("{site_name}: le ultime notizie\n\n{text_list}\nPer annullare l'iscrizione: {unsub}\n");
        match sender.send(email, &subject, &text, &html, Some(&unsub)) {
            Ok(()) => sent += 1,
            Err(e) => { failed += 1; if first_err.is_empty() { first_err = e } }
        }
        let mut s = SENDING.lock().unwrap_or_else(|e| e.into_inner());
        s.sent = sent; s.failed = failed;
    }
    {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let _ = db.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('newsletter_last', ?1)", [posts[0].published_at.to_string()]);
        let _ = db.execute("INSERT INTO newsletter_sends(subject, posts, sent, failed, created_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![subject, posts.len() as i64, sent, failed, now()]);
    }
    let msg = format!("Newsletter inviata a {sent} iscritti con {} articoli{}.", posts.len(), if failed > 0 { format!("; {failed} invii non riusciti ({first_err})") } else { String::new() });
    { let mut s = SENDING.lock().unwrap_or_else(|e| e.into_inner()); s.running = false; s.last = msg.clone(); }
    Ok(msg)
}

/// È l'ora dell'invio automatico? Ogni giorno, oppure solo il lunedì, all'ora scelta (fuso orario del sito).
pub fn due(st: &Settings) -> bool {
    if !on(st) || opt(st, "newsletter_every", "daily") == "off" { return false }
    let tz = site::tz(st);
    let t = jiff::Timestamp::from_second(now()).unwrap_or(jiff::Timestamp::UNIX_EPOCH).to_zoned(tz.clone());
    let hour: i8 = opt(st, "newsletter_hour", "7").parse().unwrap_or(7);
    if t.hour() < hour { return false }
    if opt(st, "newsletter_every", "daily") == "weekly" && t.weekday() != jiff::civil::Weekday::Monday { return false }
    let today = t.strftime("%Y-%m-%d").to_string();
    opt(st, "newsletter_day", "") != today
}
