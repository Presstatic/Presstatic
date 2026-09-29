use crate::{opt, Settings};
use serde_json::{json, Value};

const RULE_NAME: &str = "Presstatic: cache HTML";
// Il piano Free di Cloudflare non accetta una Edge TTL sotto le 2 ore.
// Non è un problema: a ogni pubblicazione Presstatic svuota subito le pagine cambiate.
const EDGE_TTL: u32 = 7200;

fn configured(st: &Settings) -> bool { !opt(st, "cf_zone", "").is_empty() && !opt(st, "cf_token", "").is_empty() }

fn api(st: &Settings, method: &str, path: &str, body: Option<Value>) -> Result<Value, (u16, String)> {
    let root = crate::test_env("PRESSTATIC_CF_API").unwrap_or_else(|| "https://api.cloudflare.com/client/v4".into());
    let url = format!("{root}/zones/{}{path}", opt(st, "cf_zone", ""));
    let req = crate::api_agent(20).request(method, &url)
        .set("Authorization", &format!("Bearer {}", opt(st, "cf_token", "")))
        .timeout(std::time::Duration::from_secs(20));
    let res = match body { Some(b) => req.send_json(b), None => req.call() };
    match res {
        Ok(r) => r.into_json().map_err(|e| (0, e.to_string())),
        Err(ureq::Error::Status(code, r)) => {
            let v: Value = r.into_json().unwrap_or_default();
            let msgs: Vec<String> = v["errors"].as_array().into_iter().flatten().filter_map(|e| e["message"].as_str().map(String::from)).collect();
            Err((code, msgs.join("; ")))
        }
        Err(e) => Err((0, e.to_string())),
    }
}

/// Svuota la cache per gli indirizzi indicati (30 per chiamata, il limite dei piani non Enterprise).
pub fn purge(st: &Settings, urls: &[String]) -> String {
    if !configured(st) || urls.is_empty() { return String::new() }
    let mut u = urls.to_vec();
    u.sort();
    u.dedup();
    for chunk in u.chunks(30) {
        if let Err((c, e)) = api(st, "POST", "/purge_cache", Some(json!({ "files": chunk }))) {
            return format!(" Cloudflare: svuotamento della cache non riuscito (errore {c}: {e}).");
        }
    }
    format!(" Cache Cloudflare svuotata per {} indirizzi.", u.len())
}

pub fn purge_all(st: &Settings) -> String {
    if !configured(st) { return String::new() }
    match api(st, "POST", "/purge_cache", Some(json!({ "purge_everything": true }))) {
        Ok(_) => " Cache Cloudflare svuotata completamente.".into(),
        Err((c, e)) => format!(" Cloudflare: svuotamento della cache non riuscito (errore {c}: {e})."),
    }
}

/// Attiva Tiered Cache (topologia Smart) e crea o aggiorna la regola che mette in cache l'HTML.
pub fn setup(st: &Settings) -> Result<String, String> {
    if !configured(st) { return Err("inserisci Zone ID e token API, poi salva le impostazioni".into()) }
    let host = opt(st, "base_url", "").split("://").nth(1).unwrap_or_default().split('/').next().unwrap_or_default().to_lowercase();
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Err("l'indirizzo del sito deve essere un dominio, per esempio https://www.miosito.it".into());
    }
    let e = |(c, m): (u16, String)| format!("errore {c}: {m}");
    api(st, "PATCH", "/argo/tiered_caching", Some(json!({ "value": "on" }))).map_err(e)?;
    api(st, "PATCH", "/cache/tiered_cache_smart_topology_enable", Some(json!({ "value": "on" }))).map_err(e)?;

    let rule = json!({
        "description": RULE_NAME,
        "expression": format!("(http.host eq \"{host}\")"),
        "action": "set_cache_settings",
        "action_parameters": { "cache": true, "edge_ttl": { "mode": "override_origin", "default": EDGE_TTL } },
        "enabled": true,
    });
    match api(st, "GET", "/rulesets/phases/http_request_cache_settings/entrypoint", None) {
        Ok(v) => {
            let rs = v["result"]["id"].as_str().unwrap_or_default();
            let existing = v["result"]["rules"].as_array().into_iter().flatten().find(|r| r["description"] == RULE_NAME);
            match existing.and_then(|r| r["id"].as_str()) {
                Some(id) => api(st, "PATCH", &format!("/rulesets/{rs}/rules/{id}"), Some(rule)),
                None => api(st, "POST", &format!("/rulesets/{rs}/rules"), Some(rule)),
            }
            .map_err(e)?;
        }
        Err((404, _)) => {
            let rs = json!({ "name": "default", "kind": "zone", "phase": "http_request_cache_settings", "rules": [rule] });
            api(st, "POST", "/rulesets", Some(rs)).map_err(e)?;
        }
        Err(x) => return Err(e(x)),
    }
    Ok(format!("Cloudflare configurato: Tiered Cache attivo e HTML di {host} in cache."))
}
