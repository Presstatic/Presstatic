//! Aggiornamenti dal pannello.
//!
//! Le versioni stanno nelle "Release" di un repository GitHub (vedi `REPO`). Ogni release ha due file:
//! il programma `presstatic-linux-x86_64` e la sua firma `presstatic-linux-x86_64.sig` (Ed25519, 64 byte).
//! Un sito installa un aggiornamento solo se la firma corrisponde alla chiave pubblica compilata nel programma
//! (file PUBLIC_KEY): anche se qualcuno prendesse il controllo del repository, non potrebbe spingere codice ai siti.
//! La chiave privata sta solo sul computer di chi pubblica (`presstatic keygen`, `presstatic sign`).
//!
//! Dopo la sostituzione del file, il processo termina e systemd (Restart=always) lo riavvia con la versione nuova.
//! Il database e la cartella public non vengono toccati: le migrazioni sono applicate all'avvio, come sempre.

use crate::{now, R};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde_json::Value;
use std::{fs, io::Read, path::PathBuf, time::Duration};

/// Repository GitHub con le release, nella forma "utente/repository". Da cambiare una volta sola, prima di pubblicare.
pub const REPO: &str = include_str!("../REPO");
/// Il programma per il processore su cui gira: presstatic-linux-x86_64 o presstatic-linux-aarch64.
fn asset_name() -> String { format!("presstatic-linux-{}", std::env::consts::ARCH) }
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_SIZE: usize = 100 * 1024 * 1024;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub sig_url: String,
    pub checked_at: i64,
}

fn repo() -> String {
    match (crate::test_env("PRESSTATIC_UPDATE_API"), crate::test_env("PRESSTATIC_UPDATE_REPO")) { (Some(_), Some(r)) => r, _ => REPO.trim().to_string() }
}

fn api() -> String { crate::test_env("PRESSTATIC_UPDATE_API").unwrap_or_else(|| "https://api.github.com".into()) }

/// Chiave pubblica per la verifica delle firme (32 byte). Nei test si può passare con PRESSTATIC_UPDATE_KEY.
pub fn public_key() -> Option<Vec<u8>> {
    // La chiave di prova vale solo fuori da systemd (test automatici): in produzione c'è solo quella nel programma.
    let hex = match (crate::test_env("PRESSTATIC_UPDATE_API"), crate::test_env("PRESSTATIC_UPDATE_KEY")) {
        (Some(_), Some(k)) => k,
        _ => include_str!("../PUBLIC_KEY").trim().to_string(),
    };
    let bytes = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>()?;
    (bytes.len() == 32).then_some(bytes)
}

/// Motivo per cui gli aggiornamenti dal pannello non sono disponibili, se c'è.
pub fn unavailable() -> Option<String> {
    if repo().is_empty() || repo().contains("UTENTE") { return Some("Il repository degli aggiornamenti non è impostato (file REPO).".into()) }
    if public_key().is_none() { return Some("La chiave pubblica per verificare gli aggiornamenti non è impostata (file PUBLIC_KEY).".into()) }
    let dir = exe_dir()?;
    let probe = dir.join(".presstatic-write-test");
    if fs::write(&probe, b"").is_err() {
        return Some(format!("Il programma sta in {} e il servizio non può sostituirlo. Rilancia install.sh: mette il programma nella cartella del sito, dove l'aggiornamento è possibile.", dir.display()));
    }
    let _ = fs::remove_file(probe);
    None
}

fn exe_dir() -> Option<PathBuf> { std::env::current_exe().ok()?.parent().map(PathBuf::from) }

fn semver(v: &str) -> (u64, u64, u64) {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|x| x.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

fn download(url: &str) -> R<Vec<u8>> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).redirects(5).build();
    let res = agent.get(url).set("User-Agent", &format!("Presstatic/{VERSION}")).set("Accept", "application/octet-stream").call().map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    let mut reader = res.into_reader();
    std::io::Read::take(&mut *reader, MAX_SIZE as u64 + 1).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    if buf.len() > MAX_SIZE { return Err("file troppo grande".into()) }
    Ok(buf)
}

/// Chiede a GitHub l'ultima release. Restituisce Some solo se è più nuova della versione in esecuzione.
pub fn check() -> R<Option<Release>> {
    let url = format!("{}/repos/{}/releases/latest", api(), repo());
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).redirects(3).build();
    let v: Value = match agent.get(&url).set("User-Agent", &format!("Presstatic/{VERSION}")).set("Accept", "application/vnd.github+json").call() {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(404, _)) => return Ok(None), // nessuna release pubblicata
        Err(e) => return Err(format!("GitHub non raggiungibile: {e}")),
    };
    let tag = v["tag_name"].as_str().unwrap_or("").to_string();
    if semver(&tag) <= semver(VERSION) { return Ok(None) }
    let asset = |name: &str| v["assets"].as_array().into_iter().flatten().find(|a| a["name"] == name).and_then(|a| a["browser_download_url"].as_str()).map(String::from);
    let name = asset_name();
    match (asset(&name), asset(&format!("{name}.sig"))) {
        (Some(url), Some(sig_url)) if url.starts_with("https://") || api() != "https://api.github.com" => Ok(Some(Release {
            version: tag.trim_start_matches('v').to_string(),
            notes: v["body"].as_str().unwrap_or("").chars().take(4000).collect(),
            url, sig_url, checked_at: now(),
        })),
        _ => Err(format!("La release {tag} non contiene i file {name} e {name}.sig")),
    }
}

/// Verifica che la firma copra QUESTA versione, non solo il binario: così un binario vecchio ma firmato
/// non può essere spacciato per una versione nuova (rollback). Il messaggio firmato è "versione\nHEXSHA256".
fn verify_versioned(version: &str, bin: &[u8], sig: &[u8]) -> R<()> {
    use ring::digest;
    let hash: String = digest::digest(&digest::SHA256, bin).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    let msg = format!("{version}\n{hash}");
    let key = public_key().ok_or("chiave pubblica non impostata")?;
    UnparsedPublicKey::new(&ED25519, &key).verify(msg.as_bytes(), sig)
        .map_err(|_| "la firma non copre questa versione (possibile tentativo di installare una versione diversa da quella pubblicata). Aggiornamento rifiutato.".to_string())
}

/// Scarica, verifica e installa la release. Il chiamante fa poi terminare il processo per il riavvio.
pub fn apply(rel: &Release) -> R<String> {
    // Un solo aggiornamento alla volta: due clic o due amministratori non scrivono insieme lo stesso file.
    static BUSY: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = BUSY.try_lock().map_err(|_| "un aggiornamento è già in corso".to_string())?;
    if let Some(why) = unavailable() { return Err(why) }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("cartella del programma non trovata")?;
    let (bin, sig) = (download(&rel.url)?, download(&rel.sig_url)?);
    if sig.len() != 64 { return Err("firma non valida".into()) }
    // La firma deve coprire la versione dichiarata dalla release: blocca sia i file manomessi sia i rollback.
    verify_versioned(&rel.version, &bin, &sig)?;
    let tmp = dir.join(".presstatic.new");
    fs::write(&tmp, &bin).map_err(|e| format!("impossibile scrivere il nuovo programma: {e}"))?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    // Prova generale: il nuovo programma deve avviarsi su questo server e dichiarare la versione attesa.
    let (success, said) = run_version(&tmp).map_err(|e| { let _ = fs::remove_file(&tmp); format!("il nuovo programma non parte su questo server: {e}") })?;
    if !success || !said.split_whitespace().any(|w| w == rel.version) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("il nuovo programma non risponde come previsto ({}). Aggiornamento annullato.", said.trim()));
    }
    let old = dir.join("presstatic.old");
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old).map_err(|e| format!("impossibile mettere da parte il programma attuale: {e}"))?;
    if let Err(e) = fs::rename(&tmp, &exe) {
        let _ = fs::rename(&old, &exe);
        return Err(format!("impossibile installare il nuovo programma: {e}"));
    }
    Ok(format!("Aggiornato dalla versione {VERSION} alla {}. La versione precedente resta in {} per un eventuale ripristino.", rel.version, old.display()))
}

/// Esegue `programma --version` e ne legge l'output, con un tempo massimo di 10 secondi.
/// Usa posix_spawn direttamente invece di std::process::Command: così il programma resta compatibile
/// con glibc 2.34 (Ubuntu 22.04, Debian 12), perché non richiede le funzioni pidfd introdotte in glibc 2.39.
fn run_version(path: &std::path::Path) -> R<(bool, String)> {
    use std::{ffi::CString, os::unix::{ffi::OsStrExt, io::FromRawFd}, time::Instant};
    let prog = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let arg = CString::new("--version").unwrap();
    let argv: [*mut libc::c_char; 3] = [prog.as_ptr() as *mut _, arg.as_ptr() as *mut _, std::ptr::null_mut()];
    let envp: [*mut libc::c_char; 1] = [std::ptr::null_mut()];
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: chiamate POSIX con puntatori validi per tutta la durata della funzione; ogni descrittore viene chiuso.
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 { return Err("pipe non disponibile".into()) }
        let mut fa: libc::posix_spawn_file_actions_t = std::mem::zeroed();
        libc::posix_spawn_file_actions_init(&mut fa);
        libc::posix_spawn_file_actions_adddup2(&mut fa, fds[1], 1);
        libc::posix_spawn_file_actions_addclose(&mut fa, fds[0]);
        let mut pid: libc::pid_t = 0;
        let rc = libc::posix_spawn(&mut pid, prog.as_ptr(), &fa, std::ptr::null(), argv.as_ptr(), envp.as_ptr());
        libc::posix_spawn_file_actions_destroy(&mut fa);
        libc::close(fds[1]);
        let mut reader = std::fs::File::from_raw_fd(fds[0]); // chiuso automaticamente alla fine
        if rc != 0 { return Err(format!("avvio non riuscito (codice {rc})")) }
        let (start, mut status) = (Instant::now(), 0);
        loop {
            let r = libc::waitpid(pid, &mut status, libc::WNOHANG);
            if r == pid { break }
            if r < 0 { return Err("attesa del processo non riuscita".into()) }
            if start.elapsed().as_secs() >= 10 { libc::kill(pid, libc::SIGKILL); libc::waitpid(pid, &mut status, 0); return Err("nessuna risposta entro 10 secondi".into()) }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let mut out = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::Read::take(&mut reader, 4096), &mut out);
        Ok((libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, String::from_utf8_lossy(&out).to_string()))
    }
}

// ---------- strumenti per chi pubblica (riga di comando) ----------

/// `presstatic keygen [file]`: crea la coppia di chiavi per firmare le release.
/// Impronta della chiave pubblica di firma: SHA-256 dei 32 byte della chiave, a gruppi di 4 caratteri.
/// È la stessa che stampa deploy/install.sh (ultimi 32 byte della chiave in formato DER), così si possono confrontare.
pub fn fingerprint() -> Option<String> {
    let k = public_key()?;
    let hex: String = ring::digest::digest(&ring::digest::SHA256, &k).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    Some(hex.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).to_string()).collect::<Vec<_>>().join(" "))
}

pub fn keygen(path: Option<String>) {
    let path = path.unwrap_or_else(|| "presstatic-signing.key".into());
    if fs::metadata(&path).is_ok() { eprintln!("Il file {path} esiste già: non lo sovrascrivo."); std::process::exit(1) }
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).expect("generazione della chiave");
    let kp = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("chiave");
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, pkcs8.as_ref())).expect("scrittura della chiave privata");
    let pub_hex: String = kp.public_key().as_ref().iter().map(|b| format!("{b:02x}")).collect();
    let spki = [b"\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00".as_slice(), kp.public_key().as_ref()].concat();
    println!("Chiave privata salvata in {path}: tienila solo sul tuo computer e fanne una copia al sicuro.\n");
    println!("Chiave pubblica (da mettere nel file PUBLIC_KEY del progetto, poi ricompila):\n{pub_hex}\n");
    println!("Impronta della chiave (pubblicala sul sito di Presstatic, fuori da GitHub, perché chi installa la confronti):\n{}\n", {
        let raw: Vec<u8> = (0..pub_hex.len()).step_by(2).filter_map(|i| u8::from_str_radix(&pub_hex[i..i + 2], 16).ok()).collect();
        let h: String = ring::digest::digest(&ring::digest::SHA256, &raw).as_ref().iter().map(|b| format!("{b:02x}")).collect();
        h.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).to_string()).collect::<Vec<_>>().join(" ")
    });
    println!("La stessa chiave in formato PEM, per la variabile PUBKEY_PEM di deploy/install.sh:");
    println!("-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----", base64_encode(&spki));
}

/// `presstatic sign <file> [chiave]`: scrive `<file>.sig`. Rifiuta una chiave diversa da quella compilata.
pub fn sign(file: &str, key: Option<String>) {
    let key_path = key.unwrap_or_else(|| "presstatic-signing.key".into());
    let pkcs8 = fs::read(&key_path).unwrap_or_else(|e| { eprintln!("Chiave {key_path} non leggibile: {e}"); std::process::exit(1) });
    let kp = Ed25519KeyPair::from_pkcs8(&pkcs8).unwrap_or_else(|_| { eprintln!("Chiave non valida"); std::process::exit(1) });
    if public_key().as_deref() != Some(kp.public_key().as_ref()) {
        eprintln!("Questa chiave non corrisponde alla chiave pubblica compilata nel programma (file PUBLIC_KEY): i siti rifiuterebbero l'aggiornamento.");
        std::process::exit(1);
    }
    let data = fs::read(file).unwrap_or_else(|e| { eprintln!("File {file} non leggibile: {e}"); std::process::exit(1) });
    // Si firma "versione\nsha256(file)": la firma è legata a questa versione, non solo al contenuto.
    use ring::digest;
    let hash: String = digest::digest(&digest::SHA256, &data).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    let msg = format!("{VERSION}\n{hash}");
    fs::write(format!("{file}.sig"), kp.sign(msg.as_bytes()).as_ref()).expect("scrittura della firma");
    println!("Firma scritta in {file}.sig (versione {VERSION}).");
}

fn base64_encode(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(b)
}
