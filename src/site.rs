use crate::{cloudflare, media, now, opt, s, App, Settings, R};
use jiff::{civil::DateTime, tz::TimeZone, Timestamp};
use minijinja::{context, Value as JV};
use rusqlite::{params, Connection, OptionalExtension, ToSql};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::{collections::{BTreeMap, BTreeSet, HashMap}, fs, sync::atomic::{AtomicUsize, Ordering}};

// Font dei temi (licenza SIL OFL), inclusi nel binario e copiati in public/assets/fonts/.
// Il numero di versione è nel nome, così i browser e Cloudflare possono tenerli in cache per sempre.
const FONTS: &[(&str, &[u8])] = &[
    ("newsreader-5.3.woff2", include_bytes!("../assets/fonts/newsreader-5.3.woff2")),
    ("newsreader-italic-5.3.woff2", include_bytes!("../assets/fonts/newsreader-italic-5.3.woff2")),
    ("schibsted-grotesk-5.3.woff2", include_bytes!("../assets/fonts/schibsted-grotesk-5.3.woff2")),
    ("schibsted-grotesk-italic-5.3.woff2", include_bytes!("../assets/fonts/schibsted-grotesk-italic-5.3.woff2")),
    ("unifrakturmaguntia-5.3.woff2", include_bytes!("../assets/fonts/unifrakturmaguntia-5.3.woff2")),
    ("bricolage-grotesque-5.3.woff2", include_bytes!("../assets/fonts/bricolage-grotesque-5.3.woff2")),
];
// Percorsi usati dal sito: nessun articolo o pagina può avere questi slug.
pub const RESERVED: &[&str] = &["admin", "assets", "autori", "category", "cerca", "media", "page", "pagefind", "sitemaps", "tag", "themes"];

#[derive(Serialize, Default, Clone)]
pub struct Post {
    pub id: i64, pub slug: String, pub title: String, pub description: String, pub body: String,
    pub category: String, pub image: String, pub schema_type: String, pub schema_data: String,
    pub status: String, pub published_at: i64, pub updated_at: i64, pub kind: String, pub tags: String,
    pub author_id: i64, pub author: String, pub author_slug: String, pub author_bio: String, pub author_photo: String,
    pub featured: bool, pub link_keywords: String,
    pub categories: String, pub coauthors: String, // categorie aggiuntive (oltre alla principale) e id dei coautori, separati da virgole
    pub comments: bool, // commenti aperti su questo articolo (se i commenti sono attivi nel sito)
}

const COLS: &str = "p.id, p.slug, p.title, p.description, p.body, p.category, p.image, p.schema_type, p.schema_data, p.status, \
    p.published_at, p.updated_at, p.kind, p.tags, COALESCE(p.author_id, 0), COALESCE(u.name, ''), COALESCE(u.slug, ''), COALESCE(u.bio, ''), COALESCE(u.photo, ''), p.featured, p.link_keywords, p.categories, p.coauthors, p.comments";

pub fn query(db: &Connection, body: bool, tail: &str, args: &[&dyn ToSql]) -> Vec<Post> {
    let cols = if body { COLS.to_string() } else { COLS.replace("p.body", "''") };
    let sql = format!("SELECT {cols} FROM posts p LEFT JOIN users u ON u.id = p.author_id {tail}");
    let Ok(mut q) = db.prepare(&sql) else { return vec![] };
    let rows = q.query_map(args, |r| Ok(Post {
        id: r.get(0)?, slug: r.get(1)?, title: r.get(2)?, description: r.get(3)?, body: r.get(4)?,
        category: r.get(5)?, image: r.get(6)?, schema_type: r.get(7)?, schema_data: r.get(8)?,
        status: r.get(9)?, published_at: r.get(10)?, updated_at: r.get(11)?, kind: r.get(12)?, tags: r.get(13)?,
        author_id: r.get(14)?, author: r.get(15)?, author_slug: r.get(16)?, author_bio: r.get(17)?, author_photo: r.get(18)?,
        featured: r.get(19)?, link_keywords: r.get(20)?, categories: r.get(21)?, coauthors: r.get(22)?, comments: r.get(23)?,
    }));
    rows.map(|it| it.filter_map(Result::ok).collect()).unwrap_or_default()
}

pub fn get_post(app: &App, id: i64) -> Option<Post> {
    query(&app.db.lock().unwrap_or_else(|e| e.into_inner()), true, "WHERE p.id = ?1", &[&id]).pop()
}

// ---------- utenti e ruoli ----------

pub const ROLES: &[(&str, &str)] = &[("admin", "Amministratore"), ("editor", "Redattore"), ("author", "Autore"), ("disabled", "Disattivato")];

#[derive(Serialize, Default, Clone)]
pub struct User { pub id: i64, pub name: String, pub email: String, pub role: String, pub bio: String, pub photo: String, pub slug: String }
impl User {
    /// Redattori e amministratori: pubblicano, rivedono il lavoro degli autori, gestiscono le pagine.
    pub fn editor(&self) -> bool { matches!(self.role.as_str(), "admin" | "editor") }
    pub fn admin(&self) -> bool { self.role == "admin" }
    /// Un autore modifica solo i propri articoli, finché non sono pubblicati.
    pub fn can_edit(&self, p: &Post) -> bool { self.editor() || (p.author_id == self.id && p.kind == "post" && p.status != "published") }
}

pub fn get_user(db: &Connection, id: i64) -> Option<User> {
    db.query_row("SELECT id, name, email, role, bio, photo, slug FROM users WHERE id = ?1", [id], |r| Ok(User {
        id: r.get(0)?, name: r.get(1)?, email: r.get(2)?, role: r.get(3)?, bio: r.get(4)?, photo: r.get(5)?, slug: r.get(6)?,
    })).optional().ok().flatten()
}

pub fn unique_user_slug(db: &Connection, name: &str, id: i64) -> String {
    let base = Some(slugify(name)).filter(|x| !x.is_empty()).unwrap_or_else(|| "autore".into());
    (1..).map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
        .find(|c| db.query_row("SELECT 1 FROM users WHERE slug = ?1 AND id <> ?2", params![c, id], |_| Ok(())).optional().ok().flatten().is_none())
        .unwrap()
}

/// Crea o aggiorna un utente. L'amministratore gestisce tutti; ognuno può modificare il proprio profilo.
pub fn save_user(app: &App, me: &User, id: i64, f: HashMap<String, String>, photo: Option<Vec<u8>>) -> R<(i64, String)> {
    if !(me.admin() || me.id == id) || (id == 0 && !me.admin()) { return Err("non hai i permessi per questo profilo".into()) }
    let v = |k: &str| f.get(k).map(|x| x.trim().to_string()).unwrap_or_default();
    let (name, email, pass) = (v("name"), v("email").to_lowercase(), v("password"));
    if name.is_empty() || !email.contains('@') { return Err("servono nome ed email".into()) }
    if name.chars().count() > 100 || email.chars().count() > 254 { return Err("nome o email troppo lunghi".into()) }
    if v("bio").chars().count() > 1000 { return Err("la biografia è troppo lunga (al massimo 1000 caratteri)".into()) }
    if (id == 0 || !pass.is_empty()) && pass.chars().count() < 12 { return Err("la password deve avere almeno 12 caratteri".into()) }
    let old = get_user(&app.db.lock().unwrap_or_else(|e| e.into_inner()), id);
    let mut role = old.as_ref().map(|u| u.role.clone()).unwrap_or_else(|| "author".into());
    if me.admin() && ROLES.iter().any(|r| r.0 == v("role")) {
        if me.id == id && v("role") != "admin" { return Err("non puoi togliere a te stesso il ruolo di amministratore".into()) }
        role = v("role");
    }
    // G) la password si cifra prima di prendere il database: Argon2 è lento di proposito e non deve fermare gli altri
    let hashed = (!pass.is_empty()).then(|| crate::hash_password(&pass));
    let (id, slug) = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        // Controllo fatto qui dentro, con il database in mano: due amministratori che si tolgono il ruolo a vicenda
        // nello stesso istante non possono lasciare il sito senza amministratori.
        let was_admin = db.query_row("SELECT role = 'admin' FROM users WHERE id = ?1", [id], |r| r.get::<_, bool>(0)).unwrap_or(false);
        if was_admin && role != "admin" {
            let others: i64 = db.query_row("SELECT COUNT(*) FROM users WHERE role = 'admin' AND id <> ?1", [id], |r| r.get(0)).map_err(s)?;
            if others == 0 { return Err("deve restare almeno un amministratore: prima dai il ruolo a qualcun altro".into()) }
        }
        // Conflitto: qualcuno ha salvato questo profilo dopo che l'ho aperto.
        if id > 0 && f.get("uv").and_then(|x| x.trim().parse::<i64>().ok()).is_some_and(|seen|
            db.query_row("SELECT version FROM users WHERE id = ?1", [id], |r| r.get::<_, i64>(0)).is_ok_and(|cur| cur != seen)) {
            return Err("nel frattempo qualcun altro ha salvato questo profilo: le tue modifiche non sono state salvate, per non sovrascrivere le sue. Ricarica la pagina e rifalle".into());
        }
        let slug = unique_user_slug(&db, &name, id);
        if id == 0 {
            db.execute("INSERT INTO users(name, email, pass, role, bio, slug) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![name, email, hashed.clone().unwrap_or_default(), role, v("bio"), slug]).map_err(|e| format!("utente non creato ({e}): l'email è forse già usata"))?;
            (db.last_insert_rowid(), slug)
        } else {
            db.execute("UPDATE users SET name = ?1, email = ?2, role = ?3, bio = ?4, slug = ?5, version = version + 1 WHERE id = ?6", params![name, email, role, v("bio"), slug, id]).map_err(s)?;
            if let Some(h) = &hashed { db.execute("UPDATE users SET pass = ?1 WHERE id = ?2", params![h, id]).map_err(s)?; }
            (id, slug)
        }
    };
    let mut note = String::new();
    if let Some(bytes) = photo {
        match media::avatar(&bytes) {
            Ok(webp) => {
                let rel = format!("/media/autori/{slug}-{}.webp", now());
                write(app, &rel, webp)?;
                app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("UPDATE users SET photo = ?1 WHERE id = ?2", params![rel, id]).map_err(s)?;
            }
            Err(e) => note = format!(" Foto non caricata: {e}."),
        }
    }
    // Nome, biografia e foto compaiono negli articoli: si aggiornano le pagine di questo autore (in coda con le pubblicazioni).
    let gen = gen_lock();
    let cx = Cx::load(app);
    let posts = query(&app.db.lock().unwrap_or_else(|e| e.into_inner()), true, "WHERE p.author_id = ?1 AND p.status = 'published' AND p.published_at <= ?2", &[&id, &now()]);
    let mut urls = vec![];
    par(&posts, |p| publish_one(app, &cx, p))?;
    if !posts.is_empty() {
        let mut changed = posts.clone();
        if let Some(o) = old.as_ref().filter(|o| o.slug != slug) {
            urls.push(author_url(&cx.st, &o.slug));
            changed.push(Post { author_slug: o.slug.clone(), ..posts[0].clone() });
        }
        rebuild_lists(app, &cx, Some(&changed))?;
        urls.extend(posts.iter().map(|p| post_url(&cx.st, &p.slug)));
        urls.extend([author_url(&cx.st, &slug), format!("{}/autori/", base(&cx.st))]);
    }
    drop(gen);
    Ok((id, format!("Profilo salvato.{note}{}", cloudflare::purge(&cx.st, &urls))))
}

// ---------- date e indirizzi ----------

pub fn tz(st: &Settings) -> TimeZone { TimeZone::get(opt(st, "timezone", "UTC")).unwrap_or(TimeZone::UTC) }
fn fmt(ts: i64, tz: &TimeZone, f: &str) -> String {
    Timestamp::from_second(ts).map(|t| t.to_zoned(tz.clone()).strftime(f).to_string()).unwrap_or_default()
}
pub fn iso(ts: i64, tz: &TimeZone) -> String { fmt(ts, tz, "%Y-%m-%dT%H:%M:%S%:z") }
pub fn human(ts: i64, tz: &TimeZone) -> String { fmt(ts, tz, "%d/%m/%Y %H:%M") }
pub fn local(ts: i64, tz: &TimeZone) -> String { fmt(ts, tz, "%Y-%m-%dT%H:%M") }
fn parse_local(v: &str, tz: &TimeZone) -> Option<i64> {
    Some(v.parse::<DateTime>().ok()?.to_zoned(tz.clone()).ok()?.timestamp().as_second())
}

pub fn base(st: &Settings) -> &str { opt(st, "base_url", "").trim_end_matches('/') }
pub fn post_url(st: &Settings, slug: &str) -> String { format!("{}/{slug}/", base(st)) }
/// Categoria principale più quelle aggiuntive, senza doppioni.
pub fn post_cats(p: &Post) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for c in std::iter::once(p.category.clone()).chain(tags(&p.categories)) {
        if !c.is_empty() && !v.iter().any(|x| slugify(x) == slugify(&c)) { v.push(c) }
    }
    v
}

/// Le categorie madri di una categoria, dalla più alta (Sport > Calcio > Serie A dà [Sport, Calcio]).
fn ancestors(cx: &Cx, cat: &str) -> Vec<String> {
    let mut chain = vec![];
    let mut cur = cat.to_string();
    for _ in 0..6 { // al massimo 6 livelli, e niente giri infiniti se qualcuno crea un cerchio
        match cx.cats.get(&slugify(&cur)).map(|c| c.1.clone()).filter(|p| !p.is_empty() && slugify(p) != slugify(cat) && !chain.iter().any(|x: &String| slugify(x) == slugify(p))) {
            Some(parent) => { chain.insert(0, parent.clone()); cur = parent }
            None => break,
        }
    }
    chain
}

/// Tutte le pagine di categoria in cui compare un articolo: le sue categorie e le loro madri (come in WordPress).
fn all_cats(cx: &Cx, p: &Post) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for c in post_cats(p) { for x in ancestors(cx, &c).into_iter().chain(std::iter::once(c)) { if !v.iter().any(|y| slugify(y) == slugify(&x)) { v.push(x) } } }
    v
}

/// Coautori di un articolo: (nome, slug, biografia, foto).
fn coauthors(cx: &Cx, p: &Post) -> Vec<(String, String, String, String)> {
    p.coauthors.split(',').filter_map(|x| x.trim().parse::<i64>().ok()).filter(|&id| id != p.author_id).filter_map(|id| cx.people.get(&id).cloned()).filter(|c| !c.1.is_empty()).collect()
}

fn cat_url(st: &Settings, cat: &str) -> String { if slugify(cat).is_empty() { String::new() } else { format!("{}/category/{}/", base(st), slugify(cat)) } }
fn tag_url(st: &Settings, tag: &str) -> String { format!("{}/tag/{}/", base(st), slugify(tag)) }
pub fn author_url(st: &Settings, slug: &str) -> String { format!("{}/autori/{slug}/", base(st)) }
fn abs(st: &Settings, p: &str) -> String { if p.is_empty() || p.starts_with("http") { p.into() } else { format!("{}{p}", base(st)) } }

/// Tag separati da virgola, senza doppioni.
pub fn tags(v: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for t in v.split(',').map(str::trim).filter(|t| !t.is_empty() && !slugify(t).is_empty()) {
        if out.len() >= 20 { break } // al massimo 20 tag: evita che un articolo generi centinaia di pagine-tag
        if !out.iter().any(|o| slugify(o) == slugify(t)) { out.push(t.to_string()) }
    }
    out
}

pub fn slugify(text: &str) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        let c = match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a', 'è' | 'é' | 'ê' | 'ë' => 'e', 'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o', 'ù' | 'ú' | 'û' | 'ü' => 'u', 'ç' => 'c', 'ñ' => 'n', c => c,
        };
        if c.is_ascii_alphanumeric() { out.push(c) } else if !out.is_empty() && !out.ends_with('-') { out.push('-') }
    }
    let out: String = out.chars().take(80).collect();
    out.trim_matches('-').to_string()
}

pub fn esc(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

// ---------- file ----------

/// Scrittura atomica: file temporaneo e poi rename, così il web server non legge mai un file a metà.
fn write(app: &App, rel: &str, content: impl AsRef<[u8]>) -> R<()> {
    let data = content.as_ref();
    let mut path = app.public.join(rel.trim_start_matches('/'));
    if rel.ends_with('/') { path.push("index.html") }
    let packable = compressible(&path);
    if fs::read(&path).is_ok_and(|old| old == data) { // niente di cambiato: nessuna riscrittura
        if packable && !(sibling(&path, "gz").exists() && sibling(&path, "br").exists()) { packer::enqueue(path) }
        return Ok(())
    }
    if let Some(dir) = path.parent() { fs::create_dir_all(dir).map_err(s)? }
    if !packable { return atomic_write(&path, data) }
    {
        // Prima si tolgono le versioni compresse della pagina vecchia, poi si scrive la nuova: il server non invia mai
        // una versione compressa più vecchia della pagina. Quelle nuove le prepara packer, in sottofondo.
        let _one = packer::LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for ext in ["gz", "br"] { let _ = fs::remove_file(sibling(&path, ext)); }
        atomic_write(&path, data)?;
    }
    packer::enqueue(path);
    Ok(())
}

/// Versioni già compresse delle pagine (index.html.gz e index.html.br), che Nginx invia così come sono
/// (gzip_static e brotli_static) invece di comprimere la pagina a ogni richiesta. Le prepara un thread in sottofondo:
/// la pubblicazione e la rigenerazione non aspettano la compressione, e finché la versione compressa non è pronta
/// Nginx comprime al volo come prima.
mod packer {
    use super::{atomic_write, brotli_pack, gzip, sibling};
    use std::{collections::BTreeSet, fs, path::PathBuf, sync::{Mutex, Once}, thread, time::Duration};
    pub static LOCK: Mutex<()> = Mutex::new(());
    static QUEUE: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
    static START: Once = Once::new();

    pub fn enqueue(path: PathBuf) {
        QUEUE.lock().unwrap_or_else(|e| e.into_inner()).insert(path);
        START.call_once(|| { thread::spawn(run); });
    }

    fn run() {
        loop {
            let batch = std::mem::take(&mut *QUEUE.lock().unwrap_or_else(|e| e.into_inner()));
            if batch.is_empty() { thread::sleep(Duration::from_millis(200)); continue }
            for path in batch { pack(&path) }
        }
    }

    fn pack(path: &PathBuf) {
        let Ok(data) = fs::read(path) else { return }; // pagina tolta nel frattempo
        let (gz, br) = (gzip(&data), brotli_pack(&data));
        let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Se la pagina è cambiata mentre si comprimeva, queste versioni sono già vecchie: ci pensa il giro successivo.
        if fs::read(path).ok().as_deref() != Some(&data[..]) { return }
        let _ = atomic_write(&sibling(path, "gz"), &gz);
        let _ = atomic_write(&sibling(path, "br"), &br);
    }
}

/// Scrive in un file temporaneo e poi lo rinomina: chi legge vede sempre il file vecchio o quello nuovo, mai a metà.
fn atomic_write(path: &std::path::Path, data: &[u8]) -> R<()> {
    static TMP: AtomicUsize = AtomicUsize::new(0); // nome unico anche con più thread al lavoro
    let tmp = path.with_extension(format!("tmp{}", TMP.fetch_add(1, Ordering::Relaxed)));
    fs::write(&tmp, data).map_err(s)?;
    fs::rename(&tmp, path).map_err(s)
}

/// Solo i file di testo: immagini e font sono già compressi.
fn compressible(path: &std::path::Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    matches!(ext, "html" | "xml" | "txt" | "css" | "js" | "json" | "svg" | "webmanifest")
}

fn sibling(path: &std::path::Path, ext: &str) -> std::path::PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push("."); name.push(ext);
    std::path::PathBuf::from(name)
}

/// gzip al livello 6: praticamente lo stesso peso del livello massimo, fatto una volta sola per pagina.
fn gzip(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::with_capacity(data.len() / 4), flate2::Compression::new(6));
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// brotli a qualità 5: quasi lo stesso peso della qualità massima (circa 8 KB invece di 7,3 per una home)
/// in circa un millisecondo invece di circa 70.
fn brotli_pack(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 4);
    let params = brotli::enc::BrotliEncoderParams { quality: 5, lgwin: 22, ..Default::default() };
    let _ = brotli::BrotliCompress(&mut &data[..], &mut out, &params);
    out
}

/// Marcatore in ogni cartella-articolo generata dal CMS: distingue le pagine del CMS dai file
/// caricati con il file manager, così eliminare o ripubblicare un articolo non tocca mai quei file.
const OWNED: &str = ".presstatic-page";

/// Vero se `dir` è una cartella-articolo del CMS (o non esiste ancora): allora il CMS può scriverci e cancellarla.
/// Falso se contiene altro (per esempio file caricati dal file manager): va lasciata stare.
pub(crate) fn cms_owned(dir: &std::path::Path) -> bool {
    !dir.exists() || dir.join(OWNED).exists()
}

fn mark_owned(app: &App, slug: &str) {
    let marker = app.public.join(slug).join(OWNED);
    if !marker.exists() { let _ = fs::write(marker, b""); }
}

/// Siti aggiornati da una versione senza marcatori: le cartelle degli articoli pubblicati
/// (quelle che contengono la pagina generata) vengono segnate come del CMS. Si esegue all'avvio.
pub fn mark_existing(app: &App) {
    let slugs: Vec<String> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let Ok(mut q) = db.prepare("SELECT slug FROM posts WHERE status = 'published'") else { return };
        q.query_map([], |r| r.get(0)).map(|it| it.filter_map(Result::ok).collect()).unwrap_or_default()
    };
    for slug in slugs {
        if app.public.join(&slug).join("index.html").is_file() { mark_owned(app, &slug) }
    }
}

fn remove_files(app: &App, slug: &str) {
    if !slug.is_empty() && slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        let dir = app.public.join(slug);
        if cms_owned(&dir) { let _ = fs::remove_dir_all(dir); }
    }
}

/// Salva un'immagine caricata. JPG, PNG e WebP vengono ricodificati e ridimensionati (vedi media.rs);
/// GIF e AVIF restano come sono. Restituisce l'indirizzo dell'immagine principale.
pub fn save_upload(app: &App, name: &str, bytes: &[u8]) -> R<String> {
    let (stem, ext) = name.rsplit_once('.').ok_or("il file non ha un'estensione")?;
    let ext = ext.to_lowercase();
    // Controlla i primi byte: un file .jpg deve essere davvero un'immagine.
    let ok = match ext.as_str() {
        "jpg" | "jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "png" => bytes.starts_with(&[0x89, b'P', b'N', b'G']),
        "gif" => bytes.starts_with(b"GIF8"),
        "webp" => bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        // AVIF: Presstatic non può ricodificarlo e quindi non può togliere i metadati (per esempio la posizione GPS).
        "avif" => return Err("le immagini AVIF non sono accettate perché non si possono ripulire dai metadati: caricala in JPG, PNG o WebP".into()),
        _ => return Err(format!("il formato .{ext} non è ammesso (usa jpg, png, webp o gif)")),
    };
    if !ok { return Err("il contenuto del file non corrisponde a un'immagine".into()) }
    // Nome unico anche se due persone caricano «foto.jpg» nello stesso secondo: istante in millisecondi più un contatore,
    // altrimenti il secondo caricamento sovrascriverebbe le immagini del primo (in un altro articolo).
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or_default();
    let base = format!("/media/{}/{}{:02}-{}", fmt(now(), &TimeZone::UTC, "%Y/%m"), ms, SEQ.fetch_add(1, Ordering::Relaxed) % 100, Some(slugify(stem)).filter(|x| !x.is_empty()).unwrap_or_else(|| "immagine".into()));
    if ext == "gif" {
        if bytes.len() > 5 * 1024 * 1024 { return Err("le GIF possono pesare al massimo 5 MB".into()) }
        // Le GIF restano animate, ma senza commenti e metadati (XMP, ecc.): si tiene solo ciò che serve a disegnarle.
        let clean = media::clean_gif(bytes).ok_or("il file GIF non è valido")?;
        write(app, &format!("{base}.gif"), clean)?;
        return Ok(format!("{base}.gif"));
    }
    let img = media::process(bytes, ext == "png").map_err(|e| format!("immagine non leggibile ({e})"))?;
    for (w, data) in &img.webp { write(app, &format!("{base}-{w}.webp"), data)? }
    let rel = format!("{base}-{}x{}.{}", img.w, img.h, img.ext);
    write(app, &rel, &img.fallback)?;
    register_media(app, &rel, stem, img.w as i64, img.h as i64);
    Ok(rel)
}

// ---------- libreria media ----------

/// Ogni immagine caricata entra nella libreria: nome, misure e, da completare, testo alternativo, didascalia e credito.
pub fn register_media(app: &App, url: &str, name: &str, w: i64, h: i64) {
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute(
        "INSERT OR IGNORE INTO media(url, name, w, h, created_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![url, name, w, h, now()]);
}

/// Testo alternativo, didascalia e credito di un'immagine della libreria (per la foto in evidenza).
pub fn media_meta(app: &App, url: &str) -> Option<(String, String, String)> {
    app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT alt, caption, credit FROM media WHERE url = ?1", [url], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).ok()
}

/// Radice comune delle varianti di un'immagine: /media/2026/09/123-foto-1600x900.jpg → /media/2026/09/123-foto
pub fn media_base(url: &str) -> String {
    let stem = url.rsplit_once('.').map(|x| x.0).unwrap_or(url);
    match stem.rsplit_once('-') { Some((b, wh)) if wh.split_once('x').is_some_and(|(w, h)| w.parse::<u32>().is_ok() && h.parse::<u32>().is_ok()) => b.to_string(), _ => stem.to_string() }
}

/// Miniatura per la libreria: la variante WebP più piccola, se c'è.
pub fn media_thumb(app: &App, url: &str) -> String {
    let base = media_base(url);
    [480, 800].iter().map(|w| format!("{base}-{w}.webp")).find(|t| app.public.join(t.trim_start_matches('/')).exists()).unwrap_or_else(|| url.to_string())
}

/// Articoli e pagine che usano un'immagine (come foto in evidenza o nel testo).
pub fn media_usage(app: &App, id: i64) -> Vec<(i64, String, String)> {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(url) = db.query_row("SELECT url FROM media WHERE id = ?1", [id], |r| r.get::<_, String>(0)) else { return vec![] };
    let like = format!("%{}%", media_base(&url).replace(['%', '_'], ""));
    let Ok(mut q) = db.prepare("SELECT id, title, slug FROM posts WHERE image = ?1 OR body LIKE ?2 ORDER BY published_at DESC LIMIT 50") else { return vec![] };
    let v = q.query_map(params![url, like], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default();
    v
}

/// Elimina l'immagine e tutte le sue misure; rifiuta se è ancora usata.
pub fn media_delete(app: &App, id: i64) -> R<()> {
    let used = media_usage(app, id);
    if let Some((_, t, _)) = used.first() { return Err(format!("è usata in {} articoli o pagine, per esempio «{t}»: toglila prima da lì", used.len())) }
    let url: String = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT url FROM media WHERE id = ?1", [id], |r| r.get(0)).map_err(|_| "immagine non trovata".to_string())?;
    let base = media_base(&url);
    let (dir, prefix) = base.rsplit_once('/').map(|(d, p)| (d.to_string(), format!("{p}-"))).unwrap_or_default();
    for f in fs::read_dir(app.public.join(dir.trim_start_matches('/'))).into_iter().flatten().flatten() {
        let n = f.file_name().to_string_lossy().to_string();
        if n.starts_with(&prefix) || format!("/{}", n) == url.rsplit_once('/').map(|x| format!("/{}", x.1)).unwrap_or_default() { let _ = fs::remove_file(f.path()); }
    }
    let _ = fs::remove_file(app.public.join(url.trim_start_matches('/')));
    app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("DELETE FROM media WHERE id = ?1", [id]).map_err(s)?;
    Ok(())
}

/// Dopo una modifica di didascalia o credito: si rifanno le pagine degli articoli che hanno quella foto in evidenza.
pub fn refresh_featured(app: &App, url: &str) {
    let _gen = gen_lock();
    let cx = Cx::load(app);
    let posts = query(&app.db.lock().unwrap_or_else(|e| e.into_inner()), true, "WHERE p.image = ?1 AND p.status = 'published' AND p.published_at <= ?2", &[&url, &now()]);
    let urls: Vec<String> = posts.iter().filter_map(|p| publish_one(app, &cx, p).ok().map(|_| post_url(&cx.st, &p.slug))).collect();
    drop(_gen);
    if !urls.is_empty() { cloudflare::purge(&cx.st, &urls); }
}

/// Siti aggiornati da una versione precedente: le immagini già presenti entrano nella libreria (una volta sola).
pub fn media_backfill(app: &App) -> usize {
    let mut n = 0;
    let root = app.public.join("media");
    let Ok(years) = fs::read_dir(&root) else { return 0 };
    for y in years.flatten().filter(|e| e.path().is_dir() && e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit())) {
        for m in fs::read_dir(y.path()).into_iter().flatten().flatten().filter(|e| e.path().is_dir()) {
            for f in fs::read_dir(m.path()).into_iter().flatten().flatten() {
                let name = f.file_name().to_string_lossy().to_string();
                let Some((stem, ext)) = name.rsplit_once('.') else { continue };
                if !matches!(ext, "jpg" | "png" | "gif") { continue }
                let (w, h) = stem.rsplit_once('-').and_then(|(_, wh)| wh.split_once('x')).and_then(|(w, h)| Some((w.parse::<i64>().ok()?, h.parse::<i64>().ok()?))).unwrap_or((0, 0));
                if ext != "gif" && w == 0 { continue } // non è l'originale di un caricamento
                let url = format!("/media/{}/{}/{name}", y.file_name().to_string_lossy(), m.file_name().to_string_lossy());
                let label = media_base(&url).rsplit('/').next().unwrap_or("").splitn(2, '-').nth(1).unwrap_or("immagine").to_string();
                register_media(app, &url, &label, w, h);
                n += 1;
            }
        }
    }
    n
}

// ---------- contenuto, annunci e schema ----------

/// Divide l'HTML in testo e tag. Un tag parte da "<" seguito da lettera, "/" o "!" e finisce al primo ">"
/// fuori dalle virgolette: un ">" o un "</p>" dentro un attributo non vengono mai presi per markup.
/// Serve a ritoccare l'HTML (immagini, annunci, link interni) senza mai scrivere dentro un attributo.
fn tokens(html: &str) -> Vec<(bool, &str)> {
    let b = html.as_bytes();
    let (mut out, mut i, mut start) = (vec![], 0, 0);
    while i < b.len() {
        if b[i] == b'<' && b.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic() || *c == b'/' || *c == b'!') {
            if start < i { out.push((false, &html[start..i])) }
            let (mut j, mut quote) = (i + 1, 0u8);
            while j < b.len() {
                let c = b[j];
                if quote != 0 { if c == quote { quote = 0 } } else if c == b'"' || c == b'\'' { quote = c } else if c == b'>' { break }
                j += 1;
            }
            let end = (j + 1).min(b.len());
            out.push((true, &html[i..end]));
            (i, start) = (end, end);
        } else { i += 1 }
    }
    if start < b.len() { out.push((false, &html[start..])) }
    out
}

fn tag_name(t: &str) -> String {
    t.trim_start_matches(['<', '/']).chars().take_while(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase()
}

/// Ripulisce l'HTML dell'editor, rende responsive le immagini, aggiunge i link interni e l'annuncio dopo il paragrafo N.
/// Lavora tag per tag: niente viene mai inserito dentro un attributo.
fn body_html(html: &str, st: &Settings, links: Option<&Linker>, id: i64) -> String {
    let all = tokens(html);
    let mut toks = Vec::with_capacity(all.len());
    let mut k = 0;
    while k < all.len() { // paragrafi vuoti lasciati dall'editor
        if all[k] == (true, "<p>") {
            if all.get(k + 1) == Some(&(true, "</p>")) { k += 2; continue }
            if all.get(k + 1) == Some(&(true, "<br>")) && all.get(k + 2) == Some(&(true, "</p>")) { k += 3; continue }
        }
        toks.push(all[k]);
        k += 1;
    }
    let (ad, n) = (consent_gate(st, opt(st, "ad_inarticle", ""), "ads"), opt(st, "ad_paragraph", "3").parse::<usize>().unwrap_or(0));
    let repeat = opt(st, "ad_repeat", "") == "on";
    let total = toks.iter().filter(|(t, s)| *t && tag_name(s) == "p" && s.starts_with("</")).count();
    let mut out = String::with_capacity(html.len() + 512);
    let (mut no_link, mut embed, mut paras, mut used) = (0usize, 0usize, 0usize, Vec::<i64>::new());
    for (is_tag, s) in toks {
        if !is_tag {
            match links { Some(l) if no_link == 0 && embed == 0 => l.link_text(s, id, &mut used, &mut out), _ => out.push_str(s) }
            continue;
        }
        let (name, closing) = (tag_name(s), s.starts_with("</"));
        let delta = |d: &mut usize| if closing { *d = d.saturating_sub(1) } else if !s.ends_with("/>") { *d += 1 };
        if matches!(name.as_str(), "a" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "figcaption" | "button") { delta(&mut no_link) }
        if matches!(name.as_str(), "div" | "script" | "style" | "iframe") { delta(&mut embed) }
        if name == "h1" { // l'unico H1 della pagina è il titolo
            out += &if closing { "</h2>".to_string() } else { format!("<h2{}", &s[3..]) };
        } else if let Some(rest) = s.strip_prefix("<img src=\"") {
            let (url, after) = rest.split_once('"').unwrap_or((rest, ">"));
            let attrs = if url.starts_with("/media/") { media::img_attrs(url, url) } else { format!("src=\"{url}\"") };
            out += &format!("<img loading=\"lazy\" decoding=\"async\" sizes=\"(min-width: 48rem) 44rem, 100vw\" {attrs}{after}");
        } else if let Some(rest) = s.strip_prefix("<iframe class=\"ql-video\"") {
            out += &format!("<iframe class=\"ql-video\" loading=\"lazy\"{rest}");
        } else {
            out.push_str(s);
        }
        if name == "p" && closing && embed == 0 {
            paras += 1;
            // mai dopo l'ultimo paragrafo
            if !ad.is_empty() && n > 0 && paras < total && (paras == n || (repeat && paras % n == 0)) {
                out += &format!("<div class=\"ad ad-inarticle\" data-pagefind-ignore>{ad}</div>");
            }
        }
    }
    out
}

/// Testo degli autori: tiene la formattazione e i video, toglie script e codice incorporato.
/// Redattori e amministratori possono invece incorporare qualsiasi codice.
/// Nessun link pericoloso nei dati dello schema, anche se salvati da una versione precedente:
/// i campi di tipo "url" devono essere https, e nessun valore può essere un link javascript:, data: o vbscript:.
fn safe_data(app: &App, p: &Post, data: &mut Map<String, Value>) {
    let urls: Vec<&str> = app.schemas[&p.schema_type]["fields"].as_array().into_iter().flatten()
        .filter(|f| f["kind"] == "url").filter_map(|f| f["key"].as_str()).collect();
    data.retain(|k, v| {
        let x = v.as_str().unwrap_or_default().trim().to_ascii_lowercase();
        !(["javascript:", "data:", "vbscript:"].iter().any(|b| x.starts_with(b)) || (urls.contains(&k.as_str()) && !x.starts_with("https://"))
            || (k == "embedUrl" && !VIDEO_HOSTS.iter().any(|h| x.starts_with(h))))
    });
}

/// Unici indirizzi ammessi come video incorporato nel tipo di contenuto Video.
pub const VIDEO_HOSTS: &[&str] = &["https://www.youtube-nocookie.com/embed/", "https://www.youtube.com/embed/", "https://player.vimeo.com/video/"];

pub(crate) fn sanitize(html: &str) -> String {
    ammonia::Builder::default()
        .add_tags(["iframe", "figure", "figcaption"])
        .add_allowed_classes("figure", ["ps-figure"])
        .add_allowed_classes("div", ["ps-gallery", "cols-2", "cols-3", "cols-4"])
        .add_allowed_classes("span", ["credit"])
        .add_tag_attributes("iframe", ["src", "frameborder", "allowfullscreen"])
        .add_allowed_classes("iframe", ["ql-video"])
        .attribute_filter(|el, attr, value| {
            let video = value.starts_with("https://www.youtube-nocookie.com/embed/") || value.starts_with("https://player.vimeo.com/video/");
            (el != "iframe" || attr != "src" || video).then(|| value.into())
        })
        .clean(html).to_string()
}

/// Testo senza tag HTML, per l'indice di ricerca del pannello.
fn plain(html: &str) -> String {
    let (mut text, mut tag) = (String::new(), false);
    for c in html.chars() {
        match c { '<' => tag = true, '>' => { tag = false; text.push(' ') } _ if !tag => text.push(c), _ => {} }
    }
    text.replace("&nbsp;", " ").replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

/// Aggiorna l'indice di ricerca del pannello per un articolo (dentro la transazione del salvataggio).
pub(crate) fn fts_put(db: &Connection, id: i64, title: &str, desc: &str, body: &str) -> R<()> {
    db.execute("DELETE FROM posts_fts WHERE rowid = ?1", [id]).map_err(s)?;
    db.execute("INSERT INTO posts_fts(rowid, title, description, body) VALUES (?1, ?2, ?3, ?4)", params![id, title, desc, plain(body)]).map_err(s)?;
    Ok(())
}

/// Indicizza tutti gli articoli e le pagine (primo avvio con la ricerca nel testo). Restituisce quanti.
pub fn fts_fill(db: &Connection) -> usize {
    let _ = db.execute_batch("BEGIN; DELETE FROM posts_fts;");
    let mut n = 0;
    if let Ok(mut q) = db.prepare("SELECT id, title, description, body FROM posts") {
        let rows: Vec<(i64, String, String, String)> = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default();
        for (id, t, d, b) in rows { if fts_put(db, id, &t, &d, &b).is_ok() { n += 1 } }
    }
    let _ = db.execute_batch("COMMIT");
    n
}

fn words(html: &str) -> usize {
    let (mut text, mut tag) = (String::new(), false);
    for c in html.chars() {
        match c { '<' => tag = true, '>' => { tag = false; text.push(' ') } _ if !tag => text.push(c), _ => {} }
    }
    text.split_whitespace().count()
}

fn org(st: &Settings) -> Value {
    let mut o = json!({"@type": "Organization", "name": opt(st, "site_name", ""), "url": format!("{}/", base(st))});
    let logo = abs(st, opt(st, "logo", ""));
    if !logo.is_empty() { o["logo"] = json!({"@type": "ImageObject", "url": logo}) }
    let same: Vec<Value> = social(st).into_iter().map(|x| x["url"].clone()).collect();
    if !same.is_empty() { o["sameAs"] = json!(same) }
    o
}

fn person(st: &Settings, p: &Post) -> Value {
    let mut a = json!({"@type": "Person", "name": p.author});
    if !p.author_slug.is_empty() { a["url"] = json!(author_url(st, &p.author_slug)) }
    a
}

fn schema(app: &App, st: &Settings, p: &Post, data: &Map<String, Value>, url: &str, tz: &TimeZone) -> Value {
    if p.kind == "page" {
        return json!({"@context": "https://schema.org", "@type": "WebPage", "name": p.title, "description": p.description, "url": url, "publisher": org(st)});
    }
    let d = &app.schemas[&p.schema_type];
    let mut m = if d["base"].is_object() { d["base"].clone() } else { json!({}) };
    m["@context"] = json!("https://schema.org");
    m["@type"] = json!(p.schema_type);
    m[if d["article"].as_bool().unwrap_or(false) { "headline" } else { "name" }] = json!(p.title);
    if !p.description.is_empty() { m["description"] = json!(p.description) }
    let img = abs(st, &p.image);
    if !img.is_empty() { m[d["imageKey"].as_str().unwrap_or("image")] = json!([img]) }
    m[d["dateKey"].as_str().unwrap_or("datePublished")] = json!(iso(p.published_at, tz));
    m["dateModified"] = json!(iso(p.updated_at.max(p.published_at), tz));
    m["author"] = person(st, p);
    m["publisher"] = org(st);
    m["mainEntityOfPage"] = json!(url);
    m["url"] = json!(url);
    let t = tags(&p.tags);
    if !t.is_empty() { m["keywords"] = json!(t.join(", ")) }
    for f in d["fields"].as_array().into_iter().flatten() {
        let key = f["key"].as_str().unwrap_or_default();
        let Some(v) = data.get(key).and_then(Value::as_str) else { continue };
        let lines = || v.lines().map(str::trim).filter(|l| !l.is_empty());
        let val = match f["kind"].as_str() {
            Some("lines") => json!(lines().collect::<Vec<_>>()),
            Some("steps") => json!(lines().map(|l| json!({"@type": "HowToStep", "text": l})).collect::<Vec<_>>()),
            Some("minutes") => match v.parse::<u32>() { Ok(n) => json!(format!("PT{n}M")), _ => continue },
            Some("number") => match v.replace(',', ".").parse::<f64>() { Ok(n) => json!(n), _ => continue },
            _ => json!(v),
        };
        // Chiavi annidate ("offers.price"): si scende solo attraverso oggetti (o valori vuoti che diventano oggetti);
        // se lungo il percorso c'è un testo o un elenco, il campo si salta invece di mandare in panic la generazione.
        let mut cur = &mut m;
        let mut fits = true;
        for part in key.split('.') {
            if !(cur.is_object() || cur.is_null()) { fits = false; break }
            cur = &mut cur[part];
        }
        if fits { *cur = val; }
    }
    prune(&mut m);
    m
}

// Toglie gli oggetti rimasti con il solo "@type" perché il redattore non ha compilato i campi.
fn prune(v: &mut Value) {
    if let Value::Object(o) = v {
        o.values_mut().for_each(prune);
        o.retain(|_, x| !x.as_object().is_some_and(|x| x.len() == 1 && x.contains_key("@type")));
    }
}

// Dati dello schema da mostrare anche nella pagina (Google chiede che siano visibili).
fn facts(app: &App, p: &Post, data: &Map<String, Value>) -> Vec<Value> {
    let fields = app.schemas[&p.schema_type]["fields"].as_array().cloned().unwrap_or_default();
    fields.iter().filter_map(|f| {
        let label = f["show"].as_str()?;
        let v = data.get(f["key"].as_str()?)?.as_str()?;
        let kind = f["kind"].as_str().unwrap_or("");
        let items: Vec<&str> = if matches!(kind, "lines" | "steps") { v.lines().map(str::trim).filter(|l| !l.is_empty()).collect() } else { vec![] };
        let text = if kind == "minutes" { format!("{v} min") } else { v.to_string() };
        Some(json!({"label": label, "text": text, "items": items, "ordered": kind == "steps"}))
    }).collect()
}

fn jsonld(v: &Value) -> JV {
    JV::from_safe_string(format!("<script type=\"application/ld+json\">{}</script>", v.to_string().replace('<', "\\u003c")))
}

// ---------- contesto dei temi ----------

pub fn valid_theme(t: &str) -> bool { !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') }
fn theme(st: &Settings) -> &str { Some(opt(st, "theme", "classico")).filter(|t| valid_theme(t)).unwrap_or("classico") }

// Le impostazioni visibili ai temi, senza le credenziali di Cloudflare.
/// Consenso ai cookie. Con il banner di Presstatic un codice esterno finisce nella pagina dentro un <template> inerte
/// e parte solo dopo il sì del lettore alla sua categoria («stats» statistiche, «ads» pubblicità e profilazione).
/// Senza banner o con una piattaforma esterna (che blocca da sé) il codice resta com'è.
pub fn consent_gate(st: &Settings, code: &str, cat: &str) -> String {
    if code.trim().is_empty() || opt(st, "consent_mode", "") != "native" { return code.to_string() }
    format!("<template data-consent=\"{}\">{code}</template>", if cat == "ads" { "ads" } else { "stats" })
}

fn site_ctx(st: &Settings) -> BTreeMap<String, String> {
    // Elenco esplicito: un tema, anche di terzi, non vede mai chiavi API, token e chiave di Google.
    const PUBLIC: &[&str] = &["site_name", "base_url", "description", "lang", "timezone", "logo", "favicon", "accent", "per_page", "footer",
        "theme", "menu", "footer_columns", "social", "head_scripts", "body_scripts", "ad_head", "ad_top", "ad_inarticle", "ad_paragraph",
        "ad_repeat", "ad_bottom", "related", "links_on", "consent_mode", "consent_gcm", "consent_text", "consent_policy", "consent_cmp", "color_mode", "mode_toggle", "ad_list", "ad_list_every", "ad_sticky"];
    let mut m: BTreeMap<String, String> = st.iter().filter(|(k, _)| PUBLIC.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    if let Some((w, h)) = media::dims(opt(st, "logo", "")) { m.insert("logo_w".into(), w.to_string()); m.insert("logo_h".into(), h.to_string()); }
    for k in ["logo", "favicon"] { if let Some(v) = m.get_mut(k) { *v = abs(st, v) } }
    m.insert("base_url".into(), base(st).into());
    for (k, cat) in [("head_scripts", opt(st, "consent_head_cat", "stats")), ("body_scripts", opt(st, "consent_body_cat", "stats")), ("ad_head", "ads"), ("ad_top", "ads"), ("ad_bottom", "ads"), ("ad_list", "ads"), ("ad_sticky", "ads")] {
        if let Some(v) = m.get_mut(k) { *v = consent_gate(st, v, cat) }
    }
    m
}

/// Una riga di menu o di piè di pagina: "Cronaca" porta alla categoria, "Contatti | /contatti/" a un indirizzo qualsiasi.
fn link(st: &Settings, line: &str) -> Value {
    let (label, url) = match line.split_once('|') { Some((a, b)) => (a.trim(), abs(st, b.trim())), None => (line, cat_url(st, line)) };
    json!({"label": label, "url": url})
}
fn lines(v: &str) -> impl Iterator<Item = &str> { v.lines().map(str::trim).filter(|l| !l.is_empty()) }
pub(crate) fn menu(st: &Settings) -> Vec<Value> { lines(opt(st, "menu", "")).map(|l| link(st, l)).collect() }

/// Profili social: un indirizzo per riga, il nome della rete si ricava dal dominio ("Nome | indirizzo" per sceglierlo).
pub(crate) fn social(st: &Settings) -> Vec<Value> {
    lines(opt(st, "social", "")).filter_map(|l| {
        let (label, url) = l.split_once('|').map(|(a, b)| (a.trim(), b.trim())).unwrap_or(("", l));
        if !url.starts_with("https://") && !url.starts_with("http://") { return None }
        let host = url.split("://").nth(1)?.split('/').next()?.trim_start_matches("www.").trim_start_matches("m.");
        let name = match host {
            "facebook.com" | "fb.com" => "Facebook", "instagram.com" => "Instagram", "x.com" | "twitter.com" => "X",
            "youtube.com" | "youtu.be" => "YouTube", "tiktok.com" => "TikTok", "linkedin.com" => "LinkedIn",
            "t.me" | "telegram.me" => "Telegram", "wa.me" | "whatsapp.com" | "chat.whatsapp.com" => "WhatsApp",
            "threads.net" | "threads.com" => "Threads", "bsky.app" => "Bluesky", h => h,
        };
        Some(json!({"name": if label.is_empty() { name } else { label }, "url": url}))
    }).collect()
}

/// Piè di pagina: colonne di link ("# Titolo" apre una colonna), profili social, note legali su più righe.
fn footer(st: &Settings) -> Value {
    let mut cols: Vec<Value> = vec![];
    for l in lines(opt(st, "footer_columns", "")) {
        if let Some(title) = l.strip_prefix('#') { cols.push(json!({"title": title.trim(), "links": []})); continue }
        if cols.is_empty() { cols.push(json!({"title": "", "links": []})) }
        if let Some(Value::Array(links)) = cols.last_mut().map(|c| &mut c["links"]) { links.push(link(st, l)) }
    }
    if cols.is_empty() && !menu(st).is_empty() { cols.push(json!({"title": "Sezioni", "links": menu(st)})) }
    json!({"columns": cols, "social": social(st), "legal": lines(opt(st, "footer", "")).collect::<Vec<_>>()})
}

/// Tutto ciò che è uguale per ogni pagina di una generazione, preparato una volta sola.
pub struct Cx {
    pub st: Settings, pub tz: TimeZone, base: JV, related: bool, now: i64, links: Option<Linker>,
    cats: HashMap<String, (String, String, String)>, // slug della categoria -> (nome, categoria madre, descrizione)
    people: HashMap<i64, (String, String, String, String)>, // utente -> (nome, slug, biografia, foto), per i coautori
    header: Option<String>, footer: Option<String>, // testata e piè di pagina costruiti con il page builder, se pubblicati
    page_builder: Option<String>, // anteprima nell'editor: il contenuto della pagina singola che si sta costruendo
    forms: HashMap<i64, String>, // moduli pronti: [modulo N] nel testo e widget «Modulo» del builder
    dirette: std::collections::HashSet<i64>, // articoli con una diretta in corso: etichetta «In diretta» negli elenchi
}
impl Cx {
    pub fn new(st: Settings) -> Cx {
        let base = context! { site => site_ctx(&st), menu => menu(&st), footer => footer(&st), theme => theme(&st) };
        // Articoli correlati disattivabili dalle impostazioni: senza, ogni articolo costa una query in meno.
        let related = st.get("related").is_none_or(|v| v == "on");
        Cx { tz: tz(&st), base, related, now: now(), st, links: None, cats: HashMap::new(), people: HashMap::new(), forms: HashMap::new(), dirette: Default::default(), header: None, footer: None, page_builder: None }
    }
    /// Come `new`, più le parole chiave dei link interni (se attivi).
    pub fn load(app: &App) -> Cx {
        let mut cx = Cx::new(app.settings());
        cx.links = Linker::load(app, &cx);
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(mut q) = db.prepare("SELECT name, parent, description FROM categories") {
            cx.cats = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                .map(|r| r.filter_map(Result::ok).map(|(n, p, d)| (slugify(&n), (n, p, d))).collect()).unwrap_or_default();
        }
        if let Ok(mut q) = db.prepare("SELECT id, name, slug, bio, photo FROM users") {
            cx.people = q.query_map([], |r| Ok((r.get::<_, i64>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))
                .map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default();
        }
        cx.forms = crate::forms::all_html(&db, &cx.st);
        cx.dirette = db.prepare("SELECT id FROM posts WHERE diretta = 1").and_then(|mut q| q.query_map([], |r| r.get::<_, i64>(0)).map(|r| r.filter_map(Result::ok).collect())).unwrap_or_default();
        let layouts: Vec<(String, String)> = db.prepare("SELECT name, published FROM layouts WHERE name IN ('header', 'footer') AND published <> ''")
            .and_then(|mut q| q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|r| r.filter_map(Result::ok).collect())).unwrap_or_default();
        drop(db);
        for (name, doc) in layouts {
            let Ok(doc) = serde_json::from_str::<Value>(&doc) else { continue };
            let html = crate::builder::render(&doc, &bctx(&cx, &[], &[], false, &name), &format!("pb-{name}"));
            if name == "header" { cx.header = Some(html) } else { cx.footer = Some(html) }
        }
        cx
    }
}

/// Link interni automatici. Tutte le parole chiave del sito stanno in un unico automa Aho-Corasick:
/// ogni articolo viene letto una volta sola, qualunque sia il numero di parole chiave.
pub struct Linker { ac: aho_corasick::AhoCorasick, targets: Vec<(i64, String)>, max: usize }

/// Opzione attiva salvo che sia stata spenta nelle impostazioni (casella non spuntata).
pub fn flag(st: &Settings, k: &str) -> bool { st.get(k).is_none_or(|v| v == "on") }

pub fn keywords(v: &str) -> Vec<String> {
    let mut out: Vec<String> = v.split(',').map(|k| k.trim().to_lowercase()).filter(|k| (3..=60).contains(&k.chars().count())).collect();
    out.sort();
    out.dedup();
    out.truncate(10);
    out
}

impl Linker {
    fn load(app: &App, cx: &Cx) -> Option<Linker> {
        if !flag(&cx.st, "links_on") { return None }
        let rows: Vec<(i64, String, String)> = {
            let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
            let mut q = db.prepare("SELECT id, slug, link_keywords FROM posts WHERE link_keywords <> '' AND kind = 'post' AND status = 'published' AND published_at <= ?1 ORDER BY published_at ASC, id ASC").ok()?;
            let v = q.query_map([cx.now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).ok()?.filter_map(Result::ok).collect();
            v
        };
        let (mut patterns, mut targets) = (vec![], vec![]);
        for (id, slug, kws) in rows {
            for k in keywords(&kws) {
                if !patterns.contains(&k) { patterns.push(k); targets.push((id, post_url(&cx.st, &slug))) } // la parola resta all'articolo che l'ha usata per primo
            }
        }
        if patterns.is_empty() { return None }
        let ac = aho_corasick::AhoCorasick::builder().ascii_case_insensitive(true).match_kind(aho_corasick::MatchKind::LeftmostLongest).build(&patterns).ok()?;
        Some(Linker { ac, targets, max: opt(&cx.st, "links_max", "3").parse().unwrap_or(3) })
    }

    /// Aggiunge i link in un pezzo di testo: un link per articolo di destinazione, mai verso l'articolo stesso.
    fn link_text(&self, text: &str, self_id: i64, used: &mut Vec<i64>, out: &mut String) {
        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
        // Una corrispondenza dentro un'entità HTML ("&amp;", "&quot;", "&#39;"…) non si tocca: spezzarla rovinerebbe il testo.
        let in_entity = |start: usize| text[..start].rfind('&').is_some_and(|a| !text[a..start].contains([';', ' ', '<', '>']) && start - a <= 10);
        let mut last = 0;
        for m in self.ac.find_iter(text) {
            let (id, url) = &self.targets[m.pattern().as_usize()];
            if used.len() >= self.max || *id == self_id || used.contains(id) || word(text[..m.start()].chars().next_back()) || word(text[m.end()..].chars().next()) || in_entity(m.start()) { continue }
            out.push_str(&text[last..m.start()]);
            *out += &format!("<a href=\"{url}\">{}</a>", &text[m.start()..m.end()]);
            used.push(*id);
            last = m.end();
        }
        out.push_str(&text[last..]);
    }
}

/// Esegue `f` su tutti gli elementi usando tutti i core del server.
pub fn par<T: Sync>(items: &[T], f: impl Fn(&T) -> R<()> + Sync) -> R<()> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(items.len()).max(1);
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads).map(|_| scope.spawn(|| loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= items.len() { return Ok(()) }
            f(&items[i])?;
        })).collect();
        workers.into_iter().try_for_each(|w| w.join().unwrap_or_else(|_| Err("errore interno durante la generazione".into())))
    })
}

const DAYS: [&str; 7] = ["lunedì", "martedì", "mercoledì", "giovedì", "venerdì", "sabato", "domenica"];
const MONTHS: [&str; 12] = ["gennaio", "febbraio", "marzo", "aprile", "maggio", "giugno", "luglio", "agosto", "settembre", "ottobre", "novembre", "dicembre"];
/// "Giovedì 25 settembre 2026"
fn long_date(ts: i64, tz: &TimeZone) -> String {
    let Ok(t) = Timestamp::from_second(ts) else { return String::new() };
    let z = t.to_zoned(tz.clone());
    let day = DAYS[z.weekday().to_monday_zero_offset() as usize];
    format!("{}{} {} {} {}", day[..1].to_uppercase(), &day[1..], z.day(), MONTHS[z.month() as usize - 1], z.year())
}

fn item(cx: &Cx, p: &Post) -> Value {
    let (st, tz) = (&cx.st, &cx.tz);
    let today = fmt(p.published_at, tz, "%Y-%m-%d") == fmt(cx.now, tz, "%Y-%m-%d");
    json!({
        "id": p.id, "title": p.title, "url": post_url(st, &p.slug), "description": p.description, "image": abs(st, &p.image),
        "img": if p.image.is_empty() { String::new() } else { media::img_attrs(&abs(st, &p.image), &p.image) },
        "category": p.category, "cat_url": cat_url(st, &p.category), "date": human(p.published_at, tz), "date_iso": iso(p.published_at, tz), "diretta": cx.dirette.contains(&p.id),
        "time": fmt(p.published_at, tz, if today { "%H:%M" } else { "%d/%m" }), "featured": p.featured,
        "author": p.author, "author_url": if p.author_slug.is_empty() { String::new() } else { author_url(st, &p.author_slug) },
    })
}

/// Rende un modello del tema scelto. Ogni pagina riceve le stesse variabili di base, qualunque sia il tema.
fn page(app: &App, cx: &Cx, file: &str, canonical: &str, meta: Value, extra: JV) -> R<String> {
    let name = format!("{}/{file}", theme(&cx.st));
    let (builder_header, builder_footer) = (cx.header.clone().map(JV::from_safe_string), cx.footer.clone().map(JV::from_safe_string));
    // In merge_maps vince l'ULTIMO dizionario che ha la chiave (così fa il codice di minijinja, anche se il suo commento
    // di esempio dice il contrario): la base comune del sito va per prima, così i dati della pagina la possono sostituire
    // (per esempio «site» senza script di terzi nell'anteprima dell'editor).
    let ctx = minijinja::value::merge_maps([cx.base.clone(), extra, context! { canonical, meta, builder_header, builder_footer }]);
    app.env.read().unwrap_or_else(|e| e.into_inner()).get_template(&name).and_then(|t| t.render(ctx)).map_err(|e| format!("{name}: {e:#}"))
}

/// Articoli correlati letti dal database: per una singola pubblicazione o un'anteprima.
pub fn related(app: &App, cx: &Cx, p: &Post) -> Vec<Value> {
    if !cx.related || p.category.is_empty() || p.kind != "post" { return vec![] }
    query(&app.db.lock().unwrap_or_else(|e| e.into_inner()), false, "WHERE p.kind = 'post' AND p.status = 'published' AND p.published_at <= ?1 AND p.category = ?2 AND p.id <> ?3 ORDER BY p.published_at DESC LIMIT 3", &[&cx.now, &p.category, &p.id])
        .iter().map(|r| item(cx, r)).collect()
}

pub fn render_post(app: &App, cx: &Cx, p: &Post, related: Vec<Value>) -> R<String> {
    let (st, tz) = (&cx.st, &cx.tz);
    let url = post_url(st, &p.slug);
    let cat = cat_url(st, &p.category);
    let mut data: Map<String, Value> = serde_json::from_str(&p.schema_data).unwrap_or_default();
    safe_data(app, p, &mut data);
    let mut crumbs = vec![json!({"@type": "ListItem", "position": 1, "name": opt(st, "site_name", "Home"), "item": format!("{}/", base(st))})];
    if !cat.is_empty() && p.kind == "post" {
        for a in ancestors(cx, &p.category) { crumbs.push(json!({"@type": "ListItem", "position": crumbs.len() + 1, "name": a, "item": cat_url(st, &a)})) }
        crumbs.push(json!({"@type": "ListItem", "position": crumbs.len() + 1, "name": p.category, "item": cat}))
    }
    crumbs.push(json!({"@type": "ListItem", "position": crumbs.len() + 1, "name": p.title}));
    let co = coauthors(cx, p);
    let mut article = schema(app, st, p, &data, &url, tz);
    // Diretta: aggiornamenti dal più recente, dati strutturati LiveBlogPosting (inizio, fine se conclusa, aggiornamenti).
    let (d_state, d_end, ups) = live_data(app, p.id);
    if d_state > 0 && p.kind == "post" {
        article["@type"] = json!("LiveBlogPosting");
        article["coverageStartTime"] = json!(iso(ups.last().map(|u| u.4).unwrap_or(p.published_at).min(p.published_at), tz));
        if d_state == 2 && d_end > 0 { article["coverageEndTime"] = json!(iso(d_end, tz)) }
        article["liveBlogUpdate"] = json!(ups.iter().take(50).map(|(id, title, body, _, at)| json!({"@type": "BlogPosting",
            "headline": if title.is_empty() { body.chars().take(110).collect::<String>() } else { title.clone() }, "articleBody": body,
            "datePublished": iso(*at, tz), "url": format!("{url}#agg-{id}")})).collect::<Vec<_>>());
    }
    let live_block = if p.kind == "post" { live_html(cx, d_state, &ups) } else { String::new() };
    if !co.is_empty() {
        if let Some(first) = article.get("author").cloned() {
            let mut all = vec![first];
            all.extend(co.iter().map(|c| json!({"@type": "Person", "name": c.0, "url": author_url(st, &c.1)})));
            article["author"] = json!(all);
        }
    }
    let ld = json!([article, {"@context": "https://schema.org", "@type": "BreadcrumbList", "itemListElement": crumbs}]);
    let (image, published, modified) = (abs(st, &p.image), iso(p.published_at, tz), iso(p.updated_at.max(p.published_at), tz));
    let description = if p.description.is_empty() { opt(st, "description", "") } else { &p.description };
    let common = context! {
        image_meta => media_meta(app, &p.image).map(|(alt, caption, credit)| context! { alt, caption, credit }),
        post => p, image, image_attrs => if p.image.is_empty() { String::new() } else { media::img_attrs(&image, &p.image) },
        body => JV::from_safe_string(crate::forms::shortcodes(&body_html(&p.body, st, cx.links.as_ref(), p.id), &cx.forms)), jsonld => jsonld(&ld),
    };
    if p.kind == "page" {
        let meta = json!({"title": format!("{} | {}", p.title, opt(st, "site_name", "")), "og_title": p.title, "og_type": "website", "description": description, "image": image});
        let builder = cx.page_builder.clone().or_else(|| builder_single(app, cx, p)).map(JV::from_safe_string);
        return page(app, cx, "page.html", &url, meta, context! { builder, ..common });
    }
    let author = json!({"name": p.author, "url": if p.author_slug.is_empty() { String::new() } else { author_url(st, &p.author_slug) }, "bio": p.author_bio, "photo": abs(st, &p.author_photo)});
    let meta = json!({
        "title": format!("{} | {}", p.title, opt(st, "site_name", "")), "og_title": p.title, "og_type": "article", "description": description,
        "image": image, "published": published, "modified": modified, "section": p.category,
    });
    // "Aggiornato" solo se la modifica è arrivata almeno 10 minuti dopo la pubblicazione.
    let updated = (p.updated_at > p.published_at + 600).then(|| human(p.updated_at, tz));
    page(app, cx, "post.html", &url, meta, context! {
        cat_url => cat, related, author, facts => facts(app, p, &data), sd => data, updated,
        coauthors => co.iter().map(|c| json!({"name": c.0, "url": author_url(st, &c.1)})).collect::<Vec<_>>(),
        comments_html => JV::from_safe_string(comments_html(app, cx, p)),
        live_html => JV::from_safe_string(live_block), diretta => d_state,
        newsletter_html => JV::from_safe_string(crate::newsletter::form_html(st)),
        share_html => JV::from_safe_string(share_html(st, &url, &p.title)),
        push_html => JV::from_safe_string(crate::push::button_html(st)),
        builder => if p.kind == "page" { cx.page_builder.clone().or_else(|| builder_single(app, cx, p)).map(JV::from_safe_string) } else { None },
        tags => tags(&p.tags).iter().map(|t| json!({"name": t, "url": tag_url(st, t)})).collect::<Vec<_>>(),
        date_iso => published, date_human => human(p.published_at, tz), reading => (words(&p.body) + 199) / 200, ..common
    })
}

// ---------- pubblicazione ----------

fn is_live(p: &Post) -> bool { p.status == "published" && p.published_at <= now() }

fn publish_one(app: &App, cx: &Cx, p: &Post) -> R<()> {
    if is_live(p) {
        write(app, &format!("/{}/", p.slug), render_post(app, cx, p, related(app, cx, p))?)?;
        mark_owned(app, &p.slug); // marca la cartella come generata dal CMS
        Ok(())
    } else { remove_files(app, &p.slug); Ok(()) }
}

/// Indirizzi da svuotare in Cloudflare quando cambia un articolo: la pagina e le prime pagine delle liste in cui compare.
fn affected(cx: &Cx, p: &Post) -> Vec<String> {
    let (st, tz) = (&cx.st, &cx.tz);
    let b = base(st);
    let mut urls = vec![post_url(st, &p.slug), format!("{b}/sitemaps/pages.xml")];
    if p.kind == "page" { return urls }
    let mut lists = vec![format!("{b}/")];
    lists.extend(all_cats(cx, p).iter().map(|c| cat_url(st, c)));
    lists.extend(tags(&p.tags).iter().map(|t| tag_url(st, t)));
    if !p.author_slug.is_empty() { lists.push(author_url(st, &p.author_slug)) }
    lists.extend(coauthors(cx, p).iter().map(|c| author_url(st, &c.1)));
    for l in lists.into_iter().filter(|l| !l.is_empty()) { urls.extend([format!("{l}page/2/"), format!("{l}page/3/"), l]) }
    urls.extend(["/feed.xml", "/sitemap.xml", "/news-sitemap.xml"].map(|u| format!("{b}{u}")));
    urls.push(format!("{b}/sitemaps/posts-{}.xml", fmt(p.published_at, tz, "%Y-%m")));
    urls
}

/// Una lista da generare (home, categoria, tag o autore): le posizioni degli articoli in `posts`.
struct List { path: String, heading: String, idx: Vec<usize>, extra: JV, ld: Option<Value> }

/// Rifà solo la home (dopo la pubblicazione di una composizione del builder) e aggiorna Cloudflare.
pub fn rebuild_home(app: &App) -> R<()> {
    let gen = gen_lock();
    let cx = Cx::load(app);
    rebuild_lists(app, &cx, Some(&[Post::default()]))?;
    drop(gen);
    cloudflare::purge(&cx.st, &[format!("{}/", base(&cx.st))]);
    Ok(())
}

/// La home costruita con il page builder e pubblicata, se c'è.
fn builder_home(app: &App, cx: &Cx, posts: &[Post], items: &[Value]) -> Option<String> {
    let doc: Value = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT published FROM layouts WHERE name = 'home'", [], |r| r.get::<_, String>(0)).ok()
        .and_then(|s| serde_json::from_str(&s).ok())?;
    Some(crate::builder::render(&doc, &bctx(cx, posts, items, false, "home"), "pb-home"))
}

/// Contenuto di una pagina singola costruita con il page builder (pubblicato), se c'è.
fn builder_single(app: &App, cx: &Cx, p: &Post) -> Option<String> {
    let doc: Value = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT published FROM layouts WHERE name = ?1", [format!("page:{}", p.id)], |r| r.get::<_, String>(0)).ok()
        .and_then(|s| serde_json::from_str(&s).ok())?;
    Some(crate::builder::render(&doc, &bctx(cx, &[], &[], false, "page"), "pb-page"))
}

/// Rifà una pagina singola (dopo la pubblicazione di una sua composizione) e aggiorna Cloudflare.
pub fn republish_page(app: &App, id: i64) -> R<()> {
    let gen = gen_lock();
    let cx = Cx::load(app);
    let p = get_post(app, id).ok_or("pagina non trovata")?;
    if is_live(&p) { publish_one(app, &cx, &p)? }
    drop(gen);
    cloudflare::purge(&cx.st, &[post_url(&cx.st, &p.slug)]);
    Ok(())
}

/// Impostazioni del sito per l'anteprima dell'editor: senza script di terzi, statistiche e annunci. L'anteprima gira
/// sul dominio del pannello, e quel codice avrebbe i privilegi dell'amministratore (oltre al blocco della CSP).
const THIRD_PARTY: &[&str] = &["head_scripts", "body_scripts", "ad_head", "ad_top", "ad_inarticle", "ad_paragraph", "ad_bottom", "consent_mode", "consent_gcm", "consent_text", "consent_policy", "consent_cmp", "consent_head_cat", "consent_body_cat", "ad_list", "ad_list_every", "ad_sticky"];
fn preview_site(st: &Settings) -> BTreeMap<String, String> {
    let mut clean = st.clone();
    for k in THIRD_PARTY { clean.remove(*k); }
    site_ctx(&clean)
}
/// Pagine singole in anteprima: lo stesso, togliendo il codice di terzi già inserito nella pagina resa.
fn strip_scripts(html: &str, st: &Settings) -> String {
    let mut out = html.to_string();
    for k in THIRD_PARTY { let v = opt(st, k, "").trim(); if v.len() > 3 { out = out.replace(v, ""); } }
    out
}

/// Testo su una riga: caratteri di controllo sostituiti da spazi, spazi ripetuti ridotti a uno.
pub fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split(' ').filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Contesto per il page builder: impostazioni, articoli, sezioni principali del giornale.
fn bctx<'a>(cx: &'a Cx, posts: &'a [Post], items: &'a [Value], editing: bool, name: &'a str) -> crate::builder::Ctx<'a> {
    let mut cats: Vec<(String, String)> = cx.cats.values().filter(|c| c.1.is_empty()).map(|c| (c.0.clone(), cat_url(&cx.st, &c.0))).collect();
    cats.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    crate::builder::Ctx { st: &cx.st, posts, items, editing, newsletter: crate::newsletter::form_html(&cx.st), name, cats, forms: cx.forms.clone() }
}

/// Anteprima per l'editor: pagina intera nel tema (la home), oppure solo il blocco. `name`: home, header o footer.
pub fn builder_preview(app: &App, doc: &Value, whole: bool, name: &str) -> R<String> {
    let mut cx = Cx::load(app);
    let posts = query(&app.db.lock().unwrap_or_else(|e| e.into_inner()), false, "WHERE p.kind = 'post' AND p.status = 'published' AND p.published_at <= ?1 ORDER BY p.published_at DESC LIMIT 500", &[&cx.now]);
    let items: Vec<Value> = posts.iter().map(|p| item(&cx, p)).collect();
    let html = crate::builder::render(doc, &bctx(&cx, &posts, &items, true, if name.starts_with("page:") { "page" } else { name }), "pb-root");
    if !whole { return Ok(html) }
    // Pagina singola: la pagina vera nel tema, con il contenuto costruito al posto del testo.
    if let Some(id) = name.strip_prefix("page:").and_then(|x| x.parse::<i64>().ok()) {
        let p = get_post(app, id).ok_or("pagina non trovata")?;
        cx.page_builder = Some(html);
        return render_post(app, &cx, &p, vec![]).map(|h| strip_scripts(&h, &cx.st));
    }
    let (home, front_v) = match name {
        "home" => (Some(html), Value::Null),
        _ => { if name == "header" { cx.header = Some(html) } else { cx.footer = Some(html) } (builder_home(app, &cx, &posts, &items), front(&cx, &posts, &items)) }
    };
    let site_name = opt(&cx.st, "site_name", "");
    let meta = json!({"title": site_name, "og_title": site_name, "description": opt(&cx.st, "description", ""), "og_type": "website", "image": abs(&cx.st, opt(&cx.st, "logo", ""))});
    let latest: Vec<&Value> = items.iter().take(20).collect();
    page(app, &cx, "list.html", &format!("{}/", base(&cx.st)), meta, context! { posts => latest, heading => "", page => 1, front => front_v, builder => home.map(JV::from_safe_string), site => preview_site(&cx.st) })
}

/// Dati della prima pagina della home: notizie in evidenza, ultime notizie, sezioni per categoria.
fn front(cx: &Cx, posts: &[Post], items: &[Value]) -> Value {
    // In evidenza: prima gli articoli segnati dalla redazione (al massimo 5), poi i più recenti.
    let mut top: Vec<usize> = (0..posts.len()).filter(|&i| posts[i].featured).take(5).collect();
    let rest: Vec<usize> = (0..posts.len()).filter(|i| !top.contains(i)).take(5 - top.len()).collect();
    top.extend(rest);
    top.sort();
    let hero: BTreeSet<usize> = top.iter().copied().collect();
    // Ultime notizie: la colonna degli ultimi articoli accanto all'apertura (fuori da quelli già in evidenza).
    let latest: Vec<usize> = (0..posts.len()).filter(|i| !hero.contains(i)).take(8).collect();
    // Sezioni: le categorie del menu, nell'ordine del menu; senza menu, le cinque più usate.
    // Una sezione può riprendere articoli già elencati tra le ultime notizie (sono solo titoli): così anche un sito
    // con pochi articoli ha sezioni piene, invece di blocchi con un solo articolo e spazi vuoti.
    let mut names: Vec<String> = lines(opt(&cx.st, "menu", "")).filter(|l| !l.contains('|')).map(String::from).collect();
    if names.is_empty() {
        let mut count: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for p in posts.iter().filter(|p| !p.category.is_empty()) { count.entry(slugify(&p.category)).or_insert((p.category.clone(), 0)).1 += 1 }
        let mut v: Vec<_> = count.into_values().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        names = v.into_iter().take(6).map(|x| x.0).collect();
    }
    let mut shown = hero.clone();
    let mut sections: Vec<Value> = vec![];
    for name in &names {
        let slug = slugify(name);
        let idx: Vec<usize> = (0..posts.len()).filter(|i| !hero.contains(i) && slugify(&posts[*i].category) == slug).take(4).collect();
        if idx.len() < 2 { continue } // una sezione con un solo articolo sembra vuota: meglio saltarla
        shown.extend(&idx);
        let label = posts.iter().find(|p| slugify(&p.category) == slug).map_or(name.as_str(), |p| p.category.as_str());
        sections.push(json!({"name": label, "url": cat_url(&cx.st, name), "items": idx.iter().map(|&i| &items[i]).collect::<Vec<_>>()}));
    }
    shown.extend(&latest);
    // Il resto della prima pagina, così nessun articolo resta fuori dalla home prima di pagina 2.
    let per = opt(&cx.st, "per_page", "20").parse::<usize>().unwrap_or(20).max(1);
    let others: Vec<usize> = (0..posts.len().min(per)).filter(|i| !shown.contains(i)).collect();
    let pick = |v: &[usize]| v.iter().map(|&i| &items[i]).collect::<Vec<_>>();
    // Più letti: l'elenco arriva da Google Analytics (vedi update_most_read), qui si trasforma in articoli.
    let most_read: Vec<&Value> = if cx.st.get("most_read_on").is_some_and(|v| v == "on") {
        serde_json::from_str::<Vec<i64>>(opt(&cx.st, "most_read", "[]")).unwrap_or_default().iter()
            .filter_map(|id| posts.iter().position(|p| p.id == *id)).map(|i| &items[i]).collect()
    } else { vec![] };
    json!({
        "top": pick(&top), "latest": pick(&latest), "others": pick(&others), "most_read": most_read,
        "sections": sections, "dateline": long_date(cx.now, &cx.tz), "updated": fmt(cx.now, &cx.tz, "%H:%M"),
    })
}

/// Rigenera le liste, le sitemap, il feed e le altre pagine di servizio.
/// Con `changed` rifà solo le liste in cui compaiono quegli articoli (pubblicazione); senza, tutto (rigenerazione).
pub fn rebuild_lists(app: &App, cx: &Cx, changed: Option<&[Post]>) -> R<()> {
    let (st, tz, t, b) = (&cx.st, &cx.tz, cx.now, base(&cx.st).to_string());
    let (posts, pages) = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        (query(&db, false, "WHERE p.kind = 'post' AND p.status = 'published' AND p.published_at <= ?1 ORDER BY p.published_at DESC", &[&t]),
         query(&db, false, "WHERE p.kind = 'page' AND p.status = 'published' ORDER BY p.title", &[]))
    };
    let items: Vec<Value> = posts.iter().map(|p| item(cx, p)).collect(); // una volta per articolo, riusati in tutte le liste
    let wanted = |kind: &str, key: &str| changed.is_none_or(|ch| ch.iter().any(|p| match kind {
        "cat" => all_cats(cx, p).iter().any(|c| slugify(c) == key), "tag" => tags(&p.tags).iter().any(|t| slugify(t) == key),
        _ => p.author_slug == key || coauthors(cx, p).iter().any(|c| c.1 == key),
    }));
    type Groups = BTreeMap<String, (String, Vec<usize>)>;
    let (mut cats, mut tagged, mut authors): (Groups, Groups, Groups) = Default::default();
    for (i, p) in posts.iter().enumerate() {
        for c in all_cats(cx, p) { cats.entry(slugify(&c)).or_insert_with(|| (c.clone(), vec![])).1.push(i) }
        for tag in tags(&p.tags) { tagged.entry(slugify(&tag)).or_insert_with(|| (tag, vec![])).1.push(i) }
        if !p.author_slug.is_empty() { authors.entry(p.author_slug.clone()).or_insert_with(|| (p.author.clone(), vec![])).1.push(i) }
        for (name, slug, _, _) in coauthors(cx, p) { authors.entry(slug).or_insert_with(|| (name, vec![])).1.push(i) }
    }
    let website = json!({"@context": "https://schema.org", "@type": "WebSite", "name": opt(st, "site_name", ""), "url": format!("{b}/"), "publisher": org(st),
        "potentialAction": {"@type": "SearchAction", "target": format!("{b}/cerca/?q={{search_term_string}}"), "query-input": "required name=search_term_string"}});
    let mut lists = vec![];
    if changed.is_none_or(|ch| !ch.is_empty()) { // la home cambia con ogni articolo, non con le pagine statiche
        // Home composta con il page builder (se pubblicata): sostituisce la prima pagina disegnata dal tema.
        let builder = builder_home(app, cx, &posts, &items).map(JV::from_safe_string);
        lists.push(List { path: "/".into(), heading: String::new(), idx: (0..posts.len()).collect(), extra: context! { front => front(cx, &posts, &items), builder }, ld: Some(website) });
    }
    for (slug, (name, idx)) in &cats {
        if wanted("cat", slug) {
            let (label, desc) = cx.cats.get(slug).map(|c| (c.0.clone(), c.2.clone())).unwrap_or_else(|| (name.clone(), String::new()));
            let parents: Vec<Value> = ancestors(cx, &label).iter().map(|a| json!({"name": a, "url": cat_url(st, a)})).collect();
            let children: Vec<Value> = cx.cats.values().filter(|c| slugify(&c.1) == *slug && cats.contains_key(&slugify(&c.0))).map(|c| json!({"name": c.0, "url": cat_url(st, &c.0)})).collect();
            lists.push(List { path: format!("/category/{slug}/"), heading: label, idx: idx.clone(), extra: context! { cat_desc => desc, parents, children }, ld: None })
        }
    }
    for (slug, (name, idx)) in &tagged {
        if wanted("tag", slug) { lists.push(List { path: format!("/tag/{slug}/"), heading: name.clone(), idx: idx.clone(), extra: context! { label => "Argomento" }, ld: None }) }
    }
    let mut people = vec![];
    for (slug, (name, idx)) in &authors {
        let p = &posts[idx[0]];
        let (bio, photo) = if p.author_slug == *slug { (p.author_bio.clone(), p.author_photo.clone()) }
            else { cx.people.values().find(|u| u.1 == *slug).map(|u| (u.2.clone(), u.3.clone())).unwrap_or_default() };
        let person = json!({"name": name, "bio": bio, "photo": abs(st, &photo), "url": author_url(st, slug), "count": idx.len()});
        if wanted("author", slug) {
            let mut who = json!({"@type": "Person", "name": name, "url": author_url(st, slug)});
            if !bio.is_empty() { who["description"] = json!(bio) }
            if !photo.is_empty() { who["image"] = json!(abs(st, &photo)) }
            lists.push(List { path: format!("/autori/{slug}/"), heading: name.clone(), idx: idx.clone(), extra: context! { author => &person, label => "Autore" },
                ld: Some(json!({"@context": "https://schema.org", "@type": "ProfilePage", "mainEntity": who})) });
        }
        people.push(person);
    }

    // Ogni pagina di ogni lista è un lavoro a sé: le pagine si generano in parallelo.
    let per = opt(st, "per_page", "20").parse::<usize>().unwrap_or(20).max(1);
    let jobs: Vec<(usize, usize)> = lists.iter().enumerate().flat_map(|(l, list)| (1..=list.idx.len().div_ceil(per).max(1)).map(move |pg| (l, pg))).collect();
    let name = opt(st, "site_name", "");
    par(&jobs, |&(l, pg)| {
        let list = &lists[l];
        let (path, heading, home) = (&list.path, list.heading.as_str(), list.path == "/");
        let n = list.idx.len().div_ceil(per).max(1);
        let page_url = |i: usize| if i == 1 { format!("{b}{path}") } else { format!("{b}{path}page/{i}/") };
        let title = match (home, pg) { (true, 1) => name.to_string(), (true, _) => format!("{name} | pagina {pg}"), (_, 1) => format!("{heading} | {name}"), _ => format!("{heading} | {name} | pagina {pg}") };
        let description = if home { opt(st, "description", "").to_string() } else { format!("Gli articoli di {heading} su {name}.") };
        let meta = json!({"title": title, "og_title": if home { name } else { heading }, "description": description, "og_type": "website", "image": abs(st, opt(st, "logo", ""))});
        let chunk: Vec<&Value> = list.idx.iter().skip((pg - 1) * per).take(per).map(|&i| &items[i]).collect();
        let html = page(app, cx, "list.html", &page_url(pg), meta, context! {
            posts => chunk, heading, page => pg, jsonld => if pg == 1 { list.ld.as_ref().map(jsonld) } else { None },
            prev_url => (pg > 1).then(|| page_url(pg - 1)), next_url => (pg < n).then(|| page_url(pg + 1)),
            ..if home && pg > 1 { context! {} } else { list.extra.clone() }
        })?;
        write(app, &if pg == 1 { path.to_string() } else { format!("{path}page/{pg}/") }, html)
    })?;

    let meta = |title: &str, noindex: bool| json!({"title": format!("{title} | {name}"), "description": opt(st, "description", ""), "og_type": "website", "noindex": noindex});
    write(app, "/autori/", page(app, cx, "list.html", &format!("{b}/autori/"), meta("La redazione", false), context! { people, heading => "La redazione", page => 1, posts => Vec::<Value>::new() })?)?;
    let latest: Vec<&Value> = items.iter().take(6).collect();
    write(app, "/404.html", page(app, cx, "list.html", &format!("{b}/"), meta("Pagina non trovata", true), context! { posts => latest, heading => "Pagina non trovata", notfound => true, page => 1 })?)?;
    // Sezioni ancora senza articoli (dal menu o dalla pagina Categorie): hanno comunque la loro pagina, con «Non ci sono
    // ancora articoli pubblicati», così il link del menu non porta a «non trovata». Restano fuori dalla sitemap e con
    // noindex finché non esce il primo articolo: a quel punto la pagina vera, sopra, prende il loro posto.
    for (slug, c) in cx.cats.iter().filter(|(slug, _)| !slug.is_empty() && !cats.contains_key(*slug)) {
        let label = c.0.clone();
        let parents: Vec<Value> = ancestors(cx, &label).iter().map(|a| json!({"name": a, "url": cat_url(st, a)})).collect();
        let html = page(app, cx, "list.html", &cat_url(st, &label), meta(&label, true), context! { posts => Vec::<Value>::new(), heading => label, page => 1, cat_desc => c.2.clone(), parents })?;
        write(app, &format!("/category/{slug}/"), html)?;
    }
    // La pagina di ricerca e i font si scrivono nella rigenerazione completa, e anche in pubblicazione se mancano
    // (per esempio subito dopo l'installazione): così /cerca/ e i font ci sono sempre.
    if changed.is_none() || !app.public.join("cerca/index.html").exists() {
        write(app, "/cerca/", page(app, cx, "search.html", &format!("{b}/cerca/"), meta("Cerca", true), context! {})?)?;
    }
    for (file, bytes) in FONTS {
        if !app.public.join("assets/fonts").join(file).exists() { write(app, &format!("/assets/fonts/{file}"), bytes)? }
    }

    // Sitemap: un file per mese (in pubblicazione solo i mesi toccati), più l'indice e le pagine di elenco.
    let mut months: BTreeMap<String, Vec<&Post>> = BTreeMap::new();
    for p in &posts { months.entry(fmt(p.published_at, tz, "%Y-%m")).or_default().push(p) }
    let touched: BTreeSet<String> = changed.unwrap_or_default().iter().map(|p| fmt(p.published_at, tz, "%Y-%m")).collect();
    let head = r#"<?xml version="1.0" encoding="UTF-8"?>"#;
    let mut index = format!(r#"{head}<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><sitemap><loc>{b}/sitemaps/pages.xml</loc></sitemap>"#);
    for (m, list) in &months {
        if changed.is_none() || touched.contains(m) {
            let mut x = format!(r#"{head}<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1">"#);
            for p in list {
                let img = abs(st, &p.image);
                let img = if img.is_empty() { String::new() } else { format!("<image:image><image:loc>{}</image:loc></image:image>", esc(&img)) };
                x += &format!("<url><loc>{}</loc><lastmod>{}</lastmod>{img}</url>", esc(&post_url(st, &p.slug)), iso(p.updated_at.max(p.published_at), tz));
            }
            write(app, &format!("/sitemaps/posts-{m}.xml"), x + "</urlset>")?;
        }
        let last = list.iter().map(|p| p.updated_at.max(p.published_at)).max().unwrap_or(t);
        index += &format!("<sitemap><loc>{b}/sitemaps/posts-{m}.xml</loc><lastmod>{}</lastmod></sitemap>", iso(last, tz));
    }
    write(app, "/sitemap.xml", index + "</sitemapindex>")?;
    let mut locs = vec![format!("{b}/"), format!("{b}/autori/")];
    locs.extend(cats.keys().map(|c| format!("{b}/category/{c}/")));
    locs.extend(tagged.keys().map(|c| format!("{b}/tag/{c}/")));
    locs.extend(authors.keys().map(|a| author_url(st, a)));
    locs.extend(pages.iter().map(|p| post_url(st, &p.slug)));
    let urls: String = locs.iter().map(|l| format!("<url><loc>{}</loc></url>", esc(l))).collect();
    write(app, "/sitemaps/pages.xml", format!(r#"{head}<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">{urls}</urlset>"#))?;

    // Sitemap Google News: solo articoli delle ultime 48 ore, massimo 1000.
    let (sname, lang) = (esc(name), esc(opt(st, "lang", "it")));
    let mut news = format!(r#"{head}<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:news="http://www.google.com/schemas/sitemap-news/0.9">"#);
    let recent = posts.iter().filter(|p| p.published_at > t - 48 * 3600 && app.schemas[&p.schema_type]["article"].as_bool().unwrap_or(false));
    for p in recent.take(1000) {
        news += &format!("<url><loc>{}</loc><news:news><news:publication><news:name>{sname}</news:name><news:language>{lang}</news:language></news:publication><news:publication_date>{}</news:publication_date><news:title>{}</news:title></news:news></url>",
            esc(&post_url(st, &p.slug)), iso(p.published_at, tz), esc(&p.title));
    }
    write(app, "/news-sitemap.xml", news + "</urlset>")?;

    let mut rss = format!(r#"{head}<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom"><channel><title>{sname}</title><link>{b}/</link><description>{}</description><language>{lang}</language><atom:link href="{b}/feed.xml" rel="self" type="application/rss+xml"/>"#, esc(opt(st, "description", "")));
    for p in posts.iter().take(30) {
        let u = esc(&post_url(st, &p.slug));
        rss += &format!("<item><title>{}</title><link>{u}</link><guid isPermaLink=\"true\">{u}</guid><pubDate>{}</pubDate><description>{}</description></item>",
            esc(&p.title), fmt(p.published_at, tz, "%a, %d %b %Y %H:%M:%S %z"), esc(&p.description));
    }
    write(app, "/feed.xml", rss + "</channel></rss>")?;
    write(app, "/robots.txt", format!("User-agent: *\nAllow: /\n\nSitemap: {b}/sitemap.xml\nSitemap: {b}/news-sitemap.xml\n"))?;
    let ads = opt(st, "ads_txt", "");
    if !ads.is_empty() { write(app, "/ads.txt", format!("{ads}\n"))? }
    app.search_dirty.store(true, Ordering::Relaxed); // l'indice di ricerca si aggiorna al prossimo giro
    Ok(())
}

/// Salva un articolo o una pagina rispettando il ruolo di chi salva, poi aggiorna il sito e la cache.
/// Coda della generazione: pubblicazioni, eliminazioni, articoli programmati, profili e rigenerazione completa
/// scrivono le pagine una alla volta. Senza, due pubblicazioni quasi contemporanee potevano scrivere la home
/// l'una sopra l'altra, e la versione più vecchia poteva vincere.
static GEN: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// File che esiste solo mentre si scrivono pagine: se al riavvio c'è ancora, il programma si era fermato a metà
/// (spegnimento, crash, aggiornamento) e il sito va rigenerato per non lasciare elenchi e pagine disallineati.
pub const GEN_MARK: &str = ".generazione-in-corso";
pub struct Gen(#[allow(dead_code)] std::sync::MutexGuard<'static, ()>);
impl Drop for Gen { fn drop(&mut self) { let _ = fs::remove_file(GEN_MARK); } }
/// Attende che finisca la generazione in corso senza lasciare il segno: serve prima di uscire per un aggiornamento.
pub fn wait_generation() -> std::sync::MutexGuard<'static, ()> { GEN.lock().unwrap_or_else(|e| e.into_inner()) }
pub fn gen_lock() -> Gen {
    let g = GEN.lock().unwrap_or_else(|e| e.into_inner());
    let _ = fs::write(GEN_MARK, b"");
    Gen(g)
}

/// Dopo quanti secondi senza segnale dall'editor il blocco di un articolo scade (come in WordPress).
pub const LOCK_WINDOW: i64 = 150;

/// Chi sta modificando l'articolo, se è un'altra persona e il suo editor ha dato segnale di recente: (nome, secondi fa).
pub fn lock_holder(app: &App, post_id: i64, me: i64) -> Option<(String, i64)> {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    db.query_row("SELECT u.name, l.at FROM post_locks l JOIN users u ON u.id = l.user_id WHERE l.post_id = ?1 AND l.user_id <> ?2 AND l.at > ?3",
        params![post_id, me, now() - LOCK_WINDOW], |r| Ok((r.get::<_, String>(0)?, now() - r.get::<_, i64>(1)?))).optional().ok().flatten()
}

/// L'articolo passa a me (apertura dell'editor o «Subentra»).
pub fn take_lock(app: &App, post_id: i64, me: i64) {
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner())
        .execute("INSERT INTO post_locks(post_id, user_id, at) VALUES (?1, ?2, ?3) ON CONFLICT(post_id) DO UPDATE SET user_id = ?2, at = ?3", params![post_id, me, now()]);
}

/// Segnale dall'editor aperto: rinnova il mio blocco. Se nel frattempo un altro è subentrato, restituisce il suo nome.
pub fn heartbeat(app: &App, post_id: i64, me: i64) -> Option<String> {
    if let Some((name, _)) = lock_holder(app, post_id, me) { return Some(name) }
    take_lock(app, post_id, me);
    None
}

/// Chiudendo l'editor il blocco si libera subito (altrimenti scade da solo).
pub fn release_lock(app: &App, post_id: i64, me: i64) {
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("DELETE FROM post_locks WHERE post_id = ?1 AND user_id = ?2", params![post_id, me]);
}

/// Versione attuale dell'articolo: cresce a ogni salvataggio.
pub fn post_version(app: &App, id: i64) -> i64 {
    app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT version FROM posts WHERE id = ?1", [id], |r| r.get(0)).unwrap_or(0)
}

pub fn save_post(app: &App, me: &User, id: i64, mut f: HashMap<String, String>, upload: Option<(String, Vec<u8>)>) -> R<(i64, String)> {
    // L'immagine caricata si elabora prima di entrare in coda: può richiedere un secondo, e intanto
    // le pubblicazioni degli altri non devono aspettare.
    let uploaded = upload.map(|(name, bytes)| save_upload(app, &name, &bytes));
    let gen = gen_lock(); // una pubblicazione alla volta: vedi GEN
    let cx = Cx::load(app);
    let tz = &cx.tz;
    let old = if id > 0 { Some(get_post(app, id).ok_or("articolo non trovato")?) } else { None };
    if old.as_ref().is_some_and(|o| !me.can_edit(o)) { return Err("non puoi modificare questo articolo".into()) }
    // Ripristino dalla cronologia: cambiano solo titolo, sommario e testo; tutto il resto si prende dall'articolo
    // com'è adesso, letto qui dentro la coda. Così un ripristino non annulla una modifica di categoria o tag
    // salvata da un collega un istante prima.
    if f.remove("_restore").is_some() {
        if let Some(o) = &old { for (k, v) in current_fields(o, tz) { f.entry(k).or_insert(v); } }
    }
    // Conflitto: qualcuno ha salvato l'articolo dopo che l'ho aperto. Invece di sovrascrivere il suo lavoro,
    // la mia versione va nella cronologia e l'articolo resta com'è: si confronta e si ripristina con un clic.
    if let (Some(o), Ok(seen)) = (&old, f.get("version").map(|x| x.trim().parse::<i64>()).unwrap_or(Ok(-1))) {
        let current = post_version(app, o.id);
        if seen >= 0 && seen != current {
            let who: String = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row(
                "SELECT COALESCE(u.name, '') FROM revisions r LEFT JOIN users u ON u.id = r.user_id WHERE r.post_id = ?1 ORDER BY r.id DESC LIMIT 1",
                [o.id], |r| r.get(0)).unwrap_or_default();
            let who = if who.is_empty() { "un'altra persona".to_string() } else { who };
            let g = |k: &str| f.get(k).map(|x| x.trim().to_string()).unwrap_or_default();
            let body = if me.editor() { g("body") } else { sanitize(&g("body")) };
            if body.len() > 2_000_000 { return Err("il testo è troppo lungo".into()) }
            let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
            db.execute("INSERT INTO revisions(post_id, user_id, title, description, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![o.id, me.id, one_line(&g("title")).chars().take(300).collect::<String>(), g("description").chars().take(500).collect::<String>(), body, now()]).map_err(s)?;
            return Err(format!("nel frattempo {who} ha salvato questo articolo, quindi le tue modifiche non sono state applicate per non sovrascrivere le sue. Titolo, sommario e testo che avevi scritto sono nella cronologia qui a destra, in cima: aprili per confrontarli e ripristinarli se servono"));
        }
    }
    let v = |k: &str| f.get(k).map(|x| x.trim().to_string()).unwrap_or_default();
    let mut notes = String::new();
    // L'immagine in evidenza può essere solo un file caricato nel sito (/media/…): niente indirizzi esterni,
    // che sarebbero un modo per tracciare i lettori o mostrare contenuti fuori dal controllo della redazione.
    let mut image = Some(v("image")).filter(|u| u.starts_with("/media/") || u.is_empty())
        .unwrap_or_else(|| old.as_ref().map(|o| o.image.clone()).unwrap_or_default());
    match uploaded { Some(Ok(u)) => image = u, Some(Err(e)) => notes += &format!(" Immagine non caricata: {e}."), None => {} }
    // Titolo su una riga sola: ogni carattere di controllo (a capo, ritorno carrello, tabulazione…) diventa uno spazio.
    // Il titolo finisce anche nell'oggetto delle email di avviso: la libreria delle email lo codifica già in modo sicuro,
    // questo è il secondo livello di difesa contro l'aggiunta di intestazioni (per esempio «Bcc:») con un «a capo».
    let title = one_line(&v("title"));
    if title.is_empty() { return Err("il titolo è obbligatorio".into()) }
    // Tetti di lunghezza: evitano che un autore riempia database e disco con campi enormi (il testo ha già il suo limite più sotto).
    if title.chars().count() > 300 { return Err("il titolo è troppo lungo".into()) }
    for (k, max) in [("description", 500usize), ("category", 100), ("slug", 200), ("tags", 1000), ("link_keywords", 1000)] {
        if v(k).chars().count() > max { return Err(format!("il campo «{k}» è troppo lungo")) }
    }
    for fd in app.schemas[&Some(v("schema_type")).filter(|t| app.schemas.get(t).is_some()).unwrap_or_else(|| "NewsArticle".into())]["fields"].as_array().into_iter().flatten() {
        if let Some(k) = fd["key"].as_str() { if v(&format!("sd_{k}")).chars().count() > 5000 { return Err("un campo dei dati strutturati è troppo lungo".into()) } }
    }
    let kind = match &old { Some(o) => o.kind.clone(), None if v("kind") == "page" && me.editor() => "page".into(), None => "post".into() };
    let wanted = slugify(&if v("slug").is_empty() { title.clone() } else { v("slug") });
    let wanted = if wanted.is_empty() { "articolo".to_string() } else { wanted };
    let ty = Some(v("schema_type")).filter(|t| app.schemas.get(t).is_some()).unwrap_or_else(|| "NewsArticle".into());
    let sd: Map<String, Value> = app.schemas[&ty]["fields"].as_array().into_iter().flatten().filter_map(|fd| {
        let k = fd["key"].as_str()?;
        let val = v(&format!("sd_{k}"));
        let bad_url = fd["kind"] == "url" && (!val.starts_with("https://") || (k == "embedUrl" && !VIDEO_HOSTS.iter().any(|h| val.to_ascii_lowercase().starts_with(h))));
        (!val.is_empty() && !bad_url).then(|| (k.to_string(), json!(val)))
    }).collect();
    // Gli autori non pubblicano: "Pubblica" diventa "Invia in revisione".
    let status = match v("status").as_str() { "published" if me.editor() => "published", "published" | "pending" => "pending", _ => "draft" };
    let author_id = v("author_id").parse::<i64>().ok().filter(|_| me.editor())
        .or(old.as_ref().map(|o| o.author_id).filter(|&a| a > 0)).unwrap_or(me.id);
    // "In evidenza" in home lo decide la redazione.
    let featured = if me.editor() { v("featured") == "on" } else { old.as_ref().is_some_and(|o| o.featured) };
    if v("body").len() > 2_000_000 { return Err("il testo è troppo lungo (al massimo 2 MB; le immagini non contano, sono file a parte)".into()) }
    let body = if me.editor() { v("body") } else { sanitize(&v("body")) };
    let tag_list = tags(&v("tags")).join(", ");
    let link_kw = keywords(&v("link_keywords")).join(", ");
    // Categorie aggiuntive (la principale è «category») e coautori: questi ultimi li decide la redazione.
    let comments_open = if f.contains_key("comments_field") { v("comments") == "on" } else { old.as_ref().is_none_or(|o| o.comments) };
    let primary = v("category");
    let extra: Vec<String> = tags(&v("categories")).into_iter().filter(|c| slugify(c) != slugify(&primary) && c.chars().count() <= 100).take(10).collect();
    let extra = extra.join(", ");
    let coauthors_ids: String = if me.editor() {
        v("coauthors").split(',').filter_map(|x| x.trim().parse::<i64>().ok()).filter(|&id| id != author_id).take(6).map(|x| x.to_string()).collect::<Vec<_>>().join(",")
    } else { old.as_ref().map(|o| o.coauthors.clone()).unwrap_or_default() };

    let id = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        // Articolo, versione nella cronologia e pulizia delle versioni vecchie in un'unica transazione:
        // o si salva tutto, o niente (anche se il server si spegne a metà).
        db.execute_batch("BEGIN IMMEDIATE").map_err(s)?;
        let saved = (|| -> R<i64> {
        // La data la impostano solo redattori e amministratori (per programmare o correggere): un autore non può
        // retrodatare o posticipare i propri articoli. Per lui vale quella già decisa, o l'istante attuale.
        let at = me.editor().then(|| parse_local(&v("published_at"), tz)).flatten()
            .or(old.as_ref().filter(|o| o.status == "published").map(|o| o.published_at)).unwrap_or_else(now);
        // Uno slug è occupato se è riservato, se un altro articolo lo usa, o se in public/ esiste già
        // qualcosa con quel nome che NON è una cartella-articolo del CMS (per esempio file caricati dal file manager):
        // così pubblicare un articolo non sovrascrive né cancella i file dell'amministratore.
        let own = |c: &str| old.as_ref().is_some_and(|o| o.slug == c); // la cartella è già di questo articolo
        let taken = |c: &str| RESERVED.contains(&c)
            || (!own(c) && app.public.join(c).exists() && !cms_owned(&app.public.join(c)))
            || db.query_row("SELECT 1 FROM posts WHERE slug = ?1 AND id <> ?2", params![c, id], |_| Ok(())).optional().ok().flatten().is_some();
        let slug = (1..).map(|n| if n == 1 { wanted.clone() } else { format!("{wanted}-{n}") }).find(|c| !taken(c)).unwrap();
        let (desc, cat, sd) = (v("description"), v("category"), Value::Object(sd).to_string());
        // L'indirizzo nuovo ora è di questo articolo: se era il vecchio indirizzo di un altro, non reindirizza più.
        db.execute("DELETE FROM redirects WHERE slug = ?1", [&slug]).map_err(s)?;
        let id = if old.is_some() {
            db.execute("UPDATE posts SET slug=?1, title=?2, description=?3, body=?4, category=?5, image=?6, schema_type=?7, schema_data=?8, status=?9, published_at=?10, updated_at=?11, tags=?12, author_id=?13, featured=?14, link_keywords=?16, categories=?17, coauthors=?18, comments=?19, version=version+1 WHERE id=?15",
                params![slug, title, desc, body, cat, image, ty, sd, status, at, now(), tag_list, author_id, featured, id, link_kw, extra, coauthors_ids, comments_open]).map_err(s)?;
            id
        } else {
            db.execute("INSERT INTO posts(slug, title, description, body, category, image, schema_type, schema_data, status, published_at, updated_at, tags, author_id, kind, featured, link_keywords, categories, coauthors, comments) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
                params![slug, title, desc, body, cat, image, ty, sd, status, at, now(), tag_list, author_id, kind, featured, link_kw, extra, coauthors_ids, comments_open]).map_err(s)?;
            db.last_insert_rowid()
        };
        fts_put(&db, id, &title, &desc, &body)?;
        for c in std::iter::once(cat.as_str()).chain(extra.split(", ")).filter(|c| !c.is_empty()) { db.execute("INSERT OR IGNORE INTO categories(name) VALUES (?1)", [c]).map_err(s)?; }
        // Il vecchio indirizzo di un articolo già online porta al nuovo invece di dare «pagina non trovata».
        if let Some(o) = old.as_ref().filter(|o| o.slug != slug && is_live(o)) {
            db.execute("INSERT OR REPLACE INTO redirects(slug, post_id) VALUES (?1, ?2)", params![o.slug, id]).map_err(s)?;
        }
        // Cronologia: una versione per ogni salvataggio che cambia il testo, al massimo 50 per articolo.
        let last: Option<(String, String, String)> = db.query_row("SELECT title, description, body FROM revisions WHERE post_id = ?1 ORDER BY id DESC LIMIT 1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(s)?;
        if last != Some((title.clone(), desc.clone(), body.clone())) {
            db.execute("INSERT INTO revisions(post_id, user_id, title, description, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![id, me.id, title, desc, body, now()]).map_err(s)?;
            db.execute("DELETE FROM revisions WHERE post_id = ?1 AND id NOT IN (SELECT id FROM revisions WHERE post_id = ?1 ORDER BY id DESC LIMIT 50)", [id]).map_err(s)?;
        }
        Ok(id)
        })();
        match saved {
            Ok(id) => { db.execute_batch("COMMIT").map_err(s)?; id }
            Err(e) => { let _ = db.execute_batch("ROLLBACK"); return Err(e) }
        }
    };
    let p = get_post(app, id).ok_or("articolo non trovato dopo il salvataggio")?;
    let cx = Cx::load(app); // ricarica le parole chiave dei link interni, comprese quelle appena salvate
    if let Some(o) = old.as_ref().filter(|o| o.slug != p.slug) { remove_files(app, &o.slug) } // il vecchio indirizzo sparisce
    publish_one(app, &cx, &p)?;
    let moved_urls = write_redirects(app, &cx, &p); // …e al suo posto c'è il reindirizzamento, se l'articolo è online
    // Link interni: se cambiano le parole chiave (o l'articolo entra o esce dal sito), si rifanno gli articoli che le contengono.
    let (was_live, live) = (old.as_ref().is_some_and(is_live), is_live(&p));
    let old_kw: BTreeSet<String> = old.as_ref().map(|o| keywords(&o.link_keywords)).unwrap_or_default().into_iter().collect();
    let new_kw: BTreeSet<String> = keywords(&p.link_keywords).into_iter().collect();
    let moved = was_live != live || old.as_ref().is_some_and(|o| o.slug != p.slug);
    let kws: Vec<String> = if moved { old_kw.union(&new_kw).cloned().collect() } else { old_kw.symmetric_difference(&new_kw).cloned().collect() };
    let linked = if flag(&cx.st, "links_on") && (live || was_live) { containing(app, &kws, p.id) } else { vec![] };
    par(&linked, |q| publish_one(app, &cx, q))?;
    let changed: Vec<Post> = old.iter().cloned().chain([p.clone()]).collect();
    // Una pagina statica non compare in home né nelle liste: basta rifare sitemap e pagine di servizio.
    let changed: &[Post] = if changed.iter().any(|x| x.kind == "post") { &changed } else { &[] };
    rebuild_lists(app, &cx, Some(changed))?;
    let mut urls = affected(&cx, &p);
    if let Some(o) = &old { urls.extend(affected(&cx, o)) }
    urls.extend(linked.iter().map(|q| post_url(&cx.st, &q.slug)));
    urls.extend(moved_urls);
    // Google Indexing API: nuovo articolo, modifica (se attivo nelle impostazioni) o rimozione.
    let mut google = vec![];
    // Vecchio indirizzo: «tolto» se l'articolo non è più online, «aggiornato» se ora reindirizza al nuovo.
    if let Some(o) = old.as_ref().filter(|o| was_live && (!live || o.slug != p.slug)) { google.push((post_url(&cx.st, &o.slug), !live)) }
    if live && !was_live { push_new(app, &cx.st, &p) } // notifica push: solo alla prima uscita, non a ogni correzione
    if live && (!was_live || opt(&cx.st, "google_updates", "") == "on" || old.as_ref().is_some_and(|o| o.slug != p.slug)) { google.push((post_url(&cx.st, &p.slug), false)) }
    // IndexNow: niente quote da risparmiare, quindi anche ogni modifica (e l'eventuale vecchio indirizzo).
    let mut indexnow: Vec<String> = vec![];
    if live || was_live { indexnow.push(post_url(&cx.st, &p.slug)) }
    if let Some(o) = old.as_ref().filter(|o| was_live && o.slug != p.slug) { indexnow.push(post_url(&cx.st, &o.slug)) }
    drop(gen); // pagine scritte: la prossima pubblicazione può partire mentre si avvisano Cloudflare, Google e IndexNow
    // Cloudflare, Google e IndexNow insieme, in parallelo: la pubblicazione aspetta il più lento, non la somma.
    let (cf, g, ix) = std::thread::scope(|sc| {
        let g = sc.spawn(|| crate::google::notify(app, &google));
        let ix = sc.spawn(|| crate::indexnow::submit(app, &indexnow));
        let cf = if live || was_live { cloudflare::purge(&cx.st, &urls) } else { String::new() };
        (cf, g.join().unwrap_or_default(), ix.join().unwrap_or_default())
    });
    let cf = cf + &g + &ix;
    let msg = match (p.status.as_str(), p.published_at > now()) {
        ("draft", _) => "Bozza salvata.".to_string(),
        ("pending", _) => "Inviato in revisione: un redattore lo controllerà e lo pubblicherà.".to_string(),
        (_, true) => format!("Programmato per il {}.", human(p.published_at, tz)),
        _ => if kind == "page" { "Pagina pubblicata.".into() } else { "Articolo pubblicato.".into() },
    };
    Ok((id, format!("{msg}{cf}{notes}")))
}

/// Primo indirizzo libero a partire da quello desiderato (stesse regole del salvataggio).
pub fn free_slug(app: &App, db: &Connection, wanted: &str) -> String {
    let wanted = if wanted.is_empty() { "articolo".to_string() } else { wanted.to_string() };
    let taken = |c: &str| RESERVED.contains(&c) || (app.public.join(c).exists() && !cms_owned(&app.public.join(c)))
        || db.query_row("SELECT 1 FROM posts WHERE slug = ?1", [c], |_| Ok(())).is_ok();
    (1..).map(|n| if n == 1 { wanted.clone() } else { format!("{wanted}-{n}") }).find(|c| !taken(c)).unwrap()
}

/// Vecchi indirizzi di un articolo.
fn redirect_slugs(app: &App, post_id: i64) -> Vec<String> {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(mut q) = db.prepare("SELECT slug FROM redirects WHERE post_id = ?1") else { return vec![] };
    let v = q.query_map([post_id], |r| r.get(0)).map(|rows| rows.filter_map(Result::ok).collect()).unwrap_or_default();
    v
}

/// Scrive, nei vecchi indirizzi dell'articolo, una pagina che porta subito a quello nuovo (reindirizzamento
/// immediato: Google lo tratta come permanente) con il canonico verso il nuovo indirizzo. Se l'articolo non è
/// online, i vecchi indirizzi tornano «pagina non trovata». Restituisce gli indirizzi toccati, per Cloudflare.
fn write_redirects(app: &App, cx: &Cx, p: &Post) -> Vec<String> {
    let target = post_url(&cx.st, &p.slug);
    redirect_slugs(app, p.id).into_iter().filter(|slug| *slug != p.slug).map(|slug| {
        if is_live(p) { redirect_page(app, cx, &slug, &p.title, &target) } else { remove_files(app, &slug) }
        post_url(&cx.st, &slug)
    }).collect()
}

/// Vecchi indirizzi che portano alla home (articoli eliminati scegliendo «alla home»).
fn write_home_redirects(app: &App, cx: &Cx) -> Vec<String> {
    let (target, name) = (format!("{}/", base(&cx.st)), opt(&cx.st, "site_name", "Home").to_string());
    redirect_slugs(app, 0).into_iter().map(|slug| { redirect_page(app, cx, &slug, &name, &target); post_url(&cx.st, &slug) }).collect()
}

/// La pagina nel vecchio indirizzo: porta subito alla destinazione, che è anche il suo indirizzo canonico.
fn redirect_page(app: &App, cx: &Cx, slug: &str, title: &str, target: &str) {
    let (t, u) = (esc(title), esc(target));
    let js = serde_json::to_string(target).unwrap_or_default().replace('<', "\\u003c");
    let html = format!("<!doctype html>\n<html lang=\"{}\"><head><meta charset=\"utf-8\"><title>{t}</title>\n<link rel=\"canonical\" href=\"{u}\">\n<meta http-equiv=\"refresh\" content=\"0; url={u}\">\n<script>location.replace({js})</script></head>\n<body><p>Questa pagina si trova ora qui: <a href=\"{u}\">{t}</a></p></body></html>\n", esc(opt(&cx.st, "lang", "it")));
    if write(app, &format!("/{slug}/"), html).is_ok() { mark_owned(app, slug) }
}

/// Esegue `f` in un'unica transazione: o tutto o niente.
fn in_tx<T>(db: &Connection, f: impl FnOnce(&Connection) -> R<T>) -> R<T> {
    db.execute_batch("BEGIN IMMEDIATE").map_err(s)?;
    match f(db) {
        Ok(v) => { db.execute_batch("COMMIT").map_err(s)?; Ok(v) }
        Err(e) => { let _ = db.execute_batch("ROLLBACK"); Err(e) }
    }
}

/// Tutti i campi del modulo di un articolo come sono adesso (per il ripristino di una versione).
fn current_fields(p: &Post, tz: &TimeZone) -> HashMap<String, String> {
    let mut f: HashMap<String, String> = [
        ("category", p.category.clone()), ("tags", p.tags.clone()), ("image", p.image.clone()), ("slug", p.slug.clone()),
        ("schema_type", p.schema_type.clone()), ("status", p.status.clone()), ("published_at", local(p.published_at, tz)),
        ("author_id", p.author_id.to_string()), ("featured", if p.featured { "on" } else { "" }.into()), ("link_keywords", p.link_keywords.clone()),
        ("categories", p.categories.clone()), ("coauthors", p.coauthors.clone()),
        ("comments_field", "1".into()), ("comments", if p.comments { "on" } else { "" }.into()),
    ].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    let sd: Map<String, Value> = serde_json::from_str(&p.schema_data).unwrap_or_default();
    for (k, v) in sd { f.insert(format!("sd_{k}"), v.as_str().unwrap_or_default().to_string()); }
    f
}

/// Ripristina una versione precedente: il testo torna quello di allora, il resto dell'articolo resta com'è.
pub fn restore_revision(app: &App, me: &User, rev: i64) -> R<(i64, String)> {
    let (post_id, title, desc, body): (i64, String, String, String) = app.db.lock().unwrap_or_else(|e| e.into_inner())
        .query_row("SELECT post_id, title, description, body FROM revisions WHERE id = ?1", [rev], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(|_| "versione non trovata".to_string())?;
    let f: HashMap<String, String> = [("title", title), ("description", desc), ("body", body), ("_restore", "1".into())]
        .into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    save_post(app, me, post_id, f, None).map(|(id, m)| (id, format!("Versione ripristinata. {m}")))
}

/// Articoli pubblicati che contengono almeno una delle parole chiave (al massimo 1000).
fn containing(app: &App, kws: &[String], except: i64) -> Vec<Post> {
    if kws.is_empty() { return vec![] }
    let like: Vec<String> = kws.iter().map(|k| format!("%{}%", k.replace(['%', '_'], ""))).collect();
    let cond = (0..like.len()).map(|i| format!("p.body LIKE ?{}", i + 3)).collect::<Vec<_>>().join(" OR ");
    let mut args: Vec<&dyn ToSql> = vec![&except];
    let t = now();
    args.push(&t);
    for l in &like { args.push(l) }
    query(&app.db.lock().unwrap_or_else(|e| e.into_inner()), true, &format!("WHERE p.id <> ?1 AND p.kind = 'post' AND p.status = 'published' AND p.published_at <= ?2 AND ({cond}) LIMIT 1000"), &args)
}

/// Elimina un articolo o una pagina. `to`: dove porta il suo indirizzo da ora in poi.
/// None = «pagina non trovata»; Some(0) = home; Some(id) = un altro articolo o pagina online.
pub fn delete_post(app: &App, me: &User, id: i64, to: Option<i64>) -> R<String> {
    let gen = gen_lock();
    let cx = Cx::load(app);
    let Some(p) = get_post(app, id) else { return Ok("L'articolo non esiste più.".into()) };
    if !me.can_edit(&p) { return Err("non puoi eliminare questo articolo".into()) }
    // Il reindirizzamento serve solo se l'indirizzo era pubblico, e solo verso qualcosa di online.
    let live = is_live(&p);
    let to = to.filter(|_| live);
    let target = match to {
        Some(t) if t > 0 => Some(get_post(app, t).filter(|q| q.id != id && is_live(q))
            .ok_or("come destinazione scegli un articolo o una pagina pubblicati, diversi da quello che elimini")?),
        _ => None,
    };
    let incoming = redirect_slugs(app, id); // vecchi indirizzi che portavano a questo articolo
    in_tx(&app.db.lock().unwrap_or_else(|e| e.into_inner()), |db| {
        db.execute("DELETE FROM posts WHERE id = ?1", [id]).map_err(s)?;
        db.execute("DELETE FROM revisions WHERE post_id = ?1", [id]).map_err(s)?;
        db.execute("DELETE FROM post_locks WHERE post_id = ?1", [id]).map_err(s)?;
        db.execute("DELETE FROM posts_fts WHERE rowid = ?1", [id]).map_err(s)?;
        db.execute("DELETE FROM post_notes WHERE post_id = ?1", [id]).map_err(s)?;
        match to {
            // anche i suoi vecchi indirizzi passano alla nuova destinazione: nessun reindirizzamento resta a vuoto
            Some(t) => {
                db.execute("UPDATE redirects SET post_id = ?1 WHERE post_id = ?2", params![t, id]).map_err(s)?;
                db.execute("INSERT OR REPLACE INTO redirects(slug, post_id) VALUES (?1, ?2)", params![p.slug, t]).map_err(s)?;
            }
            None => { db.execute("DELETE FROM redirects WHERE post_id = ?1", [id]).map_err(s)?; }
        }
        Ok(())
    })?;
    remove_files(app, &p.slug);
    let mut urls: Vec<String> = match (to, &target) {
        (Some(0), _) => write_home_redirects(app, &cx),
        (Some(_), Some(t)) => write_redirects(app, &cx, t),
        _ => incoming.iter().map(|slug| { remove_files(app, slug); post_url(&cx.st, slug) }).collect(),
    };
    rebuild_lists(app, &cx, Some(std::slice::from_ref(&p)))?;
    drop(gen);
    let cf = if live {
        urls.extend(affected(&cx, &p));
        // Google: «tolto» se l'indirizzo non porta più da nessuna parte, «aggiornato» se ora reindirizza.
        cloudflare::purge(&cx.st, &urls) + &crate::google::notify(app, &[(post_url(&cx.st, &p.slug), to.is_none())]) + &crate::indexnow::submit(app, &[post_url(&cx.st, &p.slug)])
    } else { String::new() };
    let what = if p.kind == "page" { "Pagina eliminata." } else { "Articolo eliminato." };
    let where_to = match (to, &target) {
        (Some(0), _) => " Chi arriva al suo indirizzo viene portato alla home.".to_string(),
        (Some(_), Some(t)) => format!(" Chi arriva al suo indirizzo viene portato a «{}».", t.title),
        _ => String::new(),
    };
    Ok(format!("{what}{where_to}{cf}"))
}

/// Azioni in blocco dall'elenco: tutti gli articoli scelti cambiano insieme, in un'unica transazione,
/// e il sito si aggiorna una volta sola (non una rigenerazione degli elenchi per ogni articolo).
pub fn bulk(app: &App, me: &User, ids: &[i64], action: &str, value: &str) -> R<String> {
    if ids.is_empty() { return Err("scegli almeno un articolo".into()) }
    if ids.len() > 200 { return Err("al massimo 200 articoli alla volta".into()) }
    if !me.editor() { return Err("le azioni in blocco sono per redattori e amministratori".into()) }
    if action == "delete" {
        let n = ids.iter().filter(|&&id| delete_post(app, me, id, None).is_ok()).count();
        return Ok(format!("{n} eliminati."));
    }
    let value = value.trim();
    if matches!(action, "category" | "tag") && (value.is_empty() || value.chars().count() > 100) { return Err("scrivi la categoria o il tag (al massimo 100 caratteri)".into()) }
    let gen = gen_lock();
    let cx = Cx::load(app);
    let olds: Vec<Post> = ids.iter().filter_map(|&id| get_post(app, id)).filter(|p| me.can_edit(p)).collect();
    in_tx(&app.db.lock().unwrap_or_else(|e| e.into_inner()), |db| {
        for p in &olds {
            let r = match action {
                // chi era programmato resta programmato; bozze e articoli in revisione escono adesso
                "publish" => db.execute("UPDATE posts SET status = 'published', published_at = CASE WHEN status = 'published' THEN published_at ELSE ?2 END, updated_at = ?2, version = version + 1 WHERE id = ?1", params![p.id, now()]),
                "draft" => db.execute("UPDATE posts SET status = 'draft', updated_at = ?2, version = version + 1 WHERE id = ?1", params![p.id, now()]),
                "feature" | "unfeature" => db.execute("UPDATE posts SET featured = ?2, version = version + 1 WHERE id = ?1", params![p.id, action == "feature"]),
                "category" => {
                    db.execute("INSERT OR IGNORE INTO categories(name) VALUES (?1)", [value]).map_err(s)?;
                    db.execute("UPDATE posts SET category = ?2, updated_at = ?3, version = version + 1 WHERE id = ?1", params![p.id, value, now()])
                }
                "tag" => {
                    let mut t = tags(&p.tags);
                    if !t.iter().any(|x| x.eq_ignore_ascii_case(value)) { t.push(value.to_string()) }
                    db.execute("UPDATE posts SET tags = ?2, updated_at = ?3, version = version + 1 WHERE id = ?1", params![p.id, t.join(", "), now()])
                }
                _ => return Err("azione sconosciuta".into()),
            };
            r.map_err(s)?;
        }
        Ok(())
    })?;
    let news: Vec<Post> = olds.iter().filter_map(|p| get_post(app, p.id)).collect();
    let (mut urls, mut google) = (vec![], vec![]);
    for (o, n) in olds.iter().zip(&news) {
        if is_live(n) { publish_one(app, &cx, n)?; if !is_live(o) { google.push((post_url(&cx.st, &n.slug), false)) } }
        else if is_live(o) { remove_files(app, &o.slug); google.push((post_url(&cx.st, &o.slug), true)) }
        urls.extend(write_redirects(app, &cx, n));
        if is_live(o) || is_live(n) { urls.extend(affected(&cx, o)); urls.extend(affected(&cx, n)); }
    }
    let mut changed = olds.clone();
    changed.extend(news.iter().cloned());
    rebuild_lists(app, &cx, Some(&changed))?;
    drop(gen);
    urls.sort(); urls.dedup();
    let cf = if urls.is_empty() { String::new() } else { cloudflare::purge(&cx.st, &urls) };
    if !google.is_empty() { crate::google::notify(app, &google); crate::indexnow::submit(app, &google.iter().map(|g| g.0.clone()).collect::<Vec<_>>()); }
    let what = match action { "publish" => "pubblicati", "draft" => "riportati in bozza", "feature" => "messi in evidenza", "unfeature" => "tolti dall'evidenza", "category" => "spostati nella nuova categoria", _ => "aggiornati con il tag" };
    Ok(format!("{} articoli {what}.{cf}", news.len()))
}

/// Notifica push per un articolo appena uscito: va in coda e parte entro un paio di secondi (la pubblicazione non aspetta).
/// Niente notifica per le pagine, e per gli articoli retrodatati di oltre un'ora (correzioni, importazioni).
fn push_new(app: &App, st: &Settings, p: &Post) {
    if p.kind != "post" || !crate::push::on(st) || st.get("push_on_publish").is_none_or(|v| v != "on") || p.published_at < now() - 3600 { return }
    let _ = app;
    crate::push::QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push((opt(st, "site_name", "").to_string(), p.title.clone(), post_url(st, &p.slug)));
}

// ---------- condivisione ----------

/// Social disponibili: (codice, nome, colore del social, testo scuro o chiaro per restare leggibile).
/// Il colore del testo è scelto per un contrasto di almeno 4,5:1 (WhatsApp e Telegram reggono solo il testo scuro).
// Colore del marchio e testo scuro (true) dove il bianco non avrebbe contrasto sufficiente sul colore del social.
const SHARE: &[(&str, &str, &str, bool)] = &[
    ("whatsapp", "WhatsApp", "#25D366", true), ("facebook", "Facebook", "#0866FF", false), ("x", "X", "#000000", false),
    ("telegram", "Telegram", "#26A5E4", true), ("linkedin", "LinkedIn", "#0A66C2", false), ("threads", "Threads", "#000000", false),
    ("bluesky", "Bluesky", "#0085FF", true), ("reddit", "Reddit", "#FF4500", true), ("email", "Email", "#4B5563", false),
];

/// Icone ufficiali dei social, dentro la pagina (nessuna richiesta in più): Bootstrap Icons 1.13.1,
/// licenza MIT, https://icons.getbootstrap.com — testo della licenza in assets/icons/LICENSE-bootstrap-icons.txt.
const SHARE_ICONS: &[(&str, &str)] = &[
    ("whatsapp", "<path d=\"M13.601 2.326A7.85 7.85 0 0 0 7.994 0C3.627 0 .068 3.558.064 7.926c0 1.399.366 2.76 1.057 3.965L0 16l4.204-1.102a7.9 7.9 0 0 0 3.79.965h.004c4.368 0 7.926-3.558 7.93-7.93A7.9 7.9 0 0 0 13.6 2.326zM7.994 14.521a6.6 6.6 0 0 1-3.356-.92l-.24-.144-2.494.654.666-2.433-.156-.251a6.56 6.56 0 0 1-1.007-3.505c0-3.626 2.957-6.584 6.591-6.584a6.56 6.56 0 0 1 4.66 1.931 6.56 6.56 0 0 1 1.928 4.66c-.004 3.639-2.961 6.592-6.592 6.592m3.615-4.934c-.197-.099-1.17-.578-1.353-.646-.182-.065-.315-.099-.445.099-.133.197-.513.646-.627.775-.114.133-.232.148-.43.05-.197-.1-.836-.308-1.592-.985-.59-.525-.985-1.175-1.103-1.372-.114-.198-.011-.304.088-.403.087-.088.197-.232.296-.346.1-.114.133-.198.198-.33.065-.134.034-.248-.015-.347-.05-.099-.445-1.076-.612-1.47-.16-.389-.323-.335-.445-.34-.114-.007-.247-.007-.38-.007a.73.73 0 0 0-.529.247c-.182.198-.691.677-.691 1.654s.71 1.916.81 2.049c.098.133 1.394 2.132 3.383 2.992.47.205.84.326 1.129.418.475.152.904.129 1.246.08.38-.058 1.171-.48 1.338-.943.164-.464.164-.86.114-.943-.049-.084-.182-.133-.38-.232\"/>"),
    ("facebook", "<path d=\"M16 8.049c0-4.446-3.582-8.05-8-8.05C3.58 0-.002 3.603-.002 8.05c0 4.017 2.926 7.347 6.75 7.951v-5.625h-2.03V8.05H6.75V6.275c0-2.017 1.195-3.131 3.022-3.131.876 0 1.791.157 1.791.157v1.98h-1.009c-.993 0-1.303.621-1.303 1.258v1.51h2.218l-.354 2.326H9.25V16c3.824-.604 6.75-3.934 6.75-7.951\"/>"),
    ("x", "<path d=\"M12.6.75h2.454l-5.36 6.142L16 15.25h-4.937l-3.867-5.07-4.425 5.07H.316l5.733-6.57L0 .75h5.063l3.495 4.633L12.601.75Zm-.86 13.028h1.36L4.323 2.145H2.865z\"/>"),
    ("telegram", "<path d=\"M16 8A8 8 0 1 1 0 8a8 8 0 0 1 16 0M8.287 5.906q-1.168.486-4.666 2.01-.567.225-.595.442c-.03.243.275.339.69.47l.175.055c.408.133.958.288 1.243.294q.39.01.868-.32 3.269-2.206 3.374-2.23c.05-.012.12-.026.166.016s.042.12.037.141c-.03.129-1.227 1.241-1.846 1.817-.193.18-.33.307-.358.336a8 8 0 0 1-.188.186c-.38.366-.664.64.015 1.088.327.216.589.393.85.571.284.194.568.387.936.629q.14.092.27.187c.331.236.63.448.997.414.214-.02.435-.22.547-.82.265-1.417.786-4.486.906-5.751a1.4 1.4 0 0 0-.013-.315.34.34 0 0 0-.114-.217.53.53 0 0 0-.31-.093c-.3.005-.763.166-2.984 1.09\"/>"),
    ("linkedin", "<path d=\"M0 1.146C0 .513.526 0 1.175 0h13.65C15.474 0 16 .513 16 1.146v13.708c0 .633-.526 1.146-1.175 1.146H1.175C.526 16 0 15.487 0 14.854zm4.943 12.248V6.169H2.542v7.225zm-1.2-8.212c.837 0 1.358-.554 1.358-1.248-.015-.709-.52-1.248-1.342-1.248S2.4 3.226 2.4 3.934c0 .694.521 1.248 1.327 1.248zm4.908 8.212V9.359c0-.216.016-.432.08-.586.173-.431.568-.878 1.232-.878.869 0 1.216.662 1.216 1.634v3.865h2.401V9.25c0-2.22-1.184-3.252-2.764-3.252-1.274 0-1.845.7-2.165 1.193v.025h-.016l.016-.025V6.169h-2.4c.03.678 0 7.225 0 7.225z\"/>"),
    ("email", "<path d=\"M.05 3.555A2 2 0 0 1 2 2h12a2 2 0 0 1 1.95 1.555L8 8.414zM0 4.697v7.104l5.803-3.558zM6.761 8.83l-6.57 4.027A2 2 0 0 0 2 14h12a2 2 0 0 0 1.808-1.144l-6.57-4.027L8 9.586zm3.436-.586L16 11.801V4.697z\"/>"),
    ("threads", "<path d=\"M6.321 6.016c-.27-.18-1.166-.802-1.166-.802.756-1.081 1.753-1.502 3.132-1.502.975 0 1.803.327 2.394.948s.928 1.509 1.005 2.644q.492.207.905.484c1.109.745 1.719 1.86 1.719 3.137 0 2.716-2.226 5.075-6.256 5.075C4.594 16 1 13.987 1 7.994 1 2.034 4.482 0 8.044 0 9.69 0 13.55.243 15 5.036l-1.36.353C12.516 1.974 10.163 1.43 8.006 1.43c-3.565 0-5.582 2.171-5.582 6.79 0 4.143 2.254 6.343 5.63 6.343 2.777 0 4.847-1.443 4.847-3.556 0-1.438-1.208-2.127-1.27-2.127-.236 1.234-.868 3.31-3.644 3.31-1.618 0-3.013-1.118-3.013-2.582 0-2.09 1.984-2.847 3.55-2.847.586 0 1.294.04 1.663.114 0-.637-.54-1.728-1.9-1.728-1.25 0-1.566.405-1.967.868ZM8.716 8.19c-2.04 0-2.304.87-2.304 1.416 0 .878 1.043 1.168 1.6 1.168 1.02 0 2.067-.282 2.232-2.423a6.2 6.2 0 0 0-1.528-.161\"/>"),
    ("bluesky", "<path d=\"M3.468 1.948C5.303 3.325 7.276 6.118 8 7.616c.725-1.498 2.698-4.29 4.532-5.668C13.855.955 16 .186 16 2.632c0 .489-.28 4.105-.444 4.692-.572 2.04-2.653 2.561-4.504 2.246 3.236.551 4.06 2.375 2.281 4.2-3.376 3.464-4.852-.87-5.23-1.98-.07-.204-.103-.3-.103-.218 0-.081-.033.014-.102.218-.379 1.11-1.855 5.444-5.231 1.98-1.778-1.825-.955-3.65 2.28-4.2-1.85.315-3.932-.205-4.503-2.246C.28 6.737 0 3.12 0 2.632 0 .186 2.145.955 3.468 1.948\"/>"),
    ("reddit", "<path d=\"M6.167 8a.83.83 0 0 0-.83.83c0 .459.372.84.83.831a.831.831 0 0 0 0-1.661m1.843 3.647c.315 0 1.403-.038 1.976-.611a.23.23 0 0 0 0-.306.213.213 0 0 0-.306 0c-.353.363-1.126.487-1.67.487-.545 0-1.308-.124-1.671-.487a.213.213 0 0 0-.306 0 .213.213 0 0 0 0 .306c.564.563 1.652.61 1.977.61zm.992-2.807c0 .458.373.83.831.83s.83-.381.83-.83a.831.831 0 0 0-1.66 0z\"/> <path d=\"M16 8A8 8 0 1 1 0 8a8 8 0 0 1 16 0m-3.828-1.165c-.315 0-.602.124-.812.325-.801-.573-1.9-.945-3.121-.993l.534-2.501 1.738.372a.83.83 0 1 0 .83-.869.83.83 0 0 0-.744.468l-1.938-.41a.2.2 0 0 0-.153.028.2.2 0 0 0-.086.134l-.592 2.788c-1.24.038-2.358.41-3.17.992-.21-.2-.496-.324-.81-.324a1.163 1.163 0 0 0-.478 2.224q-.03.17-.029.353c0 1.795 2.091 3.256 4.669 3.256s4.668-1.451 4.668-3.256c0-.114-.01-.238-.029-.353.401-.181.688-.592.688-1.069 0-.65-.525-1.165-1.165-1.165\"/>"),
    ("copy", "<path d=\"M4.715 6.542 3.343 7.914a3 3 0 1 0 4.243 4.243l1.828-1.829A3 3 0 0 0 8.586 5.5L8 6.086a1 1 0 0 0-.154.199 2 2 0 0 1 .861 3.337L6.88 11.45a2 2 0 1 1-2.83-2.83l.793-.792a4 4 0 0 1-.128-1.287z\"/> <path d=\"M6.586 4.672A3 3 0 0 0 7.414 9.5l.775-.776a2 2 0 0 1-.896-3.346L9.12 3.55a2 2 0 1 1 2.83 2.83l-.793.792c.112.42.155.855.128 1.287l1.372-1.372a3 3 0 1 0-4.243-4.243z\"/>"),
];
fn share_icon(code: &str) -> String {
    SHARE_ICONS.iter().find(|i| i.0 == code).map(|i| format!("<svg width=\"18\" height=\"18\" viewBox=\"0 0 16 16\" fill=\"currentColor\" aria-hidden=\"true\">{}</svg>", i.1)).unwrap_or_default()
}

fn enc(v: &str) -> String { form_urlencoded::byte_serialize(v.as_bytes()).collect() }

/// Pulsanti di condivisione sotto l'articolo: colori dei social, e il logo ufficiale se la redazione l'ha caricato.
fn share_html(st: &Settings, url: &str, title: &str) -> String {
    let nets = opt(st, "share_nets", "whatsapp,facebook,x,telegram,copy");
    let on = |code: &str| nets.split(',').any(|x| x.trim() == code);
    let brand = opt(st, "share_style", "brand") != "plain";
    let (u, t) = (enc(url), enc(title));
    let mut out = String::new();
    for (code, name, color, dark) in SHARE.iter().filter(|n| on(n.0)) {
        let href = match *code {
            "whatsapp" => format!("https://wa.me/?text={}", enc(&format!("{title} {url}"))),
            "facebook" => format!("https://www.facebook.com/sharer/sharer.php?u={u}"),
            "x" => format!("https://x.com/intent/post?url={u}&text={t}"),
            "telegram" => format!("https://t.me/share/url?url={u}&text={t}"),
            "linkedin" => format!("https://www.linkedin.com/sharing/share-offsite/?url={u}"),
            "threads" => format!("https://www.threads.net/intent/post?text={}", enc(&format!("{title} {url}"))),
            "bluesky" => format!("https://bsky.app/intent/compose?text={}", enc(&format!("{title} {url}"))),
            "reddit" => format!("https://www.reddit.com/submit?url={u}&title={t}"),
            _ => format!("mailto:?subject={t}&body={u}"),
        };
        // Un logo caricato dalla redazione ha la precedenza sull'icona inclusa.
        let logo = opt(st, &format!("share_logo_{code}"), "");
        let icon = if logo.is_empty() { share_icon(code) } else { format!("<img src=\"{}\" alt=\"\" width=\"18\" height=\"18\" loading=\"lazy\">", esc(logo)) };
        let style = if brand { format!(" style=\"--c:{color};--t:{}\"", if *dark { "#0b1638" } else { "#fff" }) } else { String::new() };
        let target = if *code == "email" { "" } else { " target=\"_blank\" rel=\"noopener\"" };
        let label = if *code == "email" { "Condividi via email".to_string() } else { format!("Condividi su {name}") };
        out += &format!("<a class=\"sh sh-{code}\" href=\"{}\"{target}{style} aria-label=\"{label}\" title=\"{label}\">{icon}<span>{name}</span></a>", esc(&href));
    }
    // «Copia link»: compare solo dove il browser sa copiare (senza JavaScript resta nascosto, non un pulsante morto).
    let copy = if on("copy") { format!("<button type=\"button\" class=\"sh sh-copy\" data-url=\"{}\" aria-label=\"Copia il link dell'articolo\" title=\"Copia il link\" hidden>{}<span>Copia link</span></button>", esc(url), share_icon("copy")) } else { String::new() };
    if out.is_empty() && copy.is_empty() { return String::new() }
    let script = if copy.is_empty() { String::new() } else { "<script>document.querySelectorAll('.sh-copy').forEach(b=>{if(!navigator.clipboard)return;b.hidden=false;b.addEventListener('click',()=>navigator.clipboard.writeText(b.dataset.url).then(()=>{const s=b.querySelector('span'),o=s.textContent;s.textContent='Link copiato';b.classList.add('ok');setTimeout(()=>{s.textContent=o;b.classList.remove('ok')},2200)}))})</script>".to_string() };
    format!("<nav class=\"share sh-bar\" aria-label=\"Condividi\" data-pagefind-ignore><style>\
.sh-bar{{display:flex;flex-wrap:wrap;align-items:center;gap:.5rem;margin:2rem 0}}.sh-bar>.sh-label{{margin-right:.3rem;font-weight:800;font-size:.95rem}}\
.sh-bar .sh{{display:inline-flex;align-items:center;gap:.5rem;min-height:2.6rem;padding:0 1.05rem 0 .85rem;border:0;border-radius:999px;background:var(--c,var(--wash,#eef1f6));color:var(--t,var(--ink,#111));font-family:inherit;font-weight:700;font-size:.92rem;line-height:1;text-decoration:none;cursor:pointer;transition:transform .15s ease,box-shadow .15s ease}}\
.sh-bar .sh[hidden]{{display:none}}.sh-bar .sh:hover{{transform:translateY(-2px);box-shadow:0 10px 20px -12px var(--c,#0b1638);color:var(--t,var(--ink,#111))}}\
.sh-bar .sh:focus-visible{{outline:3px solid var(--c,var(--accent,#1e6bff));outline-offset:2px}}.sh-bar .sh svg,.sh-bar .sh img{{display:block;flex:none;width:18px;height:18px}}\
.sh-bar .sh-copy{{background:var(--wash,#eef1f6);color:var(--ink,#111)}}.sh-bar .sh-copy.ok{{background:#15803d;color:#fff}}\
@media (max-width:40rem){{.sh-bar .sh{{justify-content:center;width:2.75rem;height:2.75rem;min-height:0;padding:0}}.sh-bar .sh svg,.sh-bar .sh img{{width:20px;height:20px}}.sh-bar .sh span{{position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%);white-space:nowrap}}}}\
@media (prefers-reduced-motion:reduce){{.sh-bar .sh{{transition:none}}.sh-bar .sh:hover{{transform:none}}}}\
:root[data-mode=dark] .sh-bar .sh-x,:root[data-mode=dark] .sh-bar .sh-threads{{box-shadow:inset 0 0 0 1px rgba(255,255,255,.28)}}@media (prefers-color-scheme:dark){{:root:not([data-mode=light]) .sh-bar .sh-x,:root:not([data-mode=light]) .sh-bar .sh-threads{{box-shadow:inset 0 0 0 1px rgba(255,255,255,.28)}}}}\
</style><span class=\"sh-label\">Condividi</span>{out}{copy}</nav>{script}")
}

// ---------- commenti ----------
// Statici: i commenti approvati sono scritti dentro la pagina dell'articolo (nessuna richiesta in più per chi legge),
// il modulo è HTML semplice senza JavaScript. Il programma entra in gioco solo quando qualcuno invia un commento.

pub fn comments_on(st: &Settings) -> bool { st.get("comments_on").is_some_and(|v| v == "on") }

/// Commenti ancora aperti su questo articolo: attivi nel sito, aperti sull'articolo, non oltre i giorni impostati.
fn comments_open(st: &Settings, p: &Post) -> bool {
    let days: i64 = opt(st, "comments_close_days", "0").parse().unwrap_or(0);
    comments_on(st) && p.comments && p.kind == "post" && (days <= 0 || now() - p.published_at < days * 86_400)
}

fn comment_text(body: &str) -> String {
    body.split("\n\n").map(str::trim).filter(|p| !p.is_empty()).map(|p| format!("<p>{}</p>", esc(p).replace('\n', "<br>"))).collect()
}

fn comments_html(app: &App, cx: &Cx, p: &Post) -> String {
    let st = &cx.st;
    if !comments_on(st) || p.kind != "post" { return String::new() }
    let rows: Vec<(i64, i64, String, String, i64, bool)> = app.db.lock().unwrap_or_else(|e| e.into_inner())
        .prepare("SELECT id, parent_id, name, body, created_at, staff FROM comments WHERE post_id = ?1 AND status = 'approved' ORDER BY id")
        .and_then(|mut q| q.query_map([p.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).map(|r| r.filter_map(Result::ok).collect()))
        .unwrap_or_default();
    let open = comments_open(st, p);
    if rows.is_empty() && !open { return String::new() }
    let one = |c: &(i64, i64, String, String, i64, bool)| format!(
        "<article class=\"cm{}\" id=\"commento-{}\"><span class=\"cm-av\" aria-hidden=\"true\">{}</span><div class=\"cm-in\"><header><strong>{}</strong>{}<time datetime=\"{}\">{}</time></header>{}</div></article>",
        if c.5 { " cm-staff" } else { "" }, c.0, esc(&c.2.chars().next().unwrap_or('?').to_uppercase().to_string()), esc(&c.2),
        if c.5 { "<span class=\"cm-badge\">Redazione</span>" } else { "" }, iso(c.4, &cx.tz), human(c.4, &cx.tz), comment_text(&c.3));
    let mut list = String::new();
    for c in rows.iter().filter(|c| c.1 == 0) {
        list += &one(c);
        let replies: String = rows.iter().filter(|r| r.1 == c.0).map(one).collect();
        if !replies.is_empty() { list += &format!("<div class=\"cm-replies\">{replies}</div>") }
    }
    let n = rows.len();
    let form = if open {
        let action = format!("{}/commenti/{}", base(st), p.id); // dal sito: Nginx la passa al programma, il pannello può restare chiuso
        let privacy = format!("{}/privacy/", base(st));
        format!("<form class=\"cm-form\" method=\"post\" action=\"{}\"><h3>Scrivi un commento</h3>\
<p class=\"cm-ok\" id=\"commento-inviato\">Grazie! Il tuo commento comparirà qui dopo l'approvazione della redazione.</p>\
<div class=\"cm-row\"><label><span>Nome</span><input name=\"name\" required maxlength=\"80\" autocomplete=\"name\"></label>\
<label><span>Email <small>non sarà pubblicata</small></span><input type=\"email\" name=\"email\" required maxlength=\"254\" autocomplete=\"email\"></label></div>\
<label class=\"cm-hp\" aria-hidden=\"true\">Sito web<input name=\"website\" tabindex=\"-1\" autocomplete=\"off\"></label>\
<label>Commento<textarea name=\"body\" required minlength=\"2\" maxlength=\"4000\" rows=\"5\"></textarea></label>\
<label class=\"cm-consent\"><input type=\"checkbox\" name=\"consent\" required><span>Ho letto l'<a href=\"{}\">informativa sulla privacy</a> e accetto che nome e commento siano pubblicati.</span></label>\
<button type=\"submit\">Invia il commento</button></form>", esc(&action), esc(&privacy))
    } else { "<p class=\"cm-closed\">I commenti sono chiusi.</p>".to_string() };
    // Stili solo qui dentro: con i commenti spenti le pagine non hanno un byte in più.
    format!("<section class=\"comments\" id=\"commenti\" data-pagefind-ignore><style>\
.comments{{margin:3rem 0 0;padding-top:1.6rem;border-top:1px solid var(--line,#e2e5ec)}}.comments h2{{font-size:1.35rem;margin:0 0 .6rem}}\
.cm{{display:flex;gap:.85rem;padding:1.05rem 0;border-bottom:1px solid var(--line,#eceef3)}}.cm-in{{flex:1;min-width:0}}\
.cm-av{{flex:none;width:2.5rem;height:2.5rem;border-radius:50%;display:grid;place-items:center;font-weight:800;font-size:1.05rem;color:#fff;background:var(--accent,#1e6bff)}}\
.cm-staff .cm-av{{background:#0b1638}}.cm header{{display:flex;flex-wrap:wrap;gap:.3rem .6rem;align-items:center;font-size:.92rem}}.cm time{{color:var(--muted,#6b7280);font-size:.85rem}}\
.cm p{{margin:.35rem 0 0;line-height:1.6;overflow-wrap:anywhere}}.cm-replies{{margin-left:3.35rem}}.cm-replies .cm{{padding:.85rem 1rem;margin:.6rem 0;border:0;border-radius:10px;background:var(--soft,#f4f6fa)}}\
.cm-badge{{font-size:.7rem;font-weight:800;letter-spacing:.02em;text-transform:uppercase;padding:.15rem .5rem;border-radius:999px;background:var(--accent,#1e6bff);color:#fff}}\
.cm-form{{display:grid;gap:.9rem;margin-top:1.8rem;padding:1.4rem 1.5rem;border-radius:12px;background:var(--soft,#f4f6fa);border:1px solid var(--line,#e2e5ec)}}.cm-form h3{{margin:0;font-size:1.15rem}}\
.cm-row{{display:grid;grid-template-columns:1fr 1fr;gap:.9rem}}.cm-form label{{display:grid;gap:.35rem;font-weight:700;font-size:.9rem}}.cm-form small{{font-weight:500;color:var(--muted,#6b7280)}}\
.cm-form input,.cm-form textarea{{font:inherit;font-weight:400;padding:.7rem .8rem;border:1px solid var(--line,#cfd4de);border-radius:8px;width:100%;box-sizing:border-box;background:#fff}}\
.cm-form .cm-consent{{display:flex;gap:.55rem;align-items:flex-start;font-weight:400;font-size:.88rem}}.cm-form .cm-consent input{{width:auto;margin-top:.2rem}}\
.cm-form button{{justify-self:start;font:inherit;font-weight:800;padding:.75rem 1.3rem;border:0;border-radius:8px;background:var(--accent,#1e6bff);color:#fff;cursor:pointer}}\
.cm-hp{{position:absolute;left:-9999px}}.cm-ok{{display:none;padding:.8rem 1rem;border-radius:8px;background:#e7f8ef;color:#0e5c34;font-weight:600}}.cm-ok:target{{display:block}}\
.cm-closed{{color:var(--muted,#6b7280)}}@media (max-width:40rem){{.cm-row{{grid-template-columns:1fr}}.cm-replies{{margin-left:1.2rem}}}}\
</style><h2>{}</h2>{list}{form}</section>", if n == 0 { "Commenti".to_string() } else if n == 1 { "1 commento".to_string() } else { format!("{n} commenti") })
}

/// Aggiorna i più letti da Google Analytics. Se l'elenco cambia, si rigenera solo la home (non gli articoli).
pub fn update_most_read(app: &App) -> R<String> {
    let st = app.settings();
    let rows = crate::google::most_read(app, &st)?;
    let n: usize = opt(&st, "most_read_count", "5").parse().unwrap_or(5).clamp(3, 10);
    let slugs: Vec<(String, i64)> = rows.iter().filter_map(|(path, views)| {
        let slug = path.split(['?', '#']).next()?.trim_matches('/');
        (!slug.is_empty() && !slug.contains('/')).then(|| (slug.to_string(), *views))
    }).collect();
    let ids: Vec<(i64, String, i64)> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        slugs.iter().filter_map(|(slug, views)| db.query_row("SELECT id, title FROM posts WHERE slug = ?1 AND kind = 'post' AND status = 'published' AND published_at <= ?2", params![slug, now()], |r| Ok((r.get(0)?, r.get(1)?))).ok().map(|(id, t): (i64, String)| (id, t, *views))).take(n).collect()
    };
    let list = serde_json::to_string(&ids.iter().map(|x| x.0).collect::<Vec<_>>()).unwrap_or_default();
    let changed = opt(&st, "most_read", "") != list;
    {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let _ = db.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('most_read', ?1)", [&list]);
        let _ = db.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('most_read_at', ?1)", [now().to_string()]);
    }
    if changed && st.get("most_read_on").is_some_and(|v| v == "on") {
        let gen = gen_lock();
        let cx = Cx::load(app);
        rebuild_lists(app, &cx, Some(&[Post::default()]))?; // solo la home: l'articolo finto non ha categorie, tag né autore
        drop(gen);
        cloudflare::purge(&cx.st, &[format!("{}/", base(&cx.st))]);
    }
    if ids.is_empty() { return Ok("Google Analytics ha risposto, ma nessuna delle pagine più viste è un articolo del sito (controlla che la proprietà sia quella giusta).".into()) }
    Ok(format!("Più letti aggiornati: {}.", ids.iter().map(|x| format!("«{}» ({} visite)", x.1, x.2)).collect::<Vec<_>>().join(", ")))
}

/// Commento inviato da un lettore. Restituisce l'indirizzo a cui rimandarlo (l'articolo, con il messaggio di ringraziamento).
pub fn comment_submit(app: &App, post_id: i64, f: &HashMap<String, String>, ip: &str) -> R<(String, Option<(String, String)>)> {
    let st = app.settings();
    let p = get_post(app, post_id).filter(is_live).ok_or("articolo non trovato")?;
    let back = format!("{}#commento-inviato", post_url(&st, &p.slug));
    if !comments_open(&st, &p) { return Err("i commenti su questo articolo sono chiusi".into()) }
    let v = |k: &str| f.get(k).map(|x| x.trim().to_string()).unwrap_or_default();
    // Campo trappola: le persone non lo vedono, i programmi di spam lo riempiono. Si fa finta di niente.
    if !v("website").is_empty() { return Ok((back, None)) }
    let (name, email, body) = (v("name"), v("email").to_lowercase(), v("body").replace("\r\n", "\n"));
    if name.is_empty() || name.chars().count() > 80 { return Err("scrivi il tuo nome (al massimo 80 caratteri)".into()) }
    if !email.contains('@') || email.len() > 254 || email.contains(char::is_whitespace) { return Err("scrivi un indirizzo email valido".into()) }
    if body.chars().count() < 2 || body.chars().count() > 4000 { return Err("il commento deve avere da 2 a 4000 caratteri".into()) }
    if v("consent") != "on" { return Err("per commentare devi accettare l'informativa sulla privacy".into()) }
    let who = crate::ip_hash(app, ip); // l'indirizzo IP non si salva, solo la sua impronta (per i limiti)
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let recent: i64 = db.query_row("SELECT COUNT(*) FROM comments WHERE ip_hash = ?1 AND created_at > ?2", params![who, now() - 600], |r| r.get(0)).unwrap_or(0);
    if recent >= 5 { return Err("hai inviato troppi commenti in poco tempo: riprova tra qualche minuto".into()) }
    // Molti link o testo già inviato identico: direttamente tra lo spam (la redazione può comunque recuperarlo).
    let links = body.matches("http").count() + body.matches("www.").count();
    let dup: bool = db.query_row("SELECT 1 FROM comments WHERE post_id = ?1 AND body = ?2", params![post_id, body], |_| Ok(())).is_ok();
    let status = if links > 2 || dup { "spam" } else { "pending" };
    db.execute("INSERT INTO comments(post_id, parent_id, name, email, body, status, ip_hash, staff, created_at) VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
        params![post_id, name, email, body, status, who, now()]).map_err(s)?;
    Ok((back, (status == "pending").then(|| (p.title.clone(), name))))
}

// ---------- dirette ----------

type LiveUp = (i64, String, String, bool, i64); // id, titolo, testo, importante, ora

fn live_data(app: &App, id: i64) -> (i64, i64, Vec<LiveUp>) {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let (state, end): (i64, i64) = db.query_row("SELECT diretta, diretta_fine FROM posts WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap_or((0, 0));
    if state == 0 { return (0, 0, vec![]) }
    let ups = db.prepare("SELECT id, title, body, key, created_at FROM diretta WHERE post_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 500")
        .and_then(|mut q| q.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? == 1, r.get(4)?))).map(|r| r.filter_map(Result::ok).collect())).unwrap_or_default();
    (state, end, ups)
}

/// Il blocco della diretta nella pagina: aggiornamenti dal più recente con l'orario. Mentre è in corso, ogni 30 secondi
/// (solo con la scheda visibile) la pagina chiede al server la versione nuova e propone «N nuovi aggiornamenti»:
/// niente ricaricamento, niente salti mentre si legge. Senza JavaScript la pagina resta completa.
fn live_html(cx: &Cx, state: i64, ups: &[LiveUp]) -> String {
    if state == 0 { return String::new() }
    let tz = &cx.tz;
    let items: String = ups.iter().map(|(id, title, body, key, at)| format!("<li id=\"agg-{id}\"{}><time datetime=\"{}\">{}</time><div>{}{}</div></li>",
        if *key { " class=\"key\"" } else { "" }, iso(*at, tz), fmt(*at, tz, "%H:%M"), if title.is_empty() { String::new() } else { format!("<h3>{}</h3>", esc(title)) }, comment_text(body))).collect();
    let status = if state == 1 { match ups.first() { Some(u) => format!("In corso: ultimo aggiornamento alle {}", fmt(u.4, tz, "%H:%M")), None => "In corso: i primi aggiornamenti arrivano a breve".into() } } else { "Diretta conclusa".into() };
    format!("<section class=\"ps-live\" id=\"diretta\" data-live=\"{live}\" aria-labelledby=\"ps-live-t\"><style>\
.ps-live{{margin:2.2rem 0;padding-top:1.2rem;border-top:3px solid #dc2626}}.ps-live>header{{display:flex;flex-wrap:wrap;align-items:baseline;justify-content:space-between;gap:.3rem 1rem;margin-bottom:.8rem}}\
.ps-live h2{{display:inline-flex;align-items:center;gap:.5rem;margin:0;font-size:1.5rem}}.ps-live h2::before{{content:\"\";width:.65rem;height:.65rem;border-radius:50%;background:#dc2626}}\
.ps-live[data-live=\"1\"] h2::before{{animation:lvb 1.4s ease-in-out infinite}}.ps-live .st{{margin:0;font-size:.9rem;color:var(--muted,#5b6270)}}\
.ps-feed{{list-style:none;margin:0;padding:0}}.ps-feed>li{{display:grid;grid-template-columns:4.2rem minmax(0,1fr);gap:1rem;padding:1.1rem 0;border-top:1px solid var(--line,#e3e5ea)}}\
.ps-feed time{{font-weight:800;font-variant-numeric:tabular-nums;color:#dc2626}}.ps-feed h3{{margin:0 0 .35rem;font-size:1.2rem}}.ps-feed p{{margin:0 0 .6rem}}.ps-feed p:last-child{{margin:0}}\
.ps-feed>li.key{{margin:0 -1rem;padding:1.1rem 1rem;border-radius:12px;border-top-color:transparent;background:color-mix(in srgb,#dc2626 9%,var(--bg,#fff))}}\
.ps-new[hidden]{{display:none}}.ps-new{{position:sticky;top:.8rem;z-index:5;display:flex;justify-content:center;margin:0 0 .6rem}}\
.ps-new button{{padding:.55rem 1.1rem;border:0;border-radius:999px;background:#dc2626;color:#fff;font:800 .92rem/1.2 inherit;cursor:pointer;box-shadow:0 10px 24px -12px rgba(0,0,0,.5)}}\
@keyframes lvb{{50%{{opacity:.3}}}}@media (prefers-reduced-motion:reduce){{.ps-live[data-live=\"1\"] h2::before{{animation:none}}}}\
@media (max-width:40rem){{.ps-feed>li{{grid-template-columns:1fr;gap:.3rem}}}}\
</style><header><h2 id=\"ps-live-t\">Diretta</h2><p class=\"st\">{status}</p></header><div class=\"ps-new\" hidden><button type=\"button\">Nuovi aggiornamenti</button></div><ol class=\"ps-feed\">{items}</ol></section>{script}",
        live = if state == 1 { "1" } else { "0" }, status = esc(&status),
        script = if state == 1 { "<script>(()=>{const s=document.getElementById('diretta');if(!s)return;const feed=s.querySelector('.ps-feed'),bar=s.querySelector('.ps-new'),btn=bar.querySelector('button');let pend=[],t;\
async function check(){try{const r=await fetch(location.pathname,{cache:'no-store'});if(!r.ok)return;const d=new DOMParser().parseFromString(await r.text(),'text/html'),ns=d.getElementById('diretta');if(!ns)return;\
const have=new Set([...feed.children].map(li=>li.id).concat(pend.map(li=>li.id)));const fresh=[...ns.querySelectorAll('.ps-feed>li')].filter(li=>!have.has(li.id));\
if(fresh.length){pend=fresh.concat(pend);btn.textContent=pend.length===1?'1 nuovo aggiornamento':pend.length+' nuovi aggiornamenti';bar.hidden=false}\
s.querySelector('.st').textContent=ns.querySelector('.st').textContent;if(ns.dataset.live!=='1'){s.dataset.live=ns.dataset.live;clearInterval(t)}}catch(e){}}\
btn.addEventListener('click',()=>{pend.slice().reverse().forEach(li=>feed.prepend(document.importNode(li,true)));pend=[];bar.hidden=true});\
t=setInterval(()=>{if(!document.hidden)check()},30000);window.psLiveCheck=check})()</script>" } else { "" })
}

/// Dopo un aggiornamento della diretta (o apertura e chiusura): l'orario di modifica avanza (dateModified), si rifanno
/// l'articolo e gli elenchi in cui compare, si svuota la cache di Cloudflare per quegli indirizzi e si avvisa IndexNow.
pub fn live_touch(app: &App, id: i64) -> R<()> {
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("UPDATE posts SET updated_at = ?1 WHERE id = ?2", params![now(), id]);
    let gen = gen_lock();
    let cx = Cx::load(app);
    let p = get_post(app, id).ok_or("articolo non trovato")?;
    if !is_live(&p) { return Ok(()) } // bozza o programmato: niente da pubblicare
    publish_one(app, &cx, &p)?;
    rebuild_lists(app, &cx, Some(&[p.clone()]))?;
    drop(gen);
    let urls = affected(&cx, &p);
    let _ = cloudflare::purge(&cx.st, &urls);
    let _ = crate::indexnow::submit(app, &[post_url(&cx.st, &p.slug)]);
    Ok(())
}

/// Moderazione: approva, sposta tra lo spam, rimetti in attesa o elimina. Le pagine degli articoli si aggiornano.
pub fn comment_action(app: &App, ids: &[i64], action: &str) -> R<String> {
    if ids.is_empty() { return Err("scegli almeno un commento".into()) }
    let posts: Vec<i64> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut posts = vec![];
        in_tx(&db, |db| {
            for id in ids {
                if let Ok(pid) = db.query_row("SELECT post_id FROM comments WHERE id = ?1", [id], |r| r.get::<_, i64>(0)) { if !posts.contains(&pid) { posts.push(pid) } }
                match action {
                    "approve" => db.execute("UPDATE comments SET status = 'approved' WHERE id = ?1", [id]),
                    "spam" => db.execute("UPDATE comments SET status = 'spam' WHERE id = ?1", [id]),
                    "pending" => db.execute("UPDATE comments SET status = 'pending' WHERE id = ?1", [id]),
                    "delete" => db.execute("DELETE FROM comments WHERE id = ?1 OR parent_id = ?1", [id]),
                    _ => return Err("azione sconosciuta".into()),
                }.map_err(s)?;
            }
            Ok(())
        })?;
        posts
    };
    refresh_posts(app, &posts);
    Ok(format!("{} {}.", ids.len(), match action { "approve" => "approvati: sono sul sito", "spam" => "spostati tra lo spam", "pending" => "rimessi in attesa", _ => "eliminati" }))
}

/// Risposta della redazione: pubblicata subito, sotto il commento a cui risponde.
pub fn comment_reply(app: &App, me: &User, parent: i64, body: &str) -> R<String> {
    let body = body.trim().replace("\r\n", "\n");
    if body.chars().count() < 2 || body.chars().count() > 4000 { return Err("la risposta deve avere da 2 a 4000 caratteri".into()) }
    let pid: i64 = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let (pid, top): (i64, i64) = db.query_row("SELECT post_id, CASE WHEN parent_id = 0 THEN id ELSE parent_id END FROM comments WHERE id = ?1", [parent], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|_| "commento non trovato".to_string())?;
        db.execute("UPDATE comments SET status = 'approved' WHERE id = ?1 AND status = 'pending'", [parent]).map_err(s)?; // rispondere vale come approvare
        db.execute("INSERT INTO comments(post_id, parent_id, name, email, body, status, ip_hash, staff, created_at) VALUES (?1, ?2, ?3, ?4, ?5, 'approved', '', 1, ?6)",
            params![pid, top, me.name, me.email, body, now()]).map_err(s)?;
        pid
    };
    refresh_posts(app, &[pid]);
    Ok("Risposta pubblicata.".into())
}

/// Rifà le pagine degli articoli indicati (per esempio dopo la moderazione dei commenti).
fn refresh_posts(app: &App, ids: &[i64]) {
    let gen = gen_lock();
    let cx = Cx::load(app);
    let urls: Vec<String> = ids.iter().filter_map(|&id| get_post(app, id)).filter(is_live).filter_map(|p| publish_one(app, &cx, &p).ok().map(|_| post_url(&cx.st, &p.slug))).collect();
    drop(gen);
    if !urls.is_empty() { cloudflare::purge(&cx.st, &urls); }
}

// ---------- categorie ----------

/// Tutte le categorie, in ordine ad albero (madri seguite dalle figlie), con il numero di articoli e la profondità.
pub fn category_tree(app: &App) -> Vec<Value> {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let _ = db.execute("INSERT OR IGNORE INTO categories(name) SELECT DISTINCT category FROM posts WHERE category <> ''", []);
    let rows: Vec<(String, String, String)> = db.prepare("SELECT name, parent, description FROM categories ORDER BY name COLLATE NOCASE")
        .and_then(|mut q| q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map(|r| r.filter_map(Result::ok).collect())).unwrap_or_default();
    let count = |n: &str| -> i64 { db.query_row("SELECT COUNT(*) FROM posts WHERE kind = 'post' AND (category = ?1 OR (', ' || categories || ', ') LIKE '%, ' || ?1 || ', %')", [n], |r| r.get(0)).unwrap_or(0) };
    let mut out = vec![];
    fn walk(rows: &[(String, String, String)], parent: &str, depth: usize, out: &mut Vec<(String, String, String, usize)>) {
        if depth > 5 { return }
        for r in rows.iter().filter(|r| r.1 == parent || (parent.is_empty() && !r.1.is_empty() && !rows.iter().any(|x| x.0 == r.1))) {
            if out.iter().any(|o| o.0 == r.0) { continue }
            out.push((r.0.clone(), r.1.clone(), r.2.clone(), depth));
            walk(rows, &r.0, depth + 1, out);
        }
    }
    let mut flat = vec![];
    walk(&rows, "", 0, &mut flat);
    for r in &rows { if !flat.iter().any(|f| f.0 == r.0) { flat.push((r.0.clone(), r.1.clone(), r.2.clone(), 0)) } } // eventuali cerchi
    for (name, parent, desc, depth) in flat { out.push(json!({"name": name, "parent": parent, "description": desc, "depth": depth, "count": count(&name), "slug": slugify(&name)})) }
    out
}

/// Salva madre e descrizione di una categoria (o la crea). Una categoria non può stare dentro sé stessa o una sua figlia.
pub fn category_save(app: &App, name: &str, parent: &str, desc: &str) -> R<String> {
    let (name, parent, desc) = (name.trim(), parent.trim(), desc.trim());
    if name.is_empty() || name.chars().count() > 100 { return Err("il nome deve avere da 1 a 100 caratteri".into()) }
    if slugify(name).is_empty() { return Err("il nome deve contenere lettere o numeri".into()) }
    if desc.chars().count() > 500 { return Err("la descrizione può avere al massimo 500 caratteri".into()) }
    {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        // la madre scelta non deve discendere da questa categoria
        let mut cur = parent.to_string();
        for _ in 0..8 {
            if cur.is_empty() { break }
            if slugify(&cur) == slugify(name) { return Err("una categoria non può stare dentro sé stessa o dentro una sua sottocategoria".into()) }
            cur = db.query_row("SELECT parent FROM categories WHERE name = ?1", [&cur], |r| r.get(0)).unwrap_or_default();
        }
        if !parent.is_empty() { db.execute("INSERT OR IGNORE INTO categories(name) VALUES (?1)", [parent]).map_err(s)?; }
        db.execute("INSERT INTO categories(name, parent, description) VALUES (?1, ?2, ?3) ON CONFLICT(name) DO UPDATE SET parent = ?2, description = ?3", params![name, parent, desc]).map_err(s)?;
    }
    rebuild_all(app).map(|m| format!("Categoria «{name}» salvata. {m}"))
}

/// Rinomina una categoria in tutti gli articoli; il vecchio indirizzo della sua pagina porta a quello nuovo.
/// Le sezioni del menu diventano categorie (se non ce n'è già una con lo stesso indirizzo, maiuscole comprese).
/// Le righe con «|» sono link («Contatti | /contatti/») e restano solo nel menu.
pub fn menu_categories(db: &Connection, menu: &str) {
    let have: Vec<String> = db.prepare("SELECT name FROM categories").and_then(|mut q| q.query_map([], |r| r.get::<_, String>(0)).map(|r| r.filter_map(Result::ok).collect())).unwrap_or_default();
    let mut slugs: Vec<String> = have.iter().map(|n| slugify(n)).collect();
    for l in menu.lines().map(str::trim).filter(|l| !l.is_empty() && !l.contains('|')) {
        let sl = slugify(l);
        if sl.is_empty() || slugs.contains(&sl) { continue }
        let _ = db.execute("INSERT OR IGNORE INTO categories(name) VALUES (?1)", [l]);
        slugs.push(sl);
    }
}

/// Rinomina (Some) o toglie (None) la voce del menu che corrisponde a una categoria.
fn menu_rename(db: &Connection, old: &str, new: Option<&str>) {
    let menu: String = db.query_row("SELECT value FROM settings WHERE key = 'menu'", [], |r| r.get(0)).unwrap_or_default();
    if menu.is_empty() { return }
    let out: Vec<String> = menu.lines().filter_map(|l| if l.trim() == old { new.map(String::from) } else { Some(l.to_string()) }).collect();
    let out = out.join("\n");
    if out != menu { let _ = db.execute("UPDATE settings SET value = ?1 WHERE key = 'menu'", [out]); }
}

pub fn category_rename(app: &App, old: &str, new: &str) -> R<String> {
    let (old, new) = (old.trim(), new.trim());
    if new.is_empty() || new.chars().count() > 100 || slugify(new).is_empty() { return Err("scrivi il nuovo nome (da 1 a 100 caratteri, con lettere o numeri)".into()) }
    if old == new { return Ok("Nessun cambiamento.".into()) }
    {
        let _gen = gen_lock();
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        in_tx(&db, |db| {
            db.execute("UPDATE posts SET category = ?2 WHERE category = ?1", params![old, new]).map_err(s)?;
            let extra: Vec<(i64, String)> = db.prepare("SELECT id, categories FROM posts WHERE categories <> ''").and_then(|mut q| q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|r| r.filter_map(Result::ok).collect())).map_err(s)?;
            for (id, cats) in extra {
                if tags(&cats).iter().any(|c| c == old) {
                    let v: Vec<String> = tags(&cats).into_iter().map(|c| if c == old { new.to_string() } else { c }).collect();
                    db.execute("UPDATE posts SET categories = ?2 WHERE id = ?1", params![id, v.join(", ")]).map_err(s)?;
                }
            }
            let (parent, desc): (String, String) = db.query_row("SELECT parent, description FROM categories WHERE name = ?1", [old], |r| Ok((r.get(0)?, r.get(1)?))).unwrap_or_default();
            db.execute("DELETE FROM categories WHERE name = ?1", [old]).map_err(s)?;
            menu_rename(&db, old, Some(new)); // la voce del menu segue la categoria
            db.execute("INSERT INTO categories(name, parent, description) VALUES (?1, ?2, ?3) ON CONFLICT(name) DO UPDATE SET parent = ?2, description = ?3", params![new, parent, desc]).map_err(s)?;
            db.execute("UPDATE categories SET parent = ?2 WHERE parent = ?1", params![old, new]).map_err(s)?;
            if slugify(old) != slugify(new) { db.execute("INSERT OR REPLACE INTO cat_moves(old_slug, name) VALUES (?1, ?2)", params![slugify(old), new]).map_err(s)?; }
            db.execute("DELETE FROM cat_moves WHERE old_slug = ?1", [slugify(new)]).map_err(s)?;
            Ok(())
        })?;
    }
    let _ = fs::remove_dir_all(app.public.join("category").join(slugify(old)));
    rebuild_all(app).map(|m| format!("Categoria rinominata in «{new}»: il vecchio indirizzo porta al nuovo. {m}"))
}

/// Elimina una categoria: gli articoli passano a quella scelta (o restano senza), le sottocategorie salgono di un livello.
pub fn category_delete(app: &App, name: &str, to: &str) -> R<String> {
    let (name, to) = (name.trim(), to.trim());
    if name == to { return Err("scegli una categoria diversa da quella che elimini".into()) }
    {
        let _gen = gen_lock();
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        in_tx(&db, |db| {
            db.execute("UPDATE posts SET category = ?2 WHERE category = ?1", params![name, to]).map_err(s)?;
            let extra: Vec<(i64, String)> = db.prepare("SELECT id, categories FROM posts WHERE categories <> ''").and_then(|mut q| q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|r| r.filter_map(Result::ok).collect())).map_err(s)?;
            for (id, cats) in extra {
                if tags(&cats).iter().any(|c| c == name) {
                    let v: Vec<String> = tags(&cats).into_iter().filter(|c| c != name).collect();
                    db.execute("UPDATE posts SET categories = ?2 WHERE id = ?1", params![id, v.join(", ")]).map_err(s)?;
                }
            }
            let parent: String = db.query_row("SELECT parent FROM categories WHERE name = ?1", [name], |r| r.get(0)).unwrap_or_default();
            db.execute("UPDATE categories SET parent = ?2 WHERE parent = ?1", params![name, parent]).map_err(s)?;
            db.execute("DELETE FROM categories WHERE name = ?1", [name]).map_err(s)?;
            menu_rename(&db, name, None); // e sparisce dal menu con lei
            if !to.is_empty() { db.execute("INSERT OR REPLACE INTO cat_moves(old_slug, name) VALUES (?1, ?2)", params![slugify(name), to]).map_err(s)?; }
            Ok(())
        })?;
    }
    let _ = fs::remove_dir_all(app.public.join("category").join(slugify(name)));
    rebuild_all(app).map(|m| format!("Categoria «{name}» eliminata. {m}"))
}

/// Note della redazione su un articolo: restano nel pannello, non finiscono mai sul sito.
pub fn notes(app: &App, post_id: i64, tz: &TimeZone) -> Vec<Value> {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(mut q) = db.prepare("SELECT n.id, n.body, n.created_at, COALESCE(u.name, ''), n.user_id FROM post_notes n LEFT JOIN users u ON u.id = n.user_id WHERE n.post_id = ?1 ORDER BY n.id") else { return vec![] };
    let v = q.query_map([post_id], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "body": r.get::<_, String>(1)?, "when": human(r.get(2)?, tz), "who": r.get::<_, String>(3)?, "user_id": r.get::<_, i64>(4)? })))
        .map(|r| r.filter_map(Result::ok).collect()).unwrap_or_default();
    v
}

/// Aggiunge una nota; restituisce l'articolo (per avvisare l'autore via email).
pub fn add_note(app: &App, me: &User, post_id: i64, body: &str) -> R<Post> {
    let p = get_post(app, post_id).ok_or("articolo non trovato")?;
    if !me.can_edit(&p) && !me.editor() { return Err("non puoi scrivere note su questo articolo".into()) }
    let body = body.trim();
    if body.is_empty() || body.chars().count() > 2000 { return Err("la nota deve avere da 1 a 2000 caratteri".into()) }
    app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT INTO post_notes(post_id, user_id, body, created_at) VALUES (?1, ?2, ?3, ?4)", params![post_id, me.id, body, now()]).map_err(s)?;
    Ok(p)
}

/// Rigenerazione completa: articoli letti a blocchi di 500 e generati in parallelo,
/// correlati calcolati in memoria invece che con una query per articolo.
pub fn rebuild_all(app: &App) -> R<String> {
    let gen = gen_lock();
    app.env.write().unwrap_or_else(|e| e.into_inner()).clear_templates(); // rilegge i file dei temi personalizzati
    let cx = Cx::load(app);
    let mut by_cat: HashMap<String, Vec<Value>> = HashMap::new();
    if cx.related {
        let list = query(&app.db.lock().unwrap_or_else(|e| e.into_inner()), false, "WHERE p.kind = 'post' AND p.status = 'published' AND p.published_at <= ?1 ORDER BY p.published_at DESC", &[&cx.now]);
        for p in list.iter().filter(|p| !p.category.is_empty()) {
            let v = by_cat.entry(p.category.clone()).or_default();
            if v.len() < 4 { v.push(item(&cx, p)) }
        }
    }
    let (mut last, mut total) = (0i64, 0);
    loop {
        // Blocchi di 500 in ordine di id: il "+" davanti alle colonne fa usare a SQLite la chiave primaria,
        // così ogni blocco costa 500 righe lette invece di un giro su tutto l'archivio.
        let batch = query(&app.db.lock().unwrap_or_else(|e| e.into_inner()), true, "WHERE +p.status = 'published' AND +p.published_at <= ?1 AND p.id > ?2 ORDER BY p.id LIMIT 500", &[&cx.now, &last]);
        let Some(l) = batch.last() else { break };
        (last, total) = (l.id, total + batch.len());
        par(&batch, |p| {
            let rel: Vec<Value> = if p.kind != "post" { vec![] } else {
                by_cat.get(&p.category).into_iter().flatten().filter(|r| r["id"] != p.id).take(3).cloned().collect()
            };
            write(app, &format!("/{}/", p.slug), render_post(app, &cx, p, rel)?)?;
            mark_owned(app, &p.slug);
            Ok(())
        })?;
    }
    rebuild_lists(app, &cx, None)?;
    crate::push::write_files(app); // service worker e script delle notifiche, solo se attive
    crate::indexnow::write_key_file(app); // file della chiave IndexNow alla radice, solo se attivo
    let ids: Vec<i64> = { let db = app.db.lock().unwrap_or_else(|e| e.into_inner()); let mut q = db.prepare("SELECT DISTINCT post_id FROM redirects").map_err(s)?; let v = q.query_map([], |r| r.get(0)).map_err(s)?.filter_map(Result::ok).collect(); v };
    for pid in ids {
        if pid == 0 { write_home_redirects(app, &cx); } else if let Some(p) = get_post(app, pid) { write_redirects(app, &cx, &p); }
    }
    let moves: Vec<(String, String)> = app.db.lock().unwrap_or_else(|e| e.into_inner()).prepare("SELECT old_slug, name FROM cat_moves")
        .and_then(|mut q| q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|r| r.filter_map(Result::ok).collect())).unwrap_or_default();
    for (old, name) in moves.into_iter().filter(|(old, _)| !cx.cats.contains_key(old)) { redirect_page(app, &cx, &format!("category/{old}"), &name, &cat_url(&cx.st, &name)); }
    drop(gen);
    Ok(format!("Sito rigenerato: {total} tra articoli e pagine.{}", cloudflare::purge_all(&cx.st)))
}

pub fn publish_due(app: &App, from: i64, to: i64) -> R<()> {
    let gen = gen_lock();
    let due = query(&app.db.lock().unwrap_or_else(|e| e.into_inner()), true, "WHERE p.status = 'published' AND p.published_at > ?1 AND p.published_at <= ?2", &[&from, &to]);
    if due.is_empty() { return Ok(()) }
    let cx = Cx::load(app);
    let mut urls = vec![];
    for p in &due {
        publish_one(app, &cx, p)?;
        urls.extend(affected(&cx, p));
        urls.extend(write_redirects(app, &cx, p));
    }
    rebuild_lists(app, &cx, Some(&due))?;
    drop(gen);
    cloudflare::purge(&cx.st, &urls);
    crate::google::notify(app, &due.iter().map(|p| (post_url(&cx.st, &p.slug), false)).collect::<Vec<_>>());
    crate::indexnow::submit(app, &due.iter().map(|p| post_url(&cx.st, &p.slug)).collect::<Vec<_>>());
    for p in &due { push_new(app, &cx.st, p) }
    Ok(())
}
