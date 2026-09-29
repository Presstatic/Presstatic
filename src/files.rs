//! File manager del pannello, solo per gli amministratori.
//!
//! Lavora esclusivamente dentro due cartelle: `public` (il sito generato, con media e caricamenti) e `themes`
//! (i temi personalizzati). Ogni percorso viene normalizzato e poi confrontato con la cartella radice anche dopo
//! aver risolto i collegamenti simbolici, quindi non si può uscire da lì né con "..", né con link.
//! Gli ZIP si estraggono con le stesse regole: nomi con ".." o assoluti vengono saltati, i link ignorati,
//! e ci sono limiti su numero e dimensione dei file.

use crate::{App, R};
use jiff::{tz::TimeZone, Timestamp};
use std::{fs, io::Read, path::{Path, PathBuf}};

pub const ROOTS: &[(&str, &str)] = &[("public", "Sito pubblico"), ("themes", "Temi")];
const MAX_ENTRIES: usize = 5000;
const MAX_TOTAL: u64 = 300 * 1024 * 1024;
const MAX_FILE: u64 = 100 * 1024 * 1024;

#[derive(serde::Serialize)]
pub struct Entry { pub name: String, pub rel: String, pub size: String, pub bytes: u64, pub modified: String, pub is_dir: bool, pub is_zip: bool }

#[derive(serde::Serialize)]
pub struct Listing { pub rel: String, pub crumbs: Vec<(String, String)>, pub entries: Vec<Entry>, pub roots: Vec<(String, String)> }

fn root_dir(app: &App, root: &str) -> Option<PathBuf> {
    match root { "public" => Some(app.public.clone()), "themes" => Some(PathBuf::from("themes")), _ => None }
}

/// Normalizza un percorso relativo ("public/media/2026") e restituisce il percorso su disco e quello pulito.
/// `must_exist`: se il percorso deve già esistere, viene controllato anche dopo aver risolto i link simbolici.
pub fn resolve(app: &App, rel: &str, must_exist: bool) -> R<(PathBuf, String)> {
    let rel = rel.trim().replace('\\', "/");
    let parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
    if parts.iter().any(|p| *p == ".." || p.contains('\0')) { return Err("percorso non valido".into()) }
    let root = parts.first().copied().ok_or("scegli una cartella")?;
    let base = root_dir(app, root).ok_or("cartella non ammessa: si può lavorare solo in public e themes")?;
    if !base.exists() { fs::create_dir_all(&base).map_err(|e| e.to_string())?; }
    let mut path = base.clone();
    for p in &parts[1..] { path.push(p) }
    if must_exist {
        let (real, real_base) = (path.canonicalize().map_err(|_| "file o cartella non trovati")?, base.canonicalize().map_err(|e| e.to_string())?);
        if !real.starts_with(&real_base) { return Err("percorso non ammesso".into()) }
    }
    Ok((path, parts.join("/")))
}

fn human(b: u64) -> String {
    if b < 1024 { format!("{b} B") } else if b < 1024 * 1024 { format!("{:.0} KB", b as f64 / 1024.0) } else { format!("{:.1} MB", b as f64 / 1048576.0) }
}

pub fn list(app: &App, rel: &str) -> R<Listing> {
    let roots = ROOTS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    if rel.trim().is_empty() {
        return Ok(Listing { rel: String::new(), crumbs: vec![], entries: vec![], roots });
    }
    let (path, rel) = resolve(app, rel, true)?;
    if !path.is_dir() { return Err("non è una cartella".into()) }
    let tz = TimeZone::get(&crate::opt(&app.settings(), "timezone", "Europe/Rome")).unwrap_or(TimeZone::UTC);
    let mut entries = vec![];
    for e in fs::read_dir(&path).map_err(|e| e.to_string())?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        // Le versioni compresse che Presstatic crea accanto alle pagine (index.html.gz, index.html.br) non si mostrano:
        // seguono il file principale da sole.
        if let Some(base) = name.strip_suffix(".gz").or_else(|| name.strip_suffix(".br")) {
            if path.join(base).is_file() { continue }
        }
        let Ok(m) = e.metadata() else { continue }; // i link rotti si saltano
        let modified = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| Timestamp::from_second(d.as_secs() as i64).ok()).map(|t| t.to_zoned(tz.clone()).strftime("%d/%m/%Y %H:%M").to_string()).unwrap_or_default();
        entries.push(Entry {
            rel: format!("{rel}/{name}"), size: if m.is_dir() { String::new() } else { human(m.len()) }, bytes: m.len(), modified,
            is_dir: m.is_dir(), is_zip: name.to_lowercase().ends_with(".zip"), name,
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    let mut crumbs = vec![];
    let mut acc = String::new();
    for p in rel.split('/') { acc = if acc.is_empty() { p.to_string() } else { format!("{acc}/{p}") }; crumbs.push((p.to_string(), acc.clone())); }
    Ok(Listing { rel, crumbs, entries, roots })
}

/// Nome di file sicuro: niente cartelle, niente caratteri di controllo, mai vuoto.
fn clean_name(name: &str) -> String {
    let n: String = name.rsplit(['/', '\\']).next().unwrap_or("").chars().filter(|c| !c.is_control()).collect();
    let n = n.trim().trim_start_matches('.').to_string();
    if n.is_empty() { "file".into() } else { n }
}

pub fn upload(app: &App, dir: &str, name: &str, bytes: &[u8]) -> R<String> {
    let (path, _) = resolve(app, dir, true)?;
    if !path.is_dir() { return Err("la destinazione non è una cartella".into()) }
    let name = clean_name(name);
    let target = path.join(&name);
    // Se esiste già un collegamento simbolico con quel nome, non lo si segue: si scrive un file normale al suo posto.
    if fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) { fs::remove_file(&target).map_err(|e| e.to_string())?; }
    fs::write(&target, bytes).map_err(|e| e.to_string())?;
    Ok(format!("Caricato {name} ({}).", human(bytes.len() as u64)))
}

pub fn mkdir(app: &App, dir: &str, name: &str) -> R<String> {
    let (path, _) = resolve(app, dir, true)?;
    let name = clean_name(name);
    fs::create_dir(path.join(&name)).map_err(|e| format!("cartella non creata: {e}"))?;
    Ok(format!("Cartella {name} creata."))
}

pub fn delete(app: &App, rel: &str) -> R<String> {
    let (path, clean) = resolve(app, rel, true)?;
    if !clean.contains('/') { return Err("le cartelle principali non si possono eliminare".into()) }
    let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if meta.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) }.map_err(|e| e.to_string())?;
    if !meta.is_dir() {
        // anche le sue versioni compresse, altrimenti il server potrebbe continuare a inviarle
        for ext in ["gz", "br"] { let _ = fs::remove_file(format!("{}.{ext}", path.display())); }
    }
    Ok(format!("Eliminato {}.", clean.rsplit('/').next().unwrap_or(&clean)))
}

pub fn download(app: &App, rel: &str) -> R<(String, Vec<u8>)> {
    let (path, clean) = resolve(app, rel, true)?;
    if !path.is_file() { return Err("non è un file".into()) }
    if path.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE { return Err("file troppo grande per il download dal pannello".into()) }
    Ok((clean.rsplit('/').next().unwrap_or("file").to_string(), fs::read(&path).map_err(|e| e.to_string())?))
}

/// Estrae uno ZIP nella cartella in cui si trova, in modo sicuro.
pub fn extract(app: &App, rel: &str) -> R<String> {
    let (path, clean) = resolve(app, rel, true)?;
    if !path.is_file() || !clean.to_lowercase().ends_with(".zip") { return Err("non è un file ZIP".into()) }
    let dest = path.parent().ok_or("cartella non trovata")?.to_path_buf();
    let data = fs::read(&path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| format!("ZIP non leggibile: {e}"))?;
    if zip.len() > MAX_ENTRIES { return Err(format!("lo ZIP contiene più di {MAX_ENTRIES} file")) }
    let (mut total, mut written, mut skipped) = (0u64, 0usize, 0usize);
    let mut created: Vec<PathBuf> = vec![]; // file e cartelle creati da questa estrazione: si tolgono se va storta
    let mut dirs: Vec<PathBuf> = vec![];
    let undo = |files: &[PathBuf], dirs: &[PathBuf]| {
        for f in files { let _ = fs::remove_file(f); }
        for d in dirs.iter().rev() { let _ = fs::remove_dir(d); } // solo se vuote: i file dell'utente restano
    };
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        // Nome sicuro: `enclosed_name` rifiuta percorsi assoluti e ".."; i link simbolici si saltano.
        let (Some(name), false) = (f.enclosed_name(), f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000)) else { skipped += 1; continue };
        let target = dest.join(&name);
        let track_new_dir = |d: &std::path::Path, dirs: &mut Vec<PathBuf>| {
            let mut stack = vec![];
            let mut cur = d;
            while !cur.exists() { stack.push(cur.to_path_buf()); match cur.parent() { Some(p) => cur = p, None => break } }
            for p in stack.into_iter().rev() { dirs.push(p); }
        };
        if f.is_dir() { track_new_dir(&target, &mut dirs); fs::create_dir_all(&target).map_err(|e| e.to_string())?; continue }
        if let Some(p) = target.parent() { track_new_dir(p, &mut dirs); fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        let mut out = match fs::File::create(&target) { Ok(o) => o, Err(e) => { undo(&created, &dirs); return Err(format!("{}: {e}", name.display())) } };
        created.push(target.clone());
        // Si copiano al massimo (spazio rimasto sotto il tetto totale) + 1 byte: si contano i byte SCRITTI davvero,
        // non la dimensione dichiarata nello ZIP, che chi prepara il file può falsificare (zip bomb).
        let allowed = (MAX_TOTAL - total).min(MAX_FILE) + 1;
        let n = match std::io::copy(&mut (&mut f as &mut dyn Read).take(allowed), &mut out) { Ok(n) => n, Err(e) => { undo(&created, &dirs); return Err(e.to_string()) } };
        total += n;
        if n > MAX_FILE || total > MAX_TOTAL {
            undo(&created, &dirs);
            return Err("lo ZIP, una volta estratto, supererebbe i limiti (300 MB in tutto, 100 MB per file)".into());
        }
        written += 1;
    }
    Ok(format!("Estratti {written} file{}.", if skipped > 0 { format!(", {skipped} saltati perché non sicuri") } else { String::new() }))
}

/// Tipo di contenuto per il download, dall'estensione.
pub fn content_type(name: &str) -> &'static str {
    match Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8", "css" => "text/css", "js" => "text/javascript", "json" => "application/json",
        "xml" => "application/xml", "txt" | "md" => "text/plain; charset=utf-8", "png" => "image/png", "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp", "gif" => "image/gif", "avif" => "image/avif", "svg" => "image/svg+xml", "zip" => "application/zip",
        "woff2" => "font/woff2", "pdf" => "application/pdf", _ => "application/octet-stream",
    }
}
