//! IndexNow: avvisa Bing, Yandex, Seznam, Naver e gli altri motori che aderiscono al protocollo quando un articolo esce,
//! cambia o viene tolto. Nessun account: il sito dimostra di essere il proprietario con un file di chiave alla radice
//! (per esempio https://www.miosito.it/<chiave>.txt). Google non aderisce: per Google c'è l'Indexing API.
use crate::{now, opt, site, App};
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::params;
use serde_json::json;
use std::time::Duration;

pub fn on(st: &crate::Settings) -> bool { st.get("indexnow_on").is_some_and(|v| v == "on") }

/// Chiave del sito (32 caratteri esadecimali): si crea la prima volta che IndexNow viene attivato.
pub fn ensure_key(app: &App) -> String {
    let st = app.settings();
    let k = opt(&st, "indexnow_key", "");
    if k.len() >= 8 { return k.to_string() }
    let mut b = [0u8; 16];
    SystemRandom::new().fill(&mut b).expect("generatore casuale");
    let key: String = b.iter().map(|x| format!("{x:02x}")).collect();
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('indexnow_key', ?1)", [&key]);
    key
}

/// Il file di chiave alla radice del sito: c'è solo con IndexNow attivo.
pub fn write_key_file(app: &App) {
    let st = app.settings();
    let key = opt(&st, "indexnow_key", "");
    if key.len() < 8 { return }
    let f = app.public.join(format!("{key}.txt"));
    if on(&st) { let _ = std::fs::write(f, key); } else { let _ = std::fs::remove_file(f); }
}

fn endpoint() -> String { crate::test_env("PRESSTATIC_INDEXNOW_API").unwrap_or_else(|| "https://api.indexnow.org/indexnow".into()) }

/// Invia gli indirizzi (nuovi, modificati o tolti). Restituisce una nota per il messaggio della redazione.
pub fn submit(app: &App, urls: &[String]) -> String {
    let st = app.settings();
    if urls.is_empty() || !on(&st) { return String::new() }
    let key = opt(&st, "indexnow_key", "").to_string();
    let base = site::base(&st).to_string();
    let host = base.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("").to_string();
    let list: Vec<&String> = urls.iter().filter(|u| u.starts_with(&base)).take(10_000).collect();
    if key.len() < 8 || host.is_empty() || list.is_empty() { return String::new() }
    let body = json!({"host": host, "key": key, "keyLocation": format!("{base}/{key}.txt"), "urlList": list});
    let r = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build().post(&endpoint())
        .set("Content-Type", "application/json; charset=utf-8").send_string(&body.to_string());
    let (ok, note) = match r {
        Ok(_) => (true, "inviato".to_string()),
        Err(ureq::Error::Status(c, _)) => (false, match c {
            400 => "richiesta non valida".into(),
            403 => "chiave non riconosciuta: il file della chiave non è raggiungibile sul sito (controlla l'indirizzo del sito nelle Impostazioni)".into(),
            422 => "gli indirizzi non appartengono al dominio del sito".into(),
            429 => "troppi invii in poco tempo: riprova più tardi".into(),
            c => format!("errore {c}"),
        }),
        Err(e) => (false, format!("servizio non raggiungibile ({e})")),
    };
    {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        for u in &list { let _ = db.execute("INSERT INTO index_log(url, kind, ok, note, at) VALUES (?1, 'IndexNow', ?2, ?3, ?4)", params![u, ok, note, now()]); }
    }
    if ok { format!(" IndexNow avvisato ({} {}).", list.len(), if list.len() == 1 { "indirizzo" } else { "indirizzi" }) } else { format!(" IndexNow non avvisato: {note}.") }
}
