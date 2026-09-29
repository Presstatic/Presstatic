//! Backup: un archivio .tar.gz con la copia coerente del database, i temi personalizzati, le immagini e i file caricati.
//! Pagine, elenchi, sitemap e indice di ricerca non servono: si rigenerano dal database al primo avvio dopo il ripristino.
//! Destinazioni: qualsiasi archivio compatibile S3 (Wasabi, Backblaze B2, Hetzner Object Storage, Amazon S3, Cloudflare R2),
//! con la firma AWS versione 4; gli archivi grandi si caricano a pezzi (multipart). Si tengono le ultime N copie.
use crate::{now, opt, site, App, R, Settings};
use ring::{digest, hmac};
use rusqlite::params;
use serde::Serialize;
use std::{fs, io::{Read, Write}, os::unix::fs::{OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}, sync::{LazyLock, Mutex}, time::Duration};

pub const DIR: &str = "backups";
const PART: usize = 32 * 1024 * 1024; // pezzi da 32 MB per gli archivi grandi (il minimo di S3 è 5 MB)

#[derive(Default, Clone, Serialize)]
pub struct State { pub running: bool, pub phase: String, pub last: String }
pub static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);
fn phase(p: &str) { STATE.lock().unwrap_or_else(|e| e.into_inner()).phase = p.to_string() }

/// Nel sito pubblico si salvano solo i file che non si possono rigenerare.
const GENERATED: &[&str] = &["index.html", "404.html", "feed.xml", "sitemap.xml", "news-sitemap.xml", "robots.txt", "sw.js", "push.js",
    "assets", "autori", "category", "cerca", "page", "pagefind", "sitemaps", "tag", "media"];

/// Crea l'archivio in backups/. Restituisce il percorso.
pub fn archive(app: &App) -> R<PathBuf> {
    fs::create_dir_all(DIR).map_err(|e| format!("cartella dei backup non creata: {e}"))?;
    // I backup contengono il database con tutti i segreti (SMTP, S3, IA, VAPID, 2FA): solo il servizio li può leggere,
    // come il database vero. Il servizio gira con UMask=0022 (Nginx deve leggere le pagine), quindi va fatto qui.
    let _ = fs::set_permissions(DIR, fs::Permissions::from_mode(0o700));
    let st = app.settings();
    let stamp = jiff::Timestamp::from_second(now()).unwrap_or(jiff::Timestamp::UNIX_EPOCH).to_zoned(site::tz(&st)).strftime("%Y%m%d-%H%M%S").to_string();
    let name = format!("presstatic-backup-{stamp}.tar.gz");
    let (snap, tmp, out) = (Path::new(DIR).join(".database.db"), Path::new(DIR).join(format!(".{name}")), Path::new(DIR).join(&name));
    let _ = fs::remove_file(&snap);
    // La copia del database nasce già privata: VACUUM INTO accetta un file vuoto esistente.
    fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&snap).map_err(|e| format!("copia del database non creata: {e}"))?;
    // Copia coerente del database anche mentre la redazione lavora (non una copia del file, che potrebbe essere a metà).
    app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("VACUUM INTO ?1", [snap.to_string_lossy()]).map_err(|e| format!("copia del database non riuscita: {e}"))?;
    let result = (|| -> std::io::Result<()> {
        let gz = flate2::write::GzEncoder::new(fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?, flate2::Compression::new(6));
        let mut tar = tar::Builder::new(gz);
        tar.follow_symlinks(false);
        tar.append_path_with_name(&snap, "presstatic.db")?;
        if Path::new("themes").is_dir() { tar.append_dir_all("themes", "themes")?; }
        let media = app.public.join("media");
        if media.is_dir() { tar.append_dir_all("public/media", &media)?; }
        for e in fs::read_dir(&app.public).into_iter().flatten().flatten() { // sito mai generato: niente da aggiungere
            let n = e.file_name().to_string_lossy().to_string();
            if GENERATED.contains(&n.as_str()) || n.ends_with(".gz") || n.ends_with(".br") || n.starts_with('.') { continue }
            let p = e.path();
            if p.is_dir() { if !site::cms_owned(&p) { tar.append_dir_all(format!("public/{n}"), &p)? } } else { tar.append_path_with_name(&p, format!("public/{n}"))? }
        }
        tar.into_inner()?.finish()?.flush()
    })();
    let _ = fs::remove_file(&snap);
    if let Err(e) = result { let _ = fs::remove_file(&tmp); return Err(format!("archivio non creato: {e}")) }
    fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
    Ok(out)
}

// ---------- archivi compatibili S3 ----------

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
fn sha(b: &[u8]) -> String { hex(digest::digest(&digest::SHA256, b).as_ref()) }
fn mac(key: &[u8], msg: &str) -> Vec<u8> { hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), msg.as_bytes()).as_ref().to_vec() }
/// Codifica di S3: lettere, cifre e - _ . ~ restano; tutto il resto diventa %XX (la barra solo se richiesto).
fn uri(s: &str, slash: bool) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) || (b == b'/' && !slash) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub struct S3 { endpoint: String, host: String, region: String, bucket: String, key: String, secret: String, pub prefix: String, agent: ureq::Agent }

impl S3 {
    pub fn from(st: &Settings) -> R<S3> {
        let endpoint = opt(st, "s3_endpoint", "").trim().trim_end_matches('/').to_string();
        let local = crate::test_env("PRESSTATIC_TEST_LOCAL_FETCH").is_some() && endpoint.starts_with("http://127.0.0.1:");
        if !endpoint.starts_with("https://") && !local { return Err("l'indirizzo dell'archivio deve iniziare con https://".into()) }
        let host = endpoint.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("").to_string();
        let g = |k: &str| opt(st, k, "").trim().to_string();
        let (region, bucket, key, secret) = (g("s3_region"), g("s3_bucket"), g("s3_key"), g("s3_secret"));
        if host.is_empty() || region.is_empty() || bucket.is_empty() || key.is_empty() || secret.is_empty() { return Err("completa indirizzo, regione, bucket e chiavi di accesso dell'archivio".into()) }
        let prefix = g("s3_prefix").trim_matches('/').to_string();
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(300)).build();
        Ok(S3 { endpoint, host, region, bucket, key, secret, prefix, agent })
    }

    fn object(&self, name: &str) -> String { if self.prefix.is_empty() { name.to_string() } else { format!("{}/{name}", self.prefix) } }

    /// Richiesta firmata (AWS Signature Version 4). `query`: parametri già in chiaro, vengono codificati qui.
    fn call(&self, method: &str, key: &str, query: &[(&str, String)], body: &[u8]) -> Result<ureq::Response, String> {
        let t = jiff::Timestamp::now();
        let (amz, day) = (t.strftime("%Y%m%dT%H%M%SZ").to_string(), t.strftime("%Y%m%d").to_string());
        let path = format!("/{}{}", uri(&self.bucket, true), if key.is_empty() { String::new() } else { format!("/{}", uri(key, false)) });
        let mut q: Vec<(String, String)> = query.iter().map(|(k, v)| (uri(k, true), uri(v, true))).collect();
        q.sort();
        let qs = q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
        let payload = sha(body);
        let canonical = format!("{method}\n{path}\n{qs}\nhost:{}\nx-amz-content-sha256:{payload}\nx-amz-date:{amz}\n\nhost;x-amz-content-sha256;x-amz-date\n{payload}", self.host);
        let scope = format!("{day}/{}/s3/aws4_request", self.region);
        let to_sign = format!("AWS4-HMAC-SHA256\n{amz}\n{scope}\n{}", sha(canonical.as_bytes()));
        let k = mac(&mac(&mac(&mac(format!("AWS4{}", self.secret).as_bytes(), &day), &self.region), "s3"), "aws4_request");
        let auth = format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature={}", self.key, hex(&mac(&k, &to_sign)));
        let url = format!("{}{path}{}", self.endpoint, if qs.is_empty() { String::new() } else { format!("?{qs}") });
        let req = self.agent.request(method, &url).set("Authorization", &auth).set("x-amz-date", &amz).set("x-amz-content-sha256", &payload);
        match req.send_bytes(body) {
            Ok(r) => Ok(r),
            Err(ureq::Error::Status(c, r)) => {
                let text = r.into_string().unwrap_or_default();
                let code = between(&text, "<Code>", "</Code>").unwrap_or_default();
                Err(match (c, code.as_str()) {
                    (403, "SignatureDoesNotMatch") => "l'archivio rifiuta la firma: controlla la chiave segreta e la regione".into(),
                    (403, _) | (401, _) => "accesso negato: controlla le chiavi e che abbiano il permesso di scrivere nel bucket".into(),
                    (404, "NoSuchBucket") => format!("il bucket «{}» non esiste in questa regione", self.bucket),
                    _ => format!("l'archivio ha risposto {c} {code}"),
                })
            }
            Err(e) => Err(format!("archivio non raggiungibile: {e}")),
        }
    }

    /// Carica un file: in una sola richiesta se piccolo, a pezzi da 32 MB se grande.
    pub fn upload(&self, file: &Path, name: &str) -> R<()> {
        let key = self.object(name);
        let size = fs::metadata(file).map_err(|e| e.to_string())?.len() as usize;
        let mut f = fs::File::open(file).map_err(|e| e.to_string())?;
        if size <= PART {
            let mut b = Vec::with_capacity(size);
            f.read_to_end(&mut b).map_err(|e| e.to_string())?;
            return self.call("PUT", &key, &[], &b).map(|_| ());
        }
        let r = self.call("POST", &key, &[("uploads", String::new())], b"")?;
        let id = between(&r.into_string().unwrap_or_default(), "<UploadId>", "</UploadId>").ok_or("l'archivio non ha aperto il caricamento a pezzi")?;
        let result = (|| -> R<()> {
            let mut parts = vec![];
            let mut buf = vec![0u8; PART];
            for n in 1.. {
                let mut len = 0;
                while len < PART { match f.read(&mut buf[len..]).map_err(|e| e.to_string())? { 0 => break, k => len += k } }
                if len == 0 { break }
                let r = self.call("PUT", &key, &[("partNumber", n.to_string()), ("uploadId", id.clone())], &buf[..len])?;
                parts.push((n, r.header("ETag").unwrap_or("").to_string()));
                if len < PART { break }
            }
            let xml = format!("<CompleteMultipartUpload>{}</CompleteMultipartUpload>", parts.iter().map(|(n, e)| format!("<Part><PartNumber>{n}</PartNumber><ETag>{e}</ETag></Part>")).collect::<String>());
            let r = self.call("POST", &key, &[("uploadId", id.clone())], xml.as_bytes())?;
            let text = r.into_string().unwrap_or_default();
            if text.contains("<Error>") { return Err(format!("caricamento a pezzi non completato: {}", between(&text, "<Code>", "</Code>").unwrap_or_default())) }
            Ok(())
        })();
        if result.is_err() { let _ = self.call("DELETE", &key, &[("uploadId", id)], b""); } // niente pezzi orfani (occupano spazio e costano)
        result
    }

    /// Backup presenti nell'archivio (nomi), dal più vecchio.
    pub fn list(&self) -> R<Vec<String>> {
        let p = self.object("presstatic-backup-");
        let text = self.call("GET", "", &[("list-type", "2".into()), ("prefix", p)], b"")?.into_string().map_err(|e| e.to_string())?;
        let mut keys: Vec<String> = text.split("<Key>").skip(1).filter_map(|x| x.split("</Key>").next()).map(|k| k.rsplit('/').next().unwrap_or(k).to_string()).collect();
        keys.sort();
        Ok(keys)
    }

    pub fn delete(&self, name: &str) -> R<()> { self.call("DELETE", &self.object(name), &[], b"").map(|_| ()) }

    /// Prova del collegamento: scrive e cancella un piccolo file.
    pub fn test(&self) -> R<()> {
        self.call("PUT", &self.object(".presstatic-prova.txt"), &[], b"Prova di scrittura di Presstatic: si puo' cancellare.")?;
        self.call("DELETE", &self.object(".presstatic-prova.txt"), &[], b"").map(|_| ())
    }
}

fn between(s: &str, a: &str, b: &str) -> Option<String> { Some(s.split(a).nth(1)?.split(b).next()?.to_string()) }

/// Esegue un backup completo: archivio, invio alle destinazioni attive, pulizia delle copie vecchie, registro.
pub fn run(app: &App) -> R<String> {
    {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if s.running { return Err("un backup è già in corso".into()) }
        *s = State { running: true, phase: "Creazione dell'archivio".into(), last: s.last.clone() };
    }
    let result = (|| -> R<String> {
        let st = app.settings();
        let file = archive(app)?;
        let name = file.file_name().unwrap_or_default().to_string_lossy().to_string();
        let size = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
        let keep: usize = opt(&st, "backup_keep", "14").parse().unwrap_or(14).clamp(1, 365);
        let mut notes = vec![format!("archivio {name} ({:.1} MB)", size as f64 / 1_048_576.0)];
        let mut ok = true;
        if st.get("s3_on").is_some_and(|v| v == "on") {
            phase("Invio all'archivio S3");
            match S3::from(&st).and_then(|s3| { s3.upload(&file, &name)?; let all = s3.list()?; let old = all.len().saturating_sub(keep); for n in &all[..old] { s3.delete(n)?; } Ok(old) }) {
                Ok(old) => notes.push(format!("caricato nell'archivio S3{}", if old > 0 { format!(", tolte {old} copie vecchie") } else { String::new() })),
                Err(e) => { ok = false; notes.push(format!("archivio S3 non riuscito: {e}")) }
            }
        }
        // Copie locali: si tengono le ultime N (anche sul server, per un ripristino veloce).
        let local: usize = opt(&st, "backup_local_keep", "2").parse().unwrap_or(2).min(30);
        let mut files: Vec<String> = fs::read_dir(DIR).map_err(|e| e.to_string())?.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with("presstatic-backup-") && n.ends_with(".tar.gz")).collect();
        files.sort();
        for n in &files[..files.len().saturating_sub(local)] { let _ = fs::remove_file(Path::new(DIR).join(n)); }
        let msg = format!("Backup {}: {}.", if ok { "completato" } else { "con errori" }, notes.join("; "));
        let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT INTO backups(name, size, ok, note, created_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![name, size as i64, ok, msg, now()]);
        if ok { Ok(msg) } else { Err(msg) }
    })();
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    s.running = false; s.phase = String::new();
    s.last = match &result { Ok(m) => m.clone(), Err(e) => e.clone() };
    result
}

/// È l'ora del backup automatico di oggi? (fuso orario del sito)
pub fn due(st: &Settings) -> bool {
    if st.get("backup_on").is_none_or(|v| v != "on") { return false }
    let t = jiff::Timestamp::from_second(now()).unwrap_or(jiff::Timestamp::UNIX_EPOCH).to_zoned(site::tz(st));
    t.hour() >= opt(st, "backup_hour", "3").parse::<i8>().unwrap_or(3) && opt(st, "backup_day", "") != t.strftime("%Y-%m-%d").to_string()
}

/// `presstatic ripristina <archivio>`: da lanciare nella cartella del sito, con il servizio fermo.
/// Rimette database, temi e file; al riavvio il sito si rigenera da solo.
/// Controlla un archivio senza scrivere niente: solo percorsi relativi dentro presstatic.db, public/ e themes/, niente
/// collegamenti. Restituisce quanti elementi contiene e se c'è il database.
fn check_archive(file: &Path) -> R<(usize, bool)> {
    let f = fs::File::open(file).map_err(|e| format!("archivio non leggibile: {e}"))?;
    let mut a = tar::Archive::new(flate2::read::GzDecoder::new(f));
    let (mut n, mut db) = (0, false);
    for entry in a.entries().map_err(|e| format!("archivio non valido: {e}"))? {
        let e = entry.map_err(|e| format!("archivio non valido: {e}"))?;
        let path = e.path().map_err(|e| format!("archivio non valido: {e}"))?.to_path_buf();
        if path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir)) { return Err(format!("percorso non ammesso: {}", path.display())) }
        if !(path == Path::new("presstatic.db") || path.starts_with("public") || path.starts_with("themes")) { return Err(format!("percorso fuori dalle cartelle previste: {}", path.display())) }
        if matches!(e.header().entry_type(), tar::EntryType::Symlink | tar::EntryType::Link) { return Err(format!("collegamento non ammesso nell'archivio: {}", path.display())) }
        if path == Path::new("presstatic.db") { db = true }
        n += 1;
    }
    Ok((n, db))
}

/// Ripristino dal pannello, a servizio acceso. Prima l'archivio si controlla tutto (se è manomesso non si scrive niente);
/// poi una copia di sicurezza dello stato attuale; poi file e immagini tornano al loro posto e il database si sostituisce
/// con la copia sicura di SQLite, senza scambiare file sotto il programma; infine il sito si rigenera.
pub fn restore_live(app: &App, file: &Path) -> R<String> {
    let (_, has_db) = check_archive(file)?;
    if !has_db { return Err("nell'archivio non c'è il database: non sembra un backup di Presstatic".into()) }
    let safety = archive(app).map_err(|e| format!("copia di sicurezza non riuscita, ripristino annullato: {e}"))?;
    let tmp = PathBuf::from(DIR).join(format!("ripristino-{}.db", now()));
    let f = fs::File::open(file).map_err(|e| format!("archivio non leggibile: {e}"))?;
    let mut a = tar::Archive::new(flate2::read::GzDecoder::new(f));
    let mut n = 0;
    for entry in a.entries().map_err(|e| e.to_string())? {
        let mut e = entry.map_err(|e| e.to_string())?;
        let path = e.path().map_err(|e| e.to_string())?.to_path_buf();
        let dest = if path == Path::new("presstatic.db") { tmp.clone() } else if let Ok(rest) = path.strip_prefix("public") { app.public.join(rest) } else { path.clone() };
        let mut walk = dest.clone(); // nessun segmento della destinazione può essere un collegamento simbolico
        while let Some(parent) = walk.parent().map(|p| p.to_path_buf()) {
            if walk.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) { return Err(format!("percorso che attraversa un collegamento: {}", dest.display())) }
            if parent.as_os_str().is_empty() { break }
            walk = parent;
        }
        if let Some(parent) = dest.parent() { let _ = fs::create_dir_all(parent); }
        e.unpack(&dest).map_err(|x| format!("{}: {x}", dest.display()))?;
        n += 1;
    }
    let slugs = |db: &rusqlite::Connection, only_live: bool| -> std::collections::HashSet<String> {
        db.prepare(if only_live { "SELECT slug FROM posts WHERE status = 'published'" } else { "SELECT slug FROM posts" })
            .and_then(|mut q| q.query_map([], |r| r.get::<_, String>(0)).map(|r| r.filter_map(Result::ok).collect())).unwrap_or_default()
    };
    let (before, r, after) = {
        let mut db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let before = slugs(&db, false);
        let r = db.restore(rusqlite::DatabaseName::Main, &tmp, None::<fn(rusqlite::backup::Progress)>);
        let after = slugs(&db, true);
        (before, r, after)
    };
    let _ = fs::remove_file(&tmp);
    r.map_err(|e| format!("database non ripristinato: {e}. Lo stato di prima è nella copia di sicurezza {}", safety.display()))?;
    // Pagine che dopo il ripristino non devono più esserci. Degli articoli si tolgono quelli assenti dal database
    // ripristinato; gli elenchi (categorie, argomenti, autori, pagine della home, sitemap) nascono tutti dal database e
    // si rifanno da zero. I file caricati dalla redazione nel sito (verifiche, documenti) non si toccano.
    for s in before.difference(&after) {
        if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !site::RESERVED.contains(&s.as_str()) { let _ = fs::remove_dir_all(app.public.join(s)); }
    }
    for d in ["category", "tag", "autori", "page", "sitemaps"] { let _ = fs::remove_dir_all(app.public.join(d)); }
    let m = site::rebuild_all(app)?;
    Ok(format!("Ripristino completato: {n} elementi. Prima del ripristino è stata salvata una copia dello stato precedente ({}), la trovi tra le copie sul server. {m}", safety.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default()))
}

pub fn restore(file: &str) {
    let fail = |m: String| -> ! { eprintln!("Ripristino non riuscito: {m}"); std::process::exit(1) };
    let f = fs::File::open(file).unwrap_or_else(|e| fail(format!("{file}: {e}")));
    if Path::new("presstatic.db").exists() {
        let saved = format!("presstatic.db.prima-del-ripristino-{}", now());
        fs::rename("presstatic.db", &saved).unwrap_or_else(|e| fail(e.to_string()));
        for x in ["presstatic.db-wal", "presstatic.db-shm"] { let _ = fs::remove_file(x); }
        println!("Il database attuale è stato messo da parte in {saved}.");
    }
    let mut a = tar::Archive::new(flate2::read::GzDecoder::new(f));
    let mut n = 0;
    for entry in a.entries().unwrap_or_else(|e| fail(e.to_string())) {
        let mut e = entry.unwrap_or_else(|e| fail(e.to_string()));
        let path = e.path().unwrap_or_else(|e| fail(e.to_string())).to_path_buf();
        // Solo percorsi relativi, senza "..", e solo dentro le cartelle previste: un archivio manomesso non scrive altrove.
        if path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir)) { fail(format!("percorso non ammesso nell'archivio: {}", path.display())) }
        let under = path == Path::new("presstatic.db") || path.starts_with("public") || path.starts_with("themes");
        if !under { fail(format!("percorso fuori dalle cartelle previste: {}", path.display())) }
        // Link simbolici e hard link nell'archivio: rifiutati. Altrimenti unpack li seguirebbe e scriverebbe fuori dal sito.
        if matches!(e.header().entry_type(), tar::EntryType::Symlink | tar::EntryType::Link) { fail(format!("collegamento non ammesso nell'archivio: {}", path.display())) }
        let dest = if let Ok(rest) = path.strip_prefix("public") { std::env::var("PRESSTATIC_PUBLIC").map(PathBuf::from).unwrap_or_else(|_| "public".into()).join(rest) } else { path.clone() };
        // Nessun segmento del percorso di destinazione deve essere già un link simbolico (creato da una voce precedente).
        let mut walk = dest.clone(); let mut safe = true;
        while let Some(parent) = walk.parent().map(|p| p.to_path_buf()) {
            if walk.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) { safe = false; break }
            if parent.as_os_str().is_empty() { break }
            walk = parent;
        }
        if !safe { fail(format!("percorso che attraversa un collegamento: {}", dest.display())) }
        if let Some(parent) = dest.parent() { let _ = fs::create_dir_all(parent); }
        e.unpack(&dest).unwrap_or_else(|x| fail(format!("{}: {x}", dest.display())));
        n += 1;
    }
    let _ = fs::write(site::GEN_MARK, b""); // al prossimo avvio si rigenera tutto il sito
    println!("Ripristinati {n} elementi. Avvia il servizio: il sito si rigenera da solo al primo avvio.");
}
