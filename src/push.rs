//! Notifiche push dei browser (Web Push), senza servizi esterni come OneSignal.
//! - Chiavi VAPID (RFC 8292) generate dal pannello: identificano il sito verso i servizi push di Google, Mozilla, Apple, Microsoft.
//! - Messaggi cifrati come prevede lo standard (RFC 8291, aes128gcm): solo il browser dell'iscritto li può leggere.
//! - Sul sito: un pulsante; lo script (/push.js) si scarica solo quando il lettore lo preme.
use crate::{now, opt, site, App, R, Settings};
use base64::Engine;
use ring::{aead, agreement, hkdf, rand::{SecureRandom, SystemRandom}, signature::{self, KeyPair}};
use rusqlite::params;
use serde_json::{json, Value};

const B64U: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Notifiche da inviare (titolo, testo, indirizzo): le mette la pubblicazione, le invia il ciclo in sottofondo.
pub static QUEUE: std::sync::Mutex<Vec<(String, String, String)>> = std::sync::Mutex::new(Vec::new());

pub fn on(st: &Settings) -> bool { st.get("push_on").is_some_and(|v| v == "on") && !opt(st, "push_vapid_public", "").is_empty() }

/// Chiavi VAPID: si creano la prima volta che la funzione viene attivata. Restituisce la chiave pubblica.
pub fn ensure_keys(app: &App) -> R<String> {
    let st = app.settings();
    let public = opt(&st, "push_vapid_public", "");
    if !public.is_empty() && !opt(&st, "push_vapid_private", "").is_empty() { return Ok(public.to_string()) }
    let rng = SystemRandom::new();
    let pkcs8 = signature::EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng).map_err(|_| "chiavi non generate")?;
    let pair = signature::EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).map_err(|_| "chiavi non valide")?;
    let public = B64U.encode(pair.public_key().as_ref());
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    db.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('push_vapid_private', ?1)", [B64.encode(pkcs8.as_ref())]).map_err(|e| e.to_string())?;
    db.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('push_vapid_public', ?1)", [&public]).map_err(|e| e.to_string())?;
    Ok(public)
}

struct Len(usize);
impl hkdf::KeyType for Len { fn len(&self) -> usize { self.0 } }
fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(ikm);
    let mut out = vec![0u8; len];
    prk.expand(&[info], Len(len)).expect("lunghezza HKDF").fill(&mut out).expect("HKDF");
    out
}

/// Cifra il messaggio per un iscritto (RFC 8291, codifica aes128gcm, un solo blocco).
pub fn encrypt(p256dh: &[u8], auth: &[u8], payload: &[u8]) -> R<Vec<u8>> {
    let rng = SystemRandom::new();
    let eph = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).map_err(|_| "chiave temporanea")?;
    let as_pub = eph.compute_public_key().map_err(|_| "chiave pubblica temporanea")?.as_ref().to_vec();
    let secret = agreement::agree_ephemeral(eph, &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, p256dh), |s| s.to_vec())
        .map_err(|_| "la chiave dell'iscritto non è valida")?;
    let key_info = [b"WebPush: info\0".as_slice(), p256dh, &as_pub].concat();
    let ikm = hkdf(auth, &secret, &key_info, 32);
    let mut salt = [0u8; 16];
    rng.fill(&mut salt).map_err(|_| "casualità")?;
    let cek = hkdf(&salt, &ikm, b"Content-Encoding: aes128gcm\0", 16);
    let nonce = hkdf(&salt, &ikm, b"Content-Encoding: nonce\0", 12);
    let mut data = payload.to_vec();
    data.push(2); // ultimo (e unico) blocco
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).map_err(|_| "chiave AES")?);
    key.seal_in_place_append_tag(aead::Nonce::try_assume_unique_for_key(&nonce).map_err(|_| "nonce")?, aead::Aad::empty(), &mut data).map_err(|_| "cifratura")?;
    let mut body = salt.to_vec();
    body.extend_from_slice(&4096u32.to_be_bytes());
    body.push(as_pub.len() as u8);
    body.extend_from_slice(&as_pub);
    body.extend_from_slice(&data);
    Ok(body)
}

/// Intestazione Authorization VAPID: un gettone firmato valido 12 ore per il servizio push dell'iscritto.
fn vapid(st: &Settings, endpoint: &str) -> R<String> {
    let pkcs8 = B64.decode(opt(st, "push_vapid_private", "")).map_err(|_| "chiave VAPID illeggibile")?;
    let rng = SystemRandom::new();
    let pair = signature::EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8, &rng).map_err(|_| "chiave VAPID non valida")?;
    let aud = endpoint.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
    let sub = if opt(st, "smtp_from", "").contains('@') { format!("mailto:{}", opt(st, "smtp_from", "")) } else { site::base(st).to_string() };
    let unsigned = format!("{}.{}", B64U.encode(br#"{"typ":"JWT","alg":"ES256"}"#), B64U.encode(json!({"aud": aud, "exp": now() + 12 * 3600, "sub": sub}).to_string()));
    let sig = pair.sign(&rng, unsigned.as_bytes()).map_err(|_| "firma VAPID")?;
    Ok(format!("vapid t={unsigned}.{}, k={}", B64U.encode(sig.as_ref()), opt(st, "push_vapid_public", "")))
}

/// Servizi push dei browser: Chrome/Edge/Android (Google), Firefox (Mozilla), Safari (Apple), Windows (Microsoft).
/// Gli indirizzi arrivano dai browser dei lettori: si accettano solo questi, così nessuno può far chiamare al server altri indirizzi.
/// L'indirizzo si analizza con un parser vero: tagliare il testo a mano faceva passare, per esempio,
/// «https://attaccante.it?.push.apple.com» (per il controllo finiva in .push.apple.com, ma il server si collegava ad attaccante.it).
fn allowed(endpoint: &str) -> bool {
    if test_local(endpoint) { return true }
    let Ok(u) = url::Url::parse(endpoint) else { return false };
    if u.scheme() != "https" || u.port().is_some() || !u.username().is_empty() || u.password().is_some() { return false }
    let Some(url::Host::Domain(h)) = u.host() else { return false }; // niente indirizzi IP scritti a mano
    h == "fcm.googleapis.com" || h == "updates.push.services.mozilla.com" || h.ends_with(".push.apple.com") || h.ends_with(".notify.windows.com")
}
fn test_local(endpoint: &str) -> bool { crate::test_env("PRESSTATIC_TEST_LOCAL_FETCH").is_some() && endpoint.starts_with("http://127.0.0.1:") }

/// Iscrizione dal browser del lettore (quello che restituisce pushManager.subscribe()).
pub fn subscribe(app: &App, body: &[u8], ip: &str) -> R<()> {
    let st = app.settings();
    if !on(&st) { return Err("le notifiche non sono attive".into()) }
    let v: Value = serde_json::from_slice(body).map_err(|_| "iscrizione illeggibile")?;
    let endpoint = v["endpoint"].as_str().unwrap_or("").to_string();
    if endpoint.len() > 1000 || !allowed(&endpoint) { return Err("servizio push non riconosciuto".into()) }
    let p256dh = B64U.decode(v["keys"]["p256dh"].as_str().unwrap_or("").trim_end_matches('=')).map_err(|_| "chiave non valida")?;
    let auth = B64U.decode(v["keys"]["auth"].as_str().unwrap_or("").trim_end_matches('=')).map_err(|_| "chiave non valida")?;
    if p256dh.len() != 65 || p256dh[0] != 4 || auth.len() != 16 { return Err("chiavi dell'iscrizione non valide".into()) }
    let who = crate::ip_hash(app, ip);
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let recent: i64 = db.query_row("SELECT COUNT(*) FROM push_subs WHERE ip_hash = ?1 AND created_at > ?2", params![who, now() - 600], |r| r.get(0)).unwrap_or(0);
    if recent >= 10 { return Err("troppe iscrizioni da questa connessione".into()) }
    db.execute("INSERT INTO push_subs(endpoint, p256dh, auth, ip_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(endpoint) DO UPDATE SET p256dh = ?2, auth = ?3",
        params![endpoint, B64U.encode(&p256dh), B64U.encode(&auth), who, now()]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Invia una notifica a tutti gli iscritti. Le iscrizioni scadute (il servizio risponde 404 o 410) vengono tolte.
pub fn broadcast(app: &App, title: &str, body: &str, url: &str) -> R<String> {
    let st = app.settings();
    if !on(&st) { return Err("le notifiche non sono attive".into()) }
    let subs: Vec<(i64, String, String, String)> = app.db.lock().unwrap_or_else(|e| e.into_inner())
        .prepare("SELECT id, endpoint, p256dh, auth FROM push_subs").and_then(|mut q| q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map(|r| r.filter_map(Result::ok).collect())).unwrap_or_default();
    let icon = [opt(&st, "favicon", ""), opt(&st, "logo", "")].into_iter().find(|x| !x.is_empty()).map(|x| format!("{}{}", site::base(&st), x)).unwrap_or_default();
    let payload = json!({"title": title, "body": body, "url": url, "icon": icon}).to_string();
    // Anche per i servizi push ammessi, la connessione va solo a indirizzi pubblici (come per i download dell'IA).
    let local = crate::test_env("PRESSTATIC_TEST_LOCAL_FETCH").is_some();
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(10)).redirects(0)
        .resolver(move |netloc: &str| -> std::io::Result<Vec<std::net::SocketAddr>> {
            use std::net::ToSocketAddrs;
            let ok: Vec<std::net::SocketAddr> = netloc.to_socket_addrs()?.filter(|a| local || crate::ai::public(a.ip())).collect();
            if ok.is_empty() { Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "indirizzo non consentito")) } else { Ok(ok) }
        }).build();
    let (mut sent, mut gone, mut failed) = (0, 0, 0);
    for (id, endpoint, p256dh, auth) in subs {
        if !allowed(&endpoint) { continue }
        let (Ok(k), Ok(a)) = (B64U.decode(&p256dh), B64U.decode(&auth)) else { continue };
        let result = encrypt(&k, &a, payload.as_bytes()).and_then(|b| vapid(&st, &endpoint).map(|v| (b, v))).map(|(b, v)| agent.post(&endpoint)
            .set("Authorization", &v).set("TTL", "86400").set("Urgency", "normal").set("Content-Encoding", "aes128gcm").set("Content-Type", "application/octet-stream").send_bytes(&b));
        match result {
            Ok(Ok(_)) => sent += 1,
            Ok(Err(ureq::Error::Status(404 | 410, _))) => { gone += 1; let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("DELETE FROM push_subs WHERE id = ?1", [id]); }
            _ => failed += 1,
        }
    }
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT INTO push_sends(title, body, sent, failed, created_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![title, body, sent, failed, now()]);
    Ok(format!("Notifica inviata a {sent} iscritti{}{}.", if gone > 0 { format!("; {gone} iscrizioni scadute tolte") } else { String::new() }, if failed > 0 { format!("; {failed} invii non riusciti") } else { String::new() }))
}

/// Pulsante sotto l'articolo. Lo script vero (/push.js) si scarica solo al clic: la pagina resta leggera.
pub fn button_html(st: &Settings) -> String {
    if !on(st) { return String::new() }
    let label = site::esc(opt(st, "push_label", "Ricevi una notifica per le notizie importanti"));
    format!("<aside class=\"pushbox\" data-pagefind-ignore><style>.pushbox{{margin:1.5rem 0}}.pushbox button{{display:inline-flex;align-items:center;gap:.55rem;font:inherit;font-weight:700;padding:.7rem 1.1rem;border:1px solid var(--line,#d5d9e2);border-radius:999px;background:var(--soft,#f4f6fa);color:var(--ink,#111);cursor:pointer}}.pushbox button:hover{{border-color:var(--accent,#1e6bff)}}.pushbox svg{{color:var(--accent,#1e6bff)}}.pushbox button.wait svg{{display:none}}.pushbox button.wait::before{{content:\"\";flex:none;width:17px;height:17px;box-sizing:border-box;border-radius:50%;border:2.5px solid currentColor;border-right-color:transparent;animation:pspin .75s linear infinite}}\
@keyframes pspin{{to{{transform:rotate(360deg)}}}}@media (prefers-reduced-motion:reduce){{.pushbox button.wait::before{{animation-duration:2.4s}}}}.pushbox button:disabled{{cursor:default}}\
.pushbox button.ok{{border-color:#15803d;color:#15803d}}</style>\
<button type=\"button\" data-push data-key=\"{}\" data-endpoint=\"{}\" onclick=\"if(!this.dataset.l){{this.dataset.l=1;var s=document.createElement('script');s.src='/push.js';document.head.appendChild(s)}}\">\
<svg width=\"18\" height=\"18\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" aria-hidden=\"true\"><path d=\"M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9\"/><path d=\"M10.3 21a1.94 1.94 0 0 0 3.4 0\"/></svg><span>{label}</span></button></aside>",
        site::esc(opt(st, "push_vapid_public", "")), site::esc(&format!("{}/push/iscrivi", site::base(st))))
}

pub const SERVICE_WORKER: &str = r#"// Presstatic: notifiche push. Mostra la notifica e al clic apre l'articolo.
self.addEventListener('push', e => {
  const d = e.data ? e.data.json() : {};
  e.waitUntil(self.registration.showNotification(d.title || '', { body: d.body || '', icon: d.icon || undefined, data: { url: d.url || '/' } }));
});
self.addEventListener('notificationclick', e => { e.notification.close(); e.waitUntil(clients.openWindow(e.notification.data.url)); });
"#;

pub const SUBSCRIBE_JS: &str = r#"// Presstatic: iscrizione alle notifiche, scaricato solo quando il lettore preme il pulsante.
// Mentre il browser chiede il permesso il pulsante mostra una rotella e dice cosa fare; se il lettore chiude la
// richiesta senza scegliere, il pulsante torna premibile.
(() => {
  const b = document.querySelector('[data-push]'), s = b.querySelector('span');
  s.setAttribute('aria-live', 'polite');
  const say = (t, st) => { s.textContent = t; b.classList.toggle('wait', st === 'wait'); b.classList.toggle('ok', st === 'ok'); b.disabled = st === 'wait' || st === 'ok'; };
  const run = async () => {
    try {
      if (!('serviceWorker' in navigator) || !('PushManager' in window) || !('Notification' in window)) { say('Questo browser non supporta le notifiche.'); return; }
      if (Notification.permission === 'denied') { say('Notifiche bloccate: puoi riattivarle dalle impostazioni del sito nel browser.'); return; }
      say(Notification.permission === 'granted' ? 'Attivazione…' : 'Clicca «Consenti» nella finestra del browser', 'wait');
      await navigator.serviceWorker.register('/sw.js');
      const reg = await navigator.serviceWorker.ready; // l'iscrizione si può chiedere solo quando il service worker è attivo
      const perm = await Notification.requestPermission();
      if (perm === 'denied') { say('Notifiche non autorizzate: puoi attivarle dalle impostazioni del browser.'); return; }
      if (perm !== 'granted') { say('Richiesta chiusa: clicca di nuovo per attivare le notifiche'); return; }
      say('Quasi fatto…', 'wait');
      const k = b.dataset.key.replace(/-/g, '+').replace(/_/g, '/'), raw = atob(k + '='.repeat((4 - k.length % 4) % 4));
      const sub = await reg.pushManager.getSubscription() || await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: Uint8Array.from(raw, c => c.charCodeAt(0)) });
      await fetch(b.dataset.endpoint, { method: 'POST', mode: 'no-cors', headers: { 'Content-Type': 'text/plain' }, body: JSON.stringify(sub) });
      say('Notifiche attive: ti avvisiamo per le notizie importanti', 'ok');
    } catch (e) {
      console.error(e);
      say('Non è stato possibile attivare le notifiche su questo browser. Riprova più tardi.');
    }
  };
  b.addEventListener('click', () => { if (!b.disabled) run(); }); // i clic successivi (lo script si carica una volta sola)
  run();
})();
"#;

/// File sul sito pubblico: il service worker deve stare alla radice. Senza notifiche attive, i file spariscono.
pub fn write_files(app: &App) {
    let st = app.settings();
    let (sw, js) = (app.public.join("sw.js"), app.public.join("push.js"));
    if on(&st) {
        let _ = std::fs::write(&sw, SERVICE_WORKER);
        let _ = std::fs::write(&js, SUBSCRIBE_JS);
    } else {
        let _ = std::fs::remove_file(sw);
        let _ = std::fs::remove_file(js);
    }
}
