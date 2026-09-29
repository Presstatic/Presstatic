// Google Indexing API: avvisa Google appena un articolo esce, cambia o viene tolto.
// Autenticazione con un service account: un JWT firmato RS256 si scambia con un token valido un'ora.
use crate::{now, opt, s, App, R};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use rusqlite::params;
use serde_json::{json, Value};

const SCOPE: &str = "https://www.googleapis.com/auth/indexing";

fn api() -> String { crate::test_env("PRESSTATIC_GOOGLE_API").unwrap_or_else(|| "https://indexing.googleapis.com".into()) }

fn key(app: &App) -> R<Value> {
    let st = app.settings();
    let raw = opt(&st, "google_sa", "");
    if raw.is_empty() { return Err("manca la chiave JSON del service account".into()) }
    let v: Value = serde_json::from_str(raw).map_err(|_| "la chiave incollata non è un file JSON valido".to_string())?;
    if v["client_email"].as_str().is_none() || v["private_key"].as_str().is_none() { return Err("nel file JSON mancano client_email o private_key".into()) }
    Ok(v)
}

/// Token di accesso per l'indicizzazione, riusato finché è valido.
fn token(app: &App, sa: &Value) -> R<String> { token_for(app, sa, SCOPE) }

/// Token per Google Analytics (sola lettura), con una sua cache: vale per un altro permesso.
static GA_TOKEN: std::sync::Mutex<Option<(String, i64)>> = std::sync::Mutex::new(None);
const GA_SCOPE: &str = "https://www.googleapis.com/auth/analytics.readonly";

fn token_for(app: &App, sa: &Value, scope: &str) -> R<String> {
    let slot = |f: &mut dyn FnMut(&mut Option<(String, i64)>)| if scope == SCOPE { f(&mut app.google_token.lock().unwrap_or_else(|e| e.into_inner())) } else { f(&mut GA_TOKEN.lock().unwrap_or_else(|e| e.into_inner())) };
    let mut cached = None;
    slot(&mut |c| cached = c.clone().filter(|(_, exp)| *exp > now() + 60));
    if let Some((t, _)) = cached { return Ok(t) }
    let pem = sa["private_key"].as_str().unwrap_or_default();
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    let der = base64::engine::general_purpose::STANDARD.decode(body.trim()).map_err(|_| "chiave privata illeggibile")?;
    let pair = ring::signature::RsaKeyPair::from_pkcs8(&der).map_err(|_| "chiave privata non valida")?;
    const GOOGLE_TOKEN: &str = "https://oauth2.googleapis.com/token";
    let aud = sa["token_uri"].as_str().unwrap_or(GOOGLE_TOKEN);
    // La chiave firmata va solo a Google (un indirizzo diverso è ammesso solo nei test, con PRESSTATIC_GOOGLE_API).
    if aud != GOOGLE_TOKEN && crate::test_env("PRESSTATIC_GOOGLE_API").is_none() { return Err(format!("token_uri non valido nella chiave: deve essere {GOOGLE_TOKEN}")) }
    let t = now();
    let claims = json!({"iss": sa["client_email"], "scope": scope, "aud": aud, "iat": t, "exp": t + 3600});
    let unsigned = format!("{}.{}", B64.encode(r#"{"alg":"RS256","typ":"JWT"}"#), B64.encode(claims.to_string()));
    let mut sig = vec![0; pair.public().modulus_len()];
    pair.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), unsigned.as_bytes(), &mut sig).map_err(|_| "firma non riuscita")?;
    let jwt = format!("{unsigned}.{}", B64.encode(sig));
    let res = crate::api_agent(10).post(aud).timeout(std::time::Duration::from_secs(10))
        .send_form(&[("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", &jwt)]);
    let v: Value = match res {
        Ok(r) => r.into_json().map_err(s)?,
        Err(ureq::Error::Status(_, r)) => { let e: Value = r.into_json().unwrap_or_default(); return Err(format!("Google ha rifiutato la chiave: {}", e["error_description"].as_str().or(e["error"].as_str()).unwrap_or("errore sconosciuto"))) }
        Err(e) => return Err(e.to_string()),
    };
    let tok = v["access_token"].as_str().ok_or("Google non ha restituito un token")?.to_string();
    let exp = t + v["expires_in"].as_i64().unwrap_or(3600);
    slot(&mut |c| *c = Some((tok.clone(), exp)));
    Ok(tok)
}

fn call(req: ureq::Request, body: Option<Value>) -> Result<Value, (u16, String)> {
    let req = req.timeout(std::time::Duration::from_secs(10));
    match match body { Some(b) => req.send_json(b), None => req.call() } {
        Ok(r) => Ok(r.into_json().unwrap_or_default()),
        Err(ureq::Error::Status(c, r)) => { let e: Value = r.into_json().unwrap_or_default(); Err((c, e["error"]["message"].as_str().unwrap_or("").to_string())) }
        Err(e) => Err((0, e.to_string())),
    }
}

fn explain(code: u16, msg: &str) -> String {
    match code {
        403 => "Google ha negato l'accesso: aggiungi l'email del service account come Proprietario in Search Console, e controlla che la Web Search Indexing API sia attiva nel progetto".into(),
        429 => "quota giornaliera esaurita (di solito 200 indirizzi al giorno)".into(),
        _ => format!("errore {code} {msg}"),
    }
}

/// Invia a Google gli indirizzi (true = rimosso). Restituisce una frase per il messaggio al redattore.
pub fn notify(app: &App, urls: &[(String, bool)]) -> String {
    let st = app.settings();
    if urls.is_empty() || opt(&st, "google_on", "") != "on" { return String::new() }
    let result = key(app).and_then(|sa| token(app, &sa)).and_then(|tok| {
        let mut sent = 0;
        for (url, deleted) in urls {
            let kind = if *deleted { "URL_DELETED" } else { "URL_UPDATED" };
            let r = call(crate::api_agent(10).post(&format!("{}/v3/urlNotifications:publish", api())).set("Authorization", &format!("Bearer {tok}")), Some(json!({"url": url, "type": kind})));
            let (ok, note) = match &r { Ok(_) => (true, "inviato".to_string()), Err((c, m)) => (false, explain(*c, m)) };
            log(app, url, kind, ok, &note);
            if let Err((c, m)) = r { return Err(explain(c, &m)) }
            sent += 1;
        }
        Ok(sent)
    });
    match result {
        Ok(n) => format!(" Google avvisato ({n} {}).", if n == 1 { "indirizzo" } else { "indirizzi" }),
        Err(e) => { log(app, "", "ERRORE", false, &e); format!(" Google non avvisato: {e}.") }
    }
}

/// Prova della configurazione: chiede a Google lo stato della home.
pub fn test(app: &App) -> R<String> {
    let sa = key(app)?;
    let tok = token(app, &sa)?;
    let home = format!("{}/", crate::site::base(&app.settings()));
    let url = format!("{}/v3/urlNotifications/metadata?url={}", api(), urlencode(&home));
    match call(crate::api_agent(10).get(&url).set("Authorization", &format!("Bearer {tok}")), None) {
        Ok(_) | Err((404, _)) => Ok(format!("Collegamento riuscito: il service account {} può usare l'Indexing API per {home}.", sa["client_email"].as_str().unwrap_or(""))),
        Err((c, m)) => Err(explain(c, &m)),
    }
}

fn urlencode(v: &str) -> String {
    v.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn log(app: &App, url: &str, kind: &str, ok: bool, note: &str) {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let _ = db.execute("INSERT INTO index_log(url, kind, ok, note, at) VALUES (?1, ?2, ?3, ?4, ?5)", params![url, kind, ok, note, now()]);
    let _ = db.execute("DELETE FROM index_log WHERE id NOT IN (SELECT id FROM index_log ORDER BY id DESC LIMIT 200)", []);
}

/// Pagine più viste negli ultimi giorni secondo Google Analytics 4 (Data API): (percorso, visualizzazioni).
/// Il service account va aggiunto alla proprietà GA4 con il ruolo Visualizzatore.
pub fn most_read(app: &App, st: &crate::Settings) -> R<Vec<(String, i64)>> {
    let property: String = crate::opt(st, "ga_property", "").chars().filter(char::is_ascii_digit).collect();
    if property.is_empty() { return Err("manca l'ID della proprietà di Google Analytics 4 (solo numeri, in Amministrazione > Dettagli proprietà)".into()) }
    let sa: Value = serde_json::from_str(crate::opt(st, "google_sa", "")).map_err(|_| "manca la chiave del service account di Google (qui sopra)".to_string())?;
    let tok = token_for(app, &sa, GA_SCOPE)?;
    let days = if crate::opt(st, "most_read_days", "1") == "7" { "7daysAgo" } else { "1daysAgo" };
    let base = crate::test_env("PRESSTATIC_GA_API").unwrap_or_else(|| "https://analyticsdata.googleapis.com".into());
    let body = json!({
        "dateRanges": [{"startDate": days, "endDate": "today"}], "dimensions": [{"name": "pagePath"}], "metrics": [{"name": "screenPageViews"}],
        "orderBys": [{"metric": {"metricName": "screenPageViews"}, "desc": true}], "limit": 100,
    });
    let v = call(crate::api_agent(15).post(&format!("{base}/v1beta/properties/{property}:runReport")).set("Authorization", &format!("Bearer {tok}")), Some(body))
        .map_err(|(c, m)| if c == 403 { "Google Analytics ha negato l'accesso: aggiungi l'email del service account alla proprietà GA4 come Visualizzatore, e attiva la Google Analytics Data API nel progetto".to_string() } else { format!("Google Analytics: errore {c} {m}") })?;
    Ok(v["rows"].as_array().into_iter().flatten().filter_map(|r| Some((r["dimensionValues"][0]["value"].as_str()?.to_string(), r["metricValues"][0]["value"].as_str()?.parse().ok()?))).collect())
}
