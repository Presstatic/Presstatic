//! Moduli: la redazione li compone nel pannello (campi, email di destinazione, consenso facoltativo) e li inserisce
//! nel sito con il widget «Modulo» del page builder o scrivendo [modulo N] nel testo di una pagina.
//! Nel sito sono HTML semplice: nessuna libreria, nessun servizio esterno. Il programma entra in gioco solo all'invio,
//! passando dal dominio del sito come i commenti. Antispam senza captcha: campo trappola, tempo minimo di
//! compilazione (se il browser lo misura), limite di invii per impronta dell'indirizzo, niente doppioni.

use crate::site::{self, esc};
use crate::{now, opt, App, Settings, R};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const TYPES: &[(&str, &str)] = &[("text", "Testo breve"), ("email", "Email"), ("tel", "Telefono"), ("textarea", "Testo lungo"), ("select", "Scelta da un elenco"), ("checkbox", "Casella da spuntare")];

#[derive(Clone)]
pub struct Form { pub id: i64, pub name: String, pub fields: Vec<Value>, pub notify: String, pub success: String, pub button: String, pub consent: bool, pub consent_text: String, pub privacy_url: String }

fn s(e: impl ToString) -> String { e.to_string() }
fn row(r: &rusqlite::Row) -> rusqlite::Result<Form> {
    Ok(Form { id: r.get(0)?, name: r.get(1)?, fields: serde_json::from_str::<Vec<Value>>(&r.get::<_, String>(2)?).unwrap_or_default(), notify: r.get(3)?,
        success: r.get(4)?, button: r.get(5)?, consent: r.get::<_, i64>(6)? == 1, consent_text: r.get(7)?, privacy_url: r.get(8)? })
}
const COLS: &str = "id, name, fields, notify, success, button, consent, consent_text, privacy_url";

pub fn get(db: &Connection, id: i64) -> Option<Form> { db.query_row(&format!("SELECT {COLS} FROM forms WHERE id = ?1"), [id], row).ok() }
pub fn all(db: &Connection) -> Vec<Form> {
    db.prepare(&format!("SELECT {COLS} FROM forms ORDER BY id")).and_then(|mut q| q.query_map([], row).map(|r| r.filter_map(Result::ok).collect())).unwrap_or_default()
}

/// Nome sicuro di un campo: lettere, numeri e trattini, al massimo 24 caratteri.
fn fid(f: &Value) -> String { f["id"].as_str().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(24).collect() }

/// L'HTML del modulo nel sito, con il suo stile (nei colori del tema) e il messaggio di conferma che compare senza
/// JavaScript: dopo l'invio l'indirizzo finisce con #modulo-N-inviato e la regola :target lo mostra.
pub fn html(st: &Settings, f: &Form) -> String {
    let mut fields = String::new();
    for x in &f.fields {
        let (id, label, kind, req) = (fid(x), esc(x["label"].as_str().unwrap_or("")), x["type"].as_str().unwrap_or("text"), x["required"].as_bool().unwrap_or(false));
        if id.is_empty() || label.is_empty() { continue }
        let (r, star) = if req { (" required", "<b aria-hidden=\"true\">*</b>") } else { ("", "") };
        let name = format!("f_{id}");
        fields += &match kind {
            "textarea" => format!("<label class=\"ps-f\"><span>{label}{star}</span><textarea name=\"{name}\" rows=\"5\" maxlength=\"5000\"{r}></textarea></label>"),
            "select" => {
                let opts: String = x["options"].as_str().unwrap_or("").lines().map(str::trim).filter(|o| !o.is_empty()).map(|o| format!("<option>{}</option>", esc(o))).collect();
                format!("<label class=\"ps-f\"><span>{label}{star}</span><select name=\"{name}\"{r}><option value=\"\">Scegli…</option>{opts}</select></label>")
            }
            "checkbox" => format!("<label class=\"ps-c\"><input type=\"checkbox\" name=\"{name}\" value=\"sì\"{r}><span>{label}{star}</span></label>"),
            k => {
                let (t, ac) = match k { "email" => ("email", " autocomplete=\"email\""), "tel" => ("tel", " autocomplete=\"tel\""), _ => ("text", "") };
                format!("<label class=\"ps-f\"><span>{label}{star}</span><input type=\"{t}\" name=\"{name}\" maxlength=\"300\"{ac}{r}></label>")
            }
        };
    }
    let consent = if f.consent {
        let text = if f.consent_text.trim().is_empty() { "Ho letto l'informativa sulla privacy e acconsento al trattamento dei dati".to_string() } else { f.consent_text.clone() };
        let link = if f.privacy_url.trim().is_empty() { String::new() } else { format!(" <a href=\"{}\" target=\"_blank\" rel=\"noopener\">Informativa</a>", esc(f.privacy_url.trim())) };
        format!("<label class=\"ps-c\"><input type=\"checkbox\" name=\"consenso\" value=\"on\" required><span>{}{link}<b aria-hidden=\"true\">*</b></span></label>", esc(&text))
    } else { String::new() };
    let ok = if f.success.trim().is_empty() { "Grazie, il messaggio è arrivato alla redazione.".to_string() } else { f.success.clone() };
    let button = if f.button.trim().is_empty() { "Invia".to_string() } else { f.button.clone() };
    format!("<div class=\"ps-form\" id=\"modulo-{id}\" data-pagefind-ignore><style>\
.ps-form{{margin:1.6rem 0}}.ps-form form{{display:grid;gap:1rem}}.ps-f{{display:grid;gap:.35rem;font-weight:700;font-size:.95rem}}\
.ps-f b,.ps-c b{{color:var(--accent,#c8102e);margin-left:.15rem}}\
.ps-f input,.ps-f textarea,.ps-f select{{width:100%;box-sizing:border-box;padding:.75rem .85rem;border:1.5px solid var(--line,#d5d9e2);border-radius:10px;background:var(--bg,#fff);color:var(--ink,#111);font:inherit;font-weight:400}}\
.ps-f textarea{{resize:vertical;min-height:8rem}}.ps-f :focus{{outline:none;border-color:var(--accent,#1e6bff);box-shadow:0 0 0 3px color-mix(in srgb,var(--accent,#1e6bff) 22%,transparent)}}\
.ps-c{{display:flex;gap:.6rem;align-items:flex-start;font-size:.95rem}}.ps-c input{{width:1.15rem;height:1.15rem;margin-top:.15rem;flex:none;accent-color:var(--accent,#1e6bff)}}\
.ps-c a{{text-decoration:underline}}.ps-form button{{justify-self:start;padding:.8rem 1.6rem;border:0;border-radius:999px;background:var(--accent,#1e6bff);color:#fff;font:inherit;font-weight:800;cursor:pointer}}\
.ps-form button:hover{{filter:brightness(1.08)}}.ps-hp{{position:absolute;left:-9999px;width:1px;height:1px;overflow:hidden}}\
.ps-ok{{display:none;margin:0 0 1rem;padding:.9rem 1.1rem;border-radius:12px;background:#e7f6ec;color:#14532d;font-weight:700}}.ps-ok:target{{display:block}}\
</style><p class=\"ps-ok\" id=\"modulo-{id}-inviato\" role=\"status\">{ok}</p>\
<form method=\"post\" action=\"{base}/modulo/{id}\" accept-charset=\"utf-8\"><div class=\"ps-hp\" aria-hidden=\"true\"><label>Non compilare<input type=\"text\" name=\"website\" tabindex=\"-1\" autocomplete=\"off\"></label></div>\
<input type=\"hidden\" name=\"t\" value=\"\">{fields}{consent}<button type=\"submit\">{button}</button>\
<script>(()=>{{const f=document.currentScript.parentElement;let t0=0;f.addEventListener('focusin',()=>{{t0=t0||Date.now()}});f.addEventListener('submit',()=>{{f.t.value=t0?Date.now()-t0:''}})}})()</script></form></div>",
        id = f.id, base = site::base(st), ok = esc(&ok), button = esc(&button))
}

/// Tutti i moduli pronti da inserire (widget del builder e [modulo N] nel testo).
pub fn all_html(db: &Connection, st: &Settings) -> HashMap<i64, String> { all(db).iter().map(|f| (f.id, html(st, f))).collect() }

/// [modulo N] nel testo di una pagina o di un articolo diventa il modulo; un numero che non esiste sparisce.
pub fn shortcodes(body: &str, forms: &HashMap<i64, String>) -> String {
    if !body.contains("[modulo ") { return body.to_string() }
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(i) = rest.find("[modulo ") {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 8..];
        match tail.find(']') {
            Some(j) if tail[..j].trim().parse::<i64>().is_ok() => {
                let id: i64 = tail[..j].trim().parse().unwrap_or(0);
                out.push_str(forms.get(&id).map(String::as_str).unwrap_or(""));
                rest = &tail[j + 1..];
            }
            _ => { out.push_str("[modulo "); rest = tail; }
        }
    }
    out.push_str(rest);
    // un modulo da solo in un paragrafo: niente <p> vuoti intorno
    out.replace("<p><div class=\"ps-form\"", "<div class=\"ps-form\"").replace("</div></p>", "</div>")
}

pub struct Sent { pub form: String, pub to: String, pub reply_to: Option<String>, pub lines: Vec<(String, String)> }

/// Invio dal sito. Ok(None): accettato ma scartato in silenzio (trappola, troppo veloce, doppione).
pub fn submit(app: &App, id: i64, f: &HashMap<String, String>, ip: &str) -> R<Option<Sent>> {
    let form = { let db = app.db.lock().unwrap_or_else(|e| e.into_inner()); get(&db, id) }.ok_or("questo modulo non esiste più")?;
    let v = |k: &str| f.get(k).map(|x| x.trim().replace("\r\n", "\n")).unwrap_or_default();
    if !v("website").is_empty() { return Ok(None) } // campo trappola
    if let Ok(ms) = v("t").parse::<i64>() { if ms < 2500 { return Ok(None) } } // compilato in meno di 2,5 secondi
    let mut lines = vec![];
    let mut reply_to = None;
    for x in &form.fields {
        let (key, label, kind, req) = (fid(x), x["label"].as_str().unwrap_or("").to_string(), x["type"].as_str().unwrap_or("text"), x["required"].as_bool().unwrap_or(false));
        if key.is_empty() || label.is_empty() { continue }
        let val = v(&format!("f_{key}"));
        if req && val.is_empty() { return Err(format!("compila il campo «{label}»")) }
        let max = if kind == "textarea" { 5000 } else { 300 };
        if val.chars().count() > max { return Err(format!("il campo «{label}» è troppo lungo (al massimo {max} caratteri)")) }
        if kind == "email" && !val.is_empty() {
            if !val.contains('@') || val.contains(char::is_whitespace) || val.len() > 254 { return Err(format!("nel campo «{label}» scrivi un indirizzo email valido")) }
            reply_to.get_or_insert_with(|| val.to_lowercase());
        }
        if kind == "select" && !val.is_empty() && !x["options"].as_str().unwrap_or("").lines().any(|o| o.trim() == val) { return Err(format!("scegli una voce dell'elenco in «{label}»")) }
        lines.push((label, if kind == "checkbox" { if val.is_empty() { "no".into() } else { "sì".into() } } else { val }));
    }
    if form.consent && v("consenso") != "on" { return Err("per inviare il modulo devi dare il consenso al trattamento dei dati".into()) }
    if lines.iter().all(|l| l.1.is_empty() || l.1 == "no") { return Err("il modulo è vuoto".into()) }
    let data = serde_json::to_string(&lines.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>()).map_err(s)?;
    let who = crate::ip_hash(app, ip); // l'indirizzo IP non si salva, solo la sua impronta (per i limiti)
    let st = app.settings(); // prima di prendere il database: le impostazioni lo leggono a loro volta (niente blocco reciproco)
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    cleanup(&db, &st);
    let recent: i64 = db.query_row("SELECT COUNT(*) FROM form_entries WHERE ip_hash = ?1 AND created_at > ?2", params![who, now() - 600], |r| r.get(0)).unwrap_or(0);
    if recent >= 5 { return Err("hai inviato troppi messaggi in poco tempo: riprova tra qualche minuto".into()) }
    let dup = db.query_row("SELECT 1 FROM form_entries WHERE form_id = ?1 AND data = ?2 AND created_at > ?3", params![id, data, now() - 600], |_| Ok(())).is_ok();
    if dup { return Ok(None) }
    db.execute("INSERT INTO form_entries(form_id, data, ip_hash, created_at, seen) VALUES (?1, ?2, ?3, ?4, 0)", params![id, data, who, now()]).map_err(s)?;
    Ok(Some(Sent { form: form.name.clone(), to: form.notify.trim().to_string(), reply_to, lines }))
}

/// Cancellazione automatica dei messaggi più vecchi dei giorni scelti (0 = mai).
pub fn cleanup(db: &Connection, st: &Settings) {
    let days: i64 = opt(st, "forms_keep_days", "180").parse().unwrap_or(180);
    if days > 0 { let _ = db.execute("DELETE FROM form_entries WHERE created_at < ?1", [now() - days * 86_400]); }
}

/// Salvataggio dal pannello: i campi arrivano come JSON dall'editor; si tengono solo i tipi noti e i dati puliti.
pub fn save(db: &Connection, id: i64, f: &HashMap<String, String>) -> R<i64> {
    let v = |k: &str| f.get(k).map(|x| x.trim().to_string()).unwrap_or_default();
    let name = v("name");
    if name.is_empty() || name.chars().count() > 80 { return Err("dai un nome al modulo (al massimo 80 caratteri)".into()) }
    let notify = v("notify");
    if !notify.is_empty() && (!notify.contains('@') || notify.contains(char::is_whitespace)) { return Err("l'email di destinazione non è valida".into()) }
    let raw: Vec<Value> = serde_json::from_str(&v("fields")).unwrap_or_default();
    let mut fields = vec![];
    for (i, x) in raw.iter().enumerate().take(40) {
        let label: String = x["label"].as_str().unwrap_or("").trim().chars().take(120).collect();
        if label.is_empty() { continue }
        let kind = x["type"].as_str().filter(|t| TYPES.iter().any(|k| k.0 == *t)).unwrap_or("text");
        let id = { let k = fid(x); if k.is_empty() { format!("c{i}") } else { k } };
        let options: String = x["options"].as_str().unwrap_or("").lines().map(str::trim).filter(|o| !o.is_empty()).take(50).map(|o| o.chars().take(120).collect::<String>()).collect::<Vec<_>>().join("\n");
        fields.push(json!({"id": id, "label": label, "type": kind, "required": x["required"].as_bool().unwrap_or(false), "options": options}));
    }
    if fields.is_empty() { return Err("aggiungi almeno un campo".into()) }
    let fields = serde_json::to_string(&fields).map_err(s)?;
    let consent = if v("consent") == "on" { 1 } else { 0 };
    let p = params![name, fields, notify, v("success"), v("button"), consent, v("consent_text"), v("privacy_url")];
    if id == 0 {
        db.execute("INSERT INTO forms(name, fields, notify, success, button, consent, consent_text, privacy_url, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%s','now'))", p).map_err(s)?;
        Ok(db.last_insert_rowid())
    } else {
        db.execute("UPDATE forms SET name = ?1, fields = ?2, notify = ?3, success = ?4, button = ?5, consent = ?6, consent_text = ?7, privacy_url = ?8 WHERE id = ?9",
            params![name, fields, notify, v("success"), v("button"), consent, v("consent_text"), v("privacy_url"), id]).map_err(s)?;
        Ok(id)
    }
}

/// Messaggi ricevuti da un modulo, dal più recente.
pub fn entries(db: &Connection, id: i64) -> Vec<(i64, i64, bool, Vec<(String, String)>)> {
    db.prepare("SELECT id, created_at, seen, data FROM form_entries WHERE form_id = ?1 ORDER BY id DESC LIMIT 2000")
        .and_then(|mut q| q.query_map([id], |r| {
            let d: Vec<Value> = serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default();
            Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? == 1, d.iter().map(|p| (p[0].as_str().unwrap_or("").to_string(), p[1].as_str().unwrap_or("").to_string())).collect()))
        }).map(|r| r.filter_map(Result::ok).collect()))
        .unwrap_or_default()
}
