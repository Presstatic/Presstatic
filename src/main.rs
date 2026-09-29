mod ai;
mod files;
mod update;
mod cloudflare;
mod google;
mod media;
mod site;
mod mail;
mod twofa;
mod wpimport;
mod newsletter;
mod push;
mod backup;
mod builder;
mod indexnow;
mod forms;

use base64::Engine as _;
use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, Form, Multipart, Path, Query, Request, State},
    http::{header, HeaderMap, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{Html, IntoResponse, Json, Redirect, Response},
    routing::{get, post},
    Extension, Router,
};
use minijinja::{context, Environment, Value};
use rusqlite::{params, Connection, OptionalExtension};
use site::User;
use std::{collections::HashMap, fs, net::SocketAddr, path::PathBuf, sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex, RwLock}};

pub type Settings = HashMap<String, String>;
pub type R<T> = Result<T, String>;
pub fn s<E: std::fmt::Display>(e: E) -> String { e.to_string() }
/// Agente per le chiamate alle API (Cloudflare, Google, OpenAI, Anthropic): non segue i reindirizzamenti,
/// così una chiave API non viene mai inviata a un server diverso da quello previsto.
pub fn api_agent(secs: u64) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(secs)).redirects(0).build()
}

pub fn now() -> i64 { jiff::Timestamp::now().as_second() }
/// Valore di un'impostazione, o `d` se vuota.
pub fn opt<'a>(st: &'a Settings, k: &str, d: &'a str) -> &'a str {
    st.get(k).map(String::as_str).filter(|v| !v.is_empty()).unwrap_or(d)
}

/// Nota sui lock: ogni `lock()` usa `unwrap_or_else(|e| e.into_inner())`. Se un'operazione va in panic mentre
/// tiene un lock, il lock risulta "avvelenato"; invece di bloccare per sempre il pannello (ogni richiesta
/// successiva andrebbe in panic), si recupera il dato e si continua.
pub struct App {
    pub db: Mutex<Connection>,
    pub env: RwLock<Environment<'static>>,
    pub public: PathBuf,
    pub schemas: serde_json::Value,
    pub search_dirty: AtomicBool, // l'indice di ricerca va rifatto
    pub google_token: Mutex<Option<(String, i64)>>, // token dell'Indexing API e scadenza
    setup: Mutex<Option<String>>, // codice dell'installazione guidata, finché non esiste un amministratore
    login_fails: Mutex<HashMap<String, (u32, i64)>>, // "ip:…" o "account:…" -> (tentativi sbagliati, inizio della finestra)
    login_gate: tokio::sync::Semaphore, // quante password si controllano insieme (Argon2 è pesante di proposito)
    ai_gate: tokio::sync::Semaphore, // bozze scritte con l'IA in contemporanea
    ai_uses: Mutex<HashMap<i64, Vec<i64>>>, // utente -> momenti delle ultime bozze con l'IA
    pub update: Mutex<Option<update::Release>>, // aggiornamento disponibile, se c'è
    sessions: Mutex<HashMap<String, (i64, i64)>>, // token -> (utente, scadenza)
    pending_2fa: Mutex<HashMap<String, (i64, i64, u32)>>, // password giusta, manca il codice: token -> (utente, scadenza, tentativi)
}
impl App {
    pub fn settings(&self) -> Settings {
        let db = self.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut q = db.prepare("SELECT key, value FROM settings").unwrap();
        let v = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect();
        v
    }
}
type S = State<Arc<App>>;
type Msg = Query<HashMap<String, String>>;
type Me = Extension<User>;

const SQL: &str = "
PRAGMA journal_mode=WAL;
CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT UNIQUE NOT NULL, pass TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS posts(id INTEGER PRIMARY KEY, slug TEXT UNIQUE NOT NULL, title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '', body TEXT NOT NULL DEFAULT '', category TEXT NOT NULL DEFAULT '',
  image TEXT NOT NULL DEFAULT '', schema_type TEXT NOT NULL DEFAULT 'NewsArticle', schema_data TEXT NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'draft', published_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, author_id INTEGER);
CREATE INDEX IF NOT EXISTS posts_pub ON posts(status, published_at);
CREATE INDEX IF NOT EXISTS posts_cat ON posts(category, published_at);
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS revisions(id INTEGER PRIMARY KEY, post_id INTEGER NOT NULL, user_id INTEGER, title TEXT NOT NULL,
  description TEXT NOT NULL, body TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS revisions_post ON revisions(post_id, id);
CREATE TABLE IF NOT EXISTS index_log(id INTEGER PRIMARY KEY, url TEXT NOT NULL, kind TEXT NOT NULL, ok INTEGER NOT NULL, note TEXT NOT NULL, at INTEGER NOT NULL);
INSERT OR IGNORE INTO settings VALUES ('site_name','Il mio giornale'),('base_url','http://127.0.0.1:8080'),
  ('lang','it'),('timezone','Europe/Rome'),('per_page','20'),('ad_paragraph','3'),('accent','#c8102e'),('theme','classico'),('related','on'),('links_on','on'),('links_max','3'),('ai_text_model','anthropic:claude-sonnet-5'),('ai_image_model','gpt-image-2'),
  ('footer_columns','# Il giornale
Chi siamo | /chi-siamo/
La redazione | /autori/
Contatti | /contatti/
# Note legali
Privacy | /privacy/
Cookie policy | /cookie-policy/');";
// Colonne aggiunte nelle versioni successive: su un database esistente vengono create, altrimenti l'errore si ignora.
const MIGRATIONS: &[&str] = &[
    "ALTER TABLE posts ADD COLUMN kind TEXT NOT NULL DEFAULT 'post'",
    "ALTER TABLE posts ADD COLUMN tags TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'admin'",
    "ALTER TABLE users ADD COLUMN bio TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE users ADD COLUMN photo TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE users ADD COLUMN slug TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE posts ADD COLUMN featured INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE posts ADD COLUMN link_keywords TEXT NOT NULL DEFAULT ''",
    // Numero di versione dell'articolo: cresce a ogni salvataggio, serve a scoprire chi salva sopra il lavoro di un altro.
    "ALTER TABLE posts ADD COLUMN version INTEGER NOT NULL DEFAULT 0",
    // Chi sta modificando un articolo in questo momento (come il blocco degli articoli di WordPress).
    "CREATE TABLE IF NOT EXISTS post_locks(post_id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL, at INTEGER NOT NULL)",
    // Numero di versione dei profili, come per gli articoli.
    "ALTER TABLE users ADD COLUMN version INTEGER NOT NULL DEFAULT 0",
    // Vecchi indirizzi degli articoli (dopo un cambio di indirizzo): portano al nuovo, come i reindirizzamenti di WordPress.
    "CREATE TABLE IF NOT EXISTS redirects(slug TEXT PRIMARY KEY, post_id INTEGER NOT NULL)",
    // Verifica in due passaggi: chiave dell'app di autenticazione, stato, ultimo passo usato (un codice vale una volta).
    "ALTER TABLE users ADD COLUMN totp_secret TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE users ADD COLUMN totp_on INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE users ADD COLUMN totp_last INTEGER NOT NULL DEFAULT 0",
    "CREATE TABLE IF NOT EXISTS recovery_codes(user_id INTEGER NOT NULL, code_hash TEXT NOT NULL, used INTEGER NOT NULL DEFAULT 0)",
    // Link per reimpostare la password: nel database solo l'impronta, mai il link vero.
    "CREATE TABLE IF NOT EXISTS password_resets(token_hash TEXT PRIMARY KEY, user_id INTEGER NOT NULL, expires INTEGER NOT NULL, used INTEGER NOT NULL DEFAULT 0)",
    // Ricerca nel testo degli articoli dal pannello: indice senza copia del testo, senza accenti (città = citta).
    // Libreria media: ogni immagine caricata, con testo alternativo, didascalia e credito fotografico.
    "CREATE TABLE IF NOT EXISTS media(id INTEGER PRIMARY KEY, url TEXT UNIQUE NOT NULL, name TEXT NOT NULL DEFAULT '', w INTEGER NOT NULL DEFAULT 0, h INTEGER NOT NULL DEFAULT 0, alt TEXT NOT NULL DEFAULT '', caption TEXT NOT NULL DEFAULT '', credit TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL DEFAULT 0)",
    // Importazione da WordPress: quale articolo di WordPress è diventato quale articolo di Presstatic (per non importarlo due volte).
    "CREATE TABLE IF NOT EXISTS wp_import(wp_id INTEGER PRIMARY KEY, post_id INTEGER NOT NULL, status TEXT NOT NULL DEFAULT 'draft', applied INTEGER NOT NULL DEFAULT 0)",
    // Note della redazione sugli articoli (restano nel pannello, mai sul sito).
    "CREATE TABLE IF NOT EXISTS post_notes(id INTEGER PRIMARY KEY, post_id INTEGER NOT NULL, user_id INTEGER NOT NULL, body TEXT NOT NULL, created_at INTEGER NOT NULL)",
    "CREATE INDEX IF NOT EXISTS post_notes_post ON post_notes(post_id)",
    // Categorie aggiuntive e coautori degli articoli; albero delle categorie con descrizione; vecchi indirizzi delle categorie rinominate.
    "ALTER TABLE posts ADD COLUMN categories TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE posts ADD COLUMN coauthors TEXT NOT NULL DEFAULT ''",
    "CREATE TABLE IF NOT EXISTS categories(name TEXT PRIMARY KEY, parent TEXT NOT NULL DEFAULT '', description TEXT NOT NULL DEFAULT '')",
    "CREATE TABLE IF NOT EXISTS cat_moves(old_slug TEXT PRIMARY KEY, name TEXT NOT NULL)",
    // Commenti dei lettori (moderati) e casella «Commenti aperti» per ogni articolo.
    "CREATE TABLE IF NOT EXISTS comments(id INTEGER PRIMARY KEY, post_id INTEGER NOT NULL, parent_id INTEGER NOT NULL DEFAULT 0, name TEXT NOT NULL, email TEXT NOT NULL DEFAULT '', body TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'pending', ip_hash TEXT NOT NULL DEFAULT '', staff INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL)",
    "CREATE INDEX IF NOT EXISTS comments_post ON comments(post_id, status)",
    "ALTER TABLE posts ADD COLUMN comments INTEGER NOT NULL DEFAULT 1",
    // Dirette: 0 articolo normale, 1 diretta in corso, 2 diretta conclusa (con l'ora di chiusura); gli aggiornamenti a parte.
    "ALTER TABLE posts ADD COLUMN diretta INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE posts ADD COLUMN diretta_fine INTEGER NOT NULL DEFAULT 0",
    "CREATE TABLE IF NOT EXISTS diretta(id INTEGER PRIMARY KEY, post_id INTEGER NOT NULL, title TEXT NOT NULL DEFAULT '', body TEXT NOT NULL, key INTEGER NOT NULL DEFAULT 0, author_id INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL)",
    "CREATE INDEX IF NOT EXISTS diretta_post ON diretta(post_id, id)",
    // Newsletter: iscritti (doppia conferma) e storico degli invii.
    "CREATE TABLE IF NOT EXISTS subscribers(id INTEGER PRIMARY KEY, email TEXT UNIQUE NOT NULL, status TEXT NOT NULL DEFAULT 'pending', token TEXT UNIQUE NOT NULL, ip_hash TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL, confirmed_at INTEGER NOT NULL DEFAULT 0)",
    "CREATE TABLE IF NOT EXISTS newsletter_sends(id INTEGER PRIMARY KEY, subject TEXT NOT NULL, posts INTEGER NOT NULL, sent INTEGER NOT NULL, failed INTEGER NOT NULL, created_at INTEGER NOT NULL)",
    // Notifiche push: iscrizioni dei browser e storico degli invii.
    "CREATE TABLE IF NOT EXISTS push_subs(id INTEGER PRIMARY KEY, endpoint TEXT UNIQUE NOT NULL, p256dh TEXT NOT NULL, auth TEXT NOT NULL, ip_hash TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS push_sends(id INTEGER PRIMARY KEY, title TEXT NOT NULL, body TEXT NOT NULL, sent INTEGER NOT NULL, failed INTEGER NOT NULL, created_at INTEGER NOT NULL)",
    // Registro dei backup.
    "CREATE TABLE IF NOT EXISTS backups(id INTEGER PRIMARY KEY, name TEXT NOT NULL, size INTEGER NOT NULL, ok INTEGER NOT NULL, note TEXT NOT NULL, created_at INTEGER NOT NULL)",
    // Page builder: struttura in bozza e pubblicata di ogni pagina costruita (per ora la home).
    "CREATE TABLE IF NOT EXISTS layouts(name TEXT PRIMARY KEY, draft TEXT NOT NULL DEFAULT '', published TEXT NOT NULL DEFAULT '', updated_at INTEGER NOT NULL DEFAULT 0)",
    // Page builder: sezioni salvate dalla redazione per riusarle in altre pagine.
    "CREATE TABLE IF NOT EXISTS layout_blocks(id INTEGER PRIMARY KEY, name TEXT NOT NULL, data TEXT NOT NULL, created_at INTEGER NOT NULL)",
    // Moduli della redazione (contatti, segnalazioni…) e i messaggi ricevuti.
    "CREATE TABLE IF NOT EXISTS forms(id INTEGER PRIMARY KEY, name TEXT NOT NULL, fields TEXT NOT NULL DEFAULT '[]', notify TEXT NOT NULL DEFAULT '', success TEXT NOT NULL DEFAULT '', button TEXT NOT NULL DEFAULT '', consent INTEGER NOT NULL DEFAULT 0, consent_text TEXT NOT NULL DEFAULT '', privacy_url TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL DEFAULT 0)",
    "CREATE TABLE IF NOT EXISTS form_entries(id INTEGER PRIMARY KEY, form_id INTEGER NOT NULL, data TEXT NOT NULL, ip_hash TEXT NOT NULL DEFAULT '', seen INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL)",
    "CREATE INDEX IF NOT EXISTS form_entries_form ON form_entries(form_id, id)",
    "CREATE VIRTUAL TABLE IF NOT EXISTS posts_fts USING fts5(title, description, body, content='', contentless_delete=1, tokenize='unicode61 remove_diacritics 2')",
];
// Pagine che ogni testata deve avere: create come bozze da completare.
const DEFAULT_PAGES: &[(&str, &str, &str)] = &[
    ("chi-siamo", "Chi siamo", "<p>Racconta chi siete, da quando esiste la testata e di cosa vi occupate.</p><p>Per una testata registrata indica qui il numero di registrazione in tribunale, l'editore e il direttore responsabile.</p>"),
    ("contatti", "Contatti", "<p>Scrivi alla redazione: redazione@esempio.it</p><p>Aggiungi gli indirizzi per segnalazioni, comunicati stampa e pubblicità.</p>"),
    ("privacy", "Privacy policy", "<p>Inserisci qui l'informativa sul trattamento dei dati personali (articolo 13 del GDPR). Puoi generarla con un servizio dedicato o farla redigere da un legale.</p>"),
    ("cookie-policy", "Cookie policy", "<p>Inserisci qui la cookie policy, con l'elenco dei cookie e degli strumenti di terze parti usati dal sito: analytics, pubblicità, social.</p>"),
];

const SETTING_KEYS: &[&str] = &[
    "site_name", "base_url", "description", "lang", "timezone", "logo", "favicon", "accent", "per_page", "footer",
    "theme", "related", "links_on", "links_max", "menu", "footer_columns", "social", "head_scripts", "body_scripts",
    "comments_on", "comments_close_days", "comments_notify",
    "consent_mode", "consent_gcm", "consent_text", "consent_policy", "consent_cmp", "consent_head_cat", "consent_body_cat", "ad_list", "ad_list_every", "ad_sticky",
    "share_nets", "share_style", "share_logo_whatsapp", "share_logo_facebook", "share_logo_x", "share_logo_telegram", "share_logo_linkedin",
    "ad_head", "ad_top", "ad_inarticle", "ad_paragraph", "ad_repeat", "ad_bottom", "ads_txt", "cf_zone", "cf_token",
];
// Integrazioni: Google Indexing API e intelligenza artificiale (pagina a parte, senza rigenerare il sito).
const INTEGRATION_KEYS: &[&str] = &["openai_key", "anthropic_key", "ai_text_model", "ai_text_custom", "ai_image_model", "ai_style", "ai_roles",
    "smtp_host", "smtp_port", "smtp_security", "smtp_user", "smtp_pass", "smtp_from", "smtp_from_name", "admin_url",
    "ga_property", "most_read_on", "most_read_days", "most_read_count"];
// Chiavi e token: non tornano mai nel browser; un campo lasciato vuoto mantiene quello salvato.
const SECRETS: &[&str] = &["cf_token", "google_sa", "openai_key", "anthropic_key", "smtp_pass", "s3_secret"];

pub const THEMES: &[(&str, &str)] = &[
    ("classico", "Classico: magazine luminoso, titoli con grazie, home che cambia ritmo"),
    ("moderno", "Moderno: magazine digitale, un colore per ogni sezione, apertura a mosaico"),
];

// Modelli inclusi nel binario. Un file con lo stesso percorso in ./themes/ li sostituisce
// (per esempio themes/classico/post.html); un tema personalizzato eredita i file che non ha da "classico".
const TEMPLATES: &[(&str, &str)] = &[
    ("seo.html", include_str!("../templates/seo.html")),
    ("enhance.html", include_str!("../templates/enhance.html")),
    ("classico/base.html", include_str!("../templates/classico/base.html")),
    ("classico/post.html", include_str!("../templates/classico/post.html")),
    ("classico/list.html", include_str!("../templates/classico/list.html")),
    ("classico/page.html", include_str!("../templates/classico/page.html")),
    ("classico/search.html", include_str!("../templates/classico/search.html")),
    ("moderno/base.html", include_str!("../templates/moderno/base.html")),
    ("moderno/post.html", include_str!("../templates/moderno/post.html")),
    ("moderno/list.html", include_str!("../templates/moderno/list.html")),
    ("admin/base.html", include_str!("../templates/admin/base.html")),
    ("admin/login.html", include_str!("../templates/admin/login.html")),
    ("admin/posts.html", include_str!("../templates/admin/posts.html")),
    ("admin/edit.html", include_str!("../templates/admin/edit.html")),
    ("admin/settings.html", include_str!("../templates/admin/settings.html")),
    ("admin/users.html", include_str!("../templates/admin/users.html")),
    ("admin/user.html", include_str!("../templates/admin/user.html")),
    ("admin/revision.html", include_str!("../templates/admin/revision.html")),
    ("admin/integrations.html", include_str!("../templates/admin/integrations.html")),
    ("admin/ai.html", include_str!("../templates/admin/ai.html")),
    ("admin/setup.html", include_str!("../templates/admin/setup.html")),
    ("admin/files.html", include_str!("../templates/admin/files.html")),
    ("admin/update.html", include_str!("../templates/admin/update.html")),
    ("admin/moduli.html", include_str!("../templates/admin/moduli.html")),
    ("admin/aspetto.html", include_str!("../templates/admin/aspetto.html")),
    ("admin/modulo.html", include_str!("../templates/admin/modulo.html")),
    ("admin/modulo-messaggi.html", include_str!("../templates/admin/modulo-messaggi.html")),
    ("admin/icons.html", include_str!("../templates/admin/icons.html")),
    ("admin/login_2fa.html", include_str!("../templates/admin/login_2fa.html")),
    ("admin/recupero.html", include_str!("../templates/admin/recupero.html")),
    ("admin/twofa.html", include_str!("../templates/admin/twofa.html")),
    ("admin/media.html", include_str!("../templates/admin/media.html")),
    ("admin/importa.html", include_str!("../templates/admin/importa.html")),
    ("admin/categorie.html", include_str!("../templates/admin/categorie.html")),
    ("admin/commenti.html", include_str!("../templates/admin/commenti.html")),
    ("admin/newsletter.html", include_str!("../templates/admin/newsletter.html")),
    ("admin/push.html", include_str!("../templates/admin/push.html")),
    ("admin/backup.html", include_str!("../templates/admin/backup.html")),
    ("admin/builder.html", include_str!("../templates/admin/builder.html")),
    ("admin/google.html", include_str!("../templates/admin/google.html")),
];

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--version") | Some("-V") => { println!("Presstatic {}", update::VERSION); return }
        Some("keygen") => return update::keygen(args.get(2).cloned()),
        Some("impronta") => { println!("{}", update::fingerprint().unwrap_or_else(|| "Nessuna chiave di firma in questo programma: gli aggiornamenti dal pannello sono spenti.".into())); return }
        Some("ripristina") => return backup::restore(args.get(2).unwrap_or_else(|| { eprintln!("uso: presstatic ripristina <archivio.tar.gz>   (nella cartella del sito, con il servizio fermo)"); std::process::exit(1) })),
        Some("sign") => return update::sign(args.get(2).unwrap_or_else(|| { eprintln!("uso: presstatic sign <file> [chiave]"); std::process::exit(1) }), args.get(3).cloned()),
        _ => {}
    }
    let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.into());
    // Il database contiene password (in hash), token e chiavi API: leggibile solo dall'utente del servizio.
    let db_path = var("PRESSTATIC_DB", "presstatic.db");
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let _ = fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(&db_path);
        let db = Connection::open(&db_path).expect("impossibile aprire il database");
        db.execute_batch(SQL).expect("impossibile creare le tabelle");
        for f in [db_path.clone(), format!("{db_path}-wal"), format!("{db_path}-shm")] { let _ = fs::set_permissions(&f, fs::Permissions::from_mode(0o600)); }
    }
    let db = Connection::open(&db_path).expect("impossibile aprire il database");
    for m in MIGRATIONS { let _ = db.execute(m, []); }
    if db.query_row("SELECT 1 FROM settings WHERE key = 'seeded'", [], |_| Ok(())).is_err() {
        for (slug, title, body) in DEFAULT_PAGES {
            let _ = db.execute("INSERT OR IGNORE INTO posts(slug, title, body, status, published_at, updated_at, kind) VALUES (?1, ?2, ?3, 'draft', ?4, ?4, 'page')", params![slug, title, body, now()]);
        }
        let _ = db.execute("INSERT INTO settings VALUES ('seeded', '1')", []);
    }
    let unnamed: Vec<(i64, String)> = db.prepare("SELECT id, name FROM users WHERE slug = ''").unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect();
    for (id, name) in unnamed { let _ = db.execute("UPDATE users SET slug = ?1 WHERE id = ?2", params![site::unique_user_slug(&db, &name, id), id]); }
    if std::env::args().nth(1).as_deref() == Some("adduser") {
        return adduser(&db);
    }
    // Primo avvio con la ricerca nel testo: si indicizzano gli articoli già presenti, una volta sola.
    if db.query_row("SELECT 1 FROM settings WHERE key = 'fts_ready'", [], |_| Ok(())).is_err() {
        let n = site::fts_fill(&db);
        let _ = db.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('fts_ready', '1')", []);
        if n > 0 { eprintln!("Ricerca nel testo: indicizzati {n} articoli e pagine."); }
    }
    let mut env = Environment::new();
    env.add_global("asset_v", update::VERSION); // i file del pannello cambiano indirizzo a ogni versione
    // «tinta»: a ogni nome (di solito una sezione) uno di otto colori, sempre lo stesso. Il tema Moderno lo usa per dare
    // a ogni sezione il suo colore in etichette, testate di sezione e pagine di categoria: {{ p.category|tinta }} → 0…7.
    // Il seme 1965 dà colori tutti diversi alle otto sezioni più comuni (Cronaca, Politica, Economia, Sport, Cultura,
    // Esteri, Tecnologia, Salute); su venti nomi comuni al massimo tre condividono un colore.
    env.add_filter("tinta", |v: Value| -> u32 { (v.as_str().unwrap_or("").to_lowercase().bytes().fold(1965u32, |h, b| (h ^ b as u32).wrapping_mul(16777619)) >> 16) % 8 });
    env.set_loader(|name| {
        // Un nome di modello non può uscire dalla cartella themes/ ("..", percorsi assoluti, barre rovesciate).
        if name.starts_with('/') || name.contains('\\') || name.split('/').any(|p| p.is_empty() || p == "." || p == "..") { return Ok(None) }
        let embedded = |n: &str| TEMPLATES.iter().find(|t| t.0 == n).map(|t| t.1.to_string());
        // Le pagine del pannello vengono SOLO da quelle incluse nel programma: un tema non può sostituirle
        // (altrimenti un tema di terzi potrebbe rimpiazzare la pagina di accesso e rubare le password).
        if name.starts_with("admin/") || name == "seo.html" || name == "enhance.html" { return Ok(embedded(name)) }
        Ok(fs::read_to_string(format!("themes/{name}")).ok()
            .or_else(|| embedded(name))
            .or_else(|| name.split_once('/').and_then(|(_, f)| embedded(&format!("classico/{f}")))))
    });
    // Escape HTML senza trasformare "/" in "&#x2f;": gli indirizzi restano leggibili nel sorgente.
    env.set_formatter(|out, state, v| {
        if v.is_undefined() || v.is_none() { return Ok(()) }
        let text = v.to_string();
        let raw = v.is_safe() || matches!(state.auto_escape(), minijinja::AutoEscape::None);
        out.write_str(&if raw { text } else { site::esc(&text) })
            .map_err(|_| minijinja::Error::new(minijinja::ErrorKind::WriteFailure, "scrittura non riuscita"))
    });
    let schemas = fs::read_to_string("schemas.json").unwrap_or_else(|_| include_str!("../schemas.json").into());
    // Chiave segreta per le impronte degli IP (limiti anti-abuso): creata una volta, resta nel database.
    if db.query_row("SELECT value FROM settings WHERE key = 'ip_secret'", [], |r| r.get::<_, String>(0)).map(|v| v.len() < 32).unwrap_or(true) {
        let _ = db.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('ip_secret', ?1)", [new_token()]);
    }
    // Primo avvio senza utenti: installazione guidata dal browser, protetta da un codice
    // che solo chi ha accesso al server può leggere (nel terminale e nel file setup-token.txt).
    let setup = (db.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0)).unwrap_or(0) == 0).then(|| {
        let token: String = new_token().chars().take(24).collect();
        use std::os::unix::fs::OpenOptionsExt; // creato già leggibile solo dall'utente del servizio
        let _ = fs::remove_file("setup-token.txt");
        let _ = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open("setup-token.txt")
            .and_then(|mut f| std::io::Write::write_all(&mut f, token.as_bytes()));
        token
    });
    let app = Arc::new(App {
        db: Mutex::new(db),
        env: RwLock::new(env),
        public: var("PRESSTATIC_PUBLIC", "public").into(),
        schemas: serde_json::from_str(&schemas).expect("schemas.json non valido"),
        search_dirty: AtomicBool::new(true),
        google_token: Mutex::default(),
        sessions: Mutex::default(),
        pending_2fa: Mutex::default(),
        setup: Mutex::new(setup.clone()),
        login_fails: Mutex::default(),
        login_gate: tokio::sync::Semaphore::new(2),
        ai_gate: tokio::sync::Semaphore::new(2),
        ai_uses: Mutex::default(),
        update: Mutex::default(),
    });
    // Va guardato prima di qualsiasi pubblicazione all'avvio, che userebbe (e poi toglierebbe) lo stesso segno.
    let interrupted = std::path::Path::new(site::GEN_MARK).exists();
    site::mark_existing(&app); // siti aggiornati da una versione precedente: segna le cartelle degli articoli
    if app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT 1 FROM settings WHERE key = 'media_ready'", [], |_| Ok(())).is_err() {
        let n = site::media_backfill(&app);
        let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('media_ready', '1')", []);
        if n > 0 { eprintln!("Libreria media: aggiunte {n} immagini già presenti."); }
    }
    // Articoli programmati la cui ora è arrivata mentre il servizio era fermo (per esempio durante un aggiornamento):
    // si pubblicano all'avvio. Si riparte dall'ultimo controllo salvato ("sched_last"), mai da zero: così un riavvio
    // non ripubblica l'intero archivio e non avvisa di nuovo Google per articoli già usciti.
    let since = app.settings().get("sched_last").and_then(|v| v.parse::<i64>().ok());
    if let Some(since) = since { let _ = site::publish_due(&app, since, now()); }
    mark_scheduled(&app, now());
    tokio::spawn(scheduler(app.clone()));
    tokio::spawn(search_indexer(app.clone()));
    tokio::spawn(most_read_loop(app.clone()));
    tokio::spawn(newsletter_loop(app.clone()));
    tokio::spawn(push_loop(app.clone()));
    tokio::spawn(backup_loop(app.clone()));
    // I) Il programma si era fermato mentre scriveva pagine (spegnimento, crash): il sito si rigenera da solo,
    // così non restano elenchi o pagine a metà tra la versione vecchia e quella nuova.
    if interrupted {
        eprintln!("Il programma si era fermato mentre aggiornava il sito: lo rigenero.");
        let a = app.clone();
        tokio::task::spawn_blocking(move || match site::rebuild_all(&a) {
            Ok(m) => eprintln!("{m}"),
            Err(e) => eprintln!("Rigenerazione dopo l'interruzione non riuscita: {e}"),
        });
    }

    let admin = Router::new()
        .route("/admin", get(posts))
        .route("/admin/edit/{id}", get(edit_form).post(save))
        .route("/admin/delete/{id}", post(delete))
        .route("/admin/suggest", get(suggest))
        .route("/admin/bulk", post(bulk))
        .route("/admin/newsletter", get(nl_page).post(nl_save))
        .route("/admin/push", get(push_page).post(push_save))
        .route("/admin/backup", get(backup_page).post(backup_save))
        .route("/admin/builder", get(builder_page))
        .route("/admin/builder/frame", get(builder_frame))
        .route("/admin/builder/render", post(builder_render))
        .route("/admin/builder/salva", post(builder_save))
        .route("/admin/builder/disattiva", post(builder_off))
        .route("/admin/builder/blocchi", post(block_save))
        .route("/admin/builder/blocchi/{id}/elimina", post(block_delete))
        .route("/admin/builder/modello/{name}", get(builder_template))
        .route("/admin/backup/esegui", post(backup_now))
        .route("/admin/backup/stato", get(backup_state))
        .route("/admin/backup/prova-s3", post(backup_test_s3))
        .route("/admin/backup/scarica/{name}", get(backup_download))
        .route("/admin/backup/ripristina", post(backup_restore))
        .route("/admin/aspetto", get(appearance_page).post(appearance_save).layer(DefaultBodyLimit::max(20 * 1024 * 1024)))
        .route("/admin/backup/ripristina-file", post(backup_restore_upload).layer(DefaultBodyLimit::disable()))
        .route("/admin/push/invia", post(push_send))
        .route("/admin/newsletter/invia", post(nl_send))
        .route("/admin/newsletter/stato", get(nl_state))
        .route("/admin/newsletter/rimuovi", post(nl_remove))
        .route("/admin/newsletter/iscritti.csv", get(nl_export))
        .route("/admin/commenti", get(comments_page))
        .route("/admin/commenti/azione", post(comments_action))
        .route("/admin/commenti/{id}/rispondi", post(comment_reply))
        .route("/admin/categorie", get(categories_page).post(category_save))
        .route("/admin/categorie/rinomina", post(category_rename))
        .route("/admin/categorie/elimina", post(category_delete))
        .route("/admin/moduli", get(forms_page).post(forms_settings))
        .route("/admin/moduli/{id}", get(form_edit).post(form_save))
        .route("/admin/moduli/{id}/elimina", post(form_delete))
        .route("/admin/moduli/{id}/messaggi", get(form_entries_page).post(form_entries_delete))
        .route("/admin/moduli/{id}/csv", get(form_csv))
        .route("/admin/edit/{id}/note", post(note_add))
        .route("/admin/edit/{id}/diretta", post(live_add))
        .route("/admin/edit/{id}/diretta/stato", post(live_state))
        .route("/admin/diretta/{uid}/elimina", post(live_delete))
        .route("/admin/note/{id}/delete", post(note_delete))
        .route("/admin/lock/{id}", post(lock_ping))
        .route("/admin/lock/{id}/take", post(lock_take))
        .route("/admin/lock/{id}/release", post(lock_release))
        .route("/admin/preview/{id}", get(preview))
        .route("/admin/upload", post(upload))
        .route("/admin/revision/{id}", get(revision).post(restore))
        .route("/admin/users", get(users))
        .route("/admin/users/{id}", get(user_form).post(user_save))
        .route("/admin/profile", get(profile))
        .route("/admin/integrations", get(integrations_form).post(integrations_save).layer(DefaultBodyLimit::max(64 * 1024)))
        .route("/admin/integrations/test", post(google_test))
        .route("/admin/indicizzazione", get(google_page).post(google_save))
        .route("/admin/indicizzazione/prova", post(google_test))
        .route("/admin/indicizzazione/indexnow", post(indexnow_test))
        .route("/admin/ai", get(ai_form).post(ai_write).layer(DefaultBodyLimit::max(256 * 1024)))
        .route("/admin/files", get(files_page).post(files_action))
        .route("/admin/files/download", get(files_download))
        .route("/admin/update", get(update_page).post(update_apply))
        .route("/admin/update/check", post(update_check))
        .route("/admin/settings", get(settings_form).post(settings_save))
        .route("/admin/rebuild", post(rebuild))
        .route("/admin/cloudflare", post(cf_setup))
        .route("/admin/logout", post(logout))
        .route("/admin/2fa", get(twofa_page))
        .route("/admin/2fa/enable", post(twofa_enable))
        .route("/admin/2fa/disable", post(twofa_disable))
        .route("/admin/users/obbligo-2fa", post(twofa_required_save))
        .route("/admin/users/{id}/2fa-reset", post(twofa_reset))
        .route("/admin/integrations/email-test", post(email_test))
        .route("/admin/integrations/piu-letti", post(most_read_now))
        .route("/admin/media", get(media_page))
        .route("/admin/media/list", get(media_list))
        .route("/admin/media/upload", post(media_upload))
        .route("/admin/media/{id}", post(media_save))
        .route("/admin/media/{id}/uso", get(media_usage))
        .route("/admin/media/{id}/delete", post(media_delete))
        .route("/admin/importa", get(import_page).post(import_start).layer(DefaultBodyLimit::max(400 * 1024 * 1024)))
        .route("/admin/importa/stato", get(import_state))
        .route_layer(middleware::from_fn_with_state(app.clone(), auth));
    let router = Router::new()
        .merge(admin)
        .route("/admin/login", get(login_form).post(login).layer(DefaultBodyLimit::max(16 * 1024)))
        .route("/admin/login/2fa", get(login_2fa_form).post(login_2fa).layer(DefaultBodyLimit::max(4 * 1024)))
        .route("/admin/recupero", get(recover_form).post(recover).layer(DefaultBodyLimit::max(4 * 1024)))
        .route("/commenti/{id}", post(comment_submit).layer(DefaultBodyLimit::max(16 * 1024)))
        .route("/modulo/{id}", post(form_submit).layer(DefaultBodyLimit::max(32 * 1024)))
        .route("/newsletter/iscriviti", post(nl_subscribe).layer(DefaultBodyLimit::max(4 * 1024)))
        .route("/push/iscrivi", post(push_subscribe).layer(DefaultBodyLimit::max(4 * 1024)))
        .route("/newsletter/conferma/{token}", get(nl_confirm))
        .route("/newsletter/disiscrivi/{token}", get(nl_unsub_page).post(nl_unsub))
        .route("/admin/recupero/{token}", get(reset_form).post(reset).layer(DefaultBodyLimit::max(4 * 1024)))
        .route("/admin/setup", get(setup_form).post(setup_save))
        .route("/admin/assets/{file}", get(editor_asset))
        .route("/admin/assets/{ver}/{file}", get(editor_asset_v))
        .fallback(static_file)
        .layer(DefaultBodyLimit::max(25 * 1024 * 1024))
        .layer(middleware::from_fn(same_origin))
        .layer(middleware::from_fn(security_headers))
        .with_state(app);

    let addr = var("PRESSTATIC_ADDR", "127.0.0.1:8080");
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("porta occupata o indirizzo non valido");
    println!("Presstatic è attivo. Pannello: http://{addr}/admin");
    if let Some(t) = setup { println!("Installazione guidata: apri /admin/setup?token={t} sul dominio del pannello (il codice è anche nel file setup-token.txt)") }
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
}

// ---------- utenti e accesso ----------

fn adduser(db: &Connection) {
    let ask = |q: &str| {
        print!("{q}: ");
        std::io::Write::flush(&mut std::io::stdout()).ok();
        let mut l = String::new();
        std::io::stdin().read_line(&mut l).ok();
        l.trim().to_string()
    };
    let (name, email, pass) = (ask("Nome e cognome"), ask("Email"), ask("Password (almeno 12 caratteri)"));
    if name.is_empty() || !email.contains('@') || pass.chars().count() < 12 {
        return eprintln!("Dati non validi: servono nome, email e una password di almeno 12 caratteri.");
    }
    let slug = site::unique_user_slug(db, &name, 0);
    match db.execute("INSERT INTO users(name, email, pass, role, slug) VALUES (?1, ?2, ?3, 'admin', ?4)", params![name, email.to_lowercase(), hash_password(&pass), slug]) {
        Ok(_) => println!("Amministratore creato. Ora avvia Presstatic e accedi da /admin."),
        Err(e) => eprintln!("Utente non creato: {e}"),
    }
}

pub fn hash_password(pass: &str) -> String {
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    argon2::Argon2::default().hash_password(pass.as_bytes(), &SaltString::generate(&mut OsRng)).unwrap().to_string()
}

fn check_password(pass: &str, hash: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    PasswordHash::new(hash).is_ok_and(|h| argon2::Argon2::default().verify_password(pass.as_bytes(), &h).is_ok())
}

fn new_token() -> String {
    use argon2::password_hash::rand_core::{OsRng, RngCore};
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn https(h: &HeaderMap) -> bool { h.get("x-forwarded-proto").is_some_and(|v| v == "https") }

/// In HTTPS il cookie si chiama __Host-ps: il browser non permette a nessun sottodominio (per esempio www) di impostarlo.
fn cookie(h: &HeaderMap) -> Option<String> {
    let name = if https(h) { "__Host-ps=" } else { "ps=" };
    h.get(header::COOKIE)?.to_str().ok()?.split(';').find_map(|c| c.trim().strip_prefix(name).map(String::from))
}

fn session_cookie(t: &str, h: &HeaderMap) -> String {
    if https(h) { format!("__Host-ps={t}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=43200") }
    else { format!("ps={t}; Path=/admin; HttpOnly; SameSite=Strict; Max-Age=43200") }
}

async fn auth(State(app): S, mut req: Request, next: Next) -> Response {
    let uid = cookie(req.headers())
        .and_then(|t| app.sessions.lock().unwrap_or_else(|e| e.into_inner()).get(&t).filter(|v| v.1 > now()).map(|v| v.0));
    let user = uid.and_then(|id| site::get_user(&app.db.lock().unwrap_or_else(|e| e.into_inner()), id)).filter(|u| u.role != "disabled");
    match user {
        Some(u) => {
            // Verifica in due passaggi obbligatoria (Utenti > impostazione): redattori e amministratori senza, finché non
            // la attivano, possono aprire solo le pagine per attivarla. HTML libero + password rubata non basta più.
            if u.editor() && opt(&app.settings(), "twofa_required", "") == "on" {
                let path = req.uri().path().to_string();
                let free = path.starts_with("/admin/2fa") || path == "/admin/logout" || path.starts_with("/admin/assets/");
                let has: bool = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT totp_on FROM users WHERE id = ?1", [u.id], |r| r.get(0)).unwrap_or(false);
                if !has && !free { return back("/admin/2fa", "Per usare il pannello devi attivare la verifica in due passaggi: lo richiede l'amministratore del sito."); }
            }
            req.extensions_mut().insert(u);
            next.run(req).await
        }
        None => Redirect::to("/admin/login").into_response(),
    }
}

/// Rende obbligatoria (o no) la verifica in due passaggi per redattori e amministratori.
async fn twofa_required_save(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.admin() { return deny() }
    let on = f.get("twofa_required").is_some();
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('twofa_required', ?1)", [if on { "on" } else { "" }]);
    back("/admin/users", if on { "Verifica in due passaggi obbligatoria per redattori e amministratori: chi non l'ha ancora attivata dovrà farlo al prossimo passaggio nel pannello." } else { "Verifica in due passaggi non più obbligatoria." })
}

async fn login_form(State(app): S, Query(q): Msg) -> Response {
    if app.setup.lock().unwrap_or_else(|e| e.into_inner()).is_some() { return Redirect::to("/admin/setup").into_response() }
    render(&app, "admin/login.html", context! { msg => q.get("msg"), mail_on => mail::configured(&app.settings()) })
}

// ---------- protezioni ----------

/// CSRF: una richiesta che modifica qualcosa deve partire da una pagina del pannello stesso.
/// SameSite non basta: per il browser www.miosito.it e admin.miosito.it sono lo stesso sito,
/// quindi uno script sulle pagine pubbliche (pubblicità, statistiche) potrebbe agire al posto dell'amministratore.
/// Intestazioni di sicurezza su ogni risposta del pannello, qualunque sia il server davanti (Nginx, Apache, altro).
/// frame-ancestors 'self': nessun sito esterno può incorniciare il pannello (clickjacking), ma il pannello può
/// incorniciare sé stesso, come fa l'anteprima del page builder. Dove c'è, i browser la preferiscono a X-Frame-Options.
async fn security_headers(req: Request, next: Next) -> Response {
    let mut r = next.run(req).await;
    let h = r.headers_mut();
    // Si AGGIUNGE frame-ancestors alla policy già presente, senza sostituirla: le pagine del sito mostrate dal
    // pannello hanno «sandbox», che impedisce all'HTML caricato di eseguire codice con i privilegi del pannello.
    let csp = match h.get(header::CONTENT_SECURITY_POLICY).and_then(|v| v.to_str().ok()) {
        Some(v) if v.contains("frame-ancestors") => None,
        Some(v) => Some(format!("{}; frame-ancestors 'self'", v.trim_end_matches([';', ' ']))),
        None => Some("frame-ancestors 'self'; object-src 'none'; base-uri 'self'; form-action 'self'".to_string()),
    };
    if let Some(v) = csp.and_then(|c| header::HeaderValue::from_str(&c).ok()) { h.insert(header::CONTENT_SECURITY_POLICY, v); }
    // Nulla di ciò che risponde il programma va nei motori di ricerca: il pannello e le pagine del sito viste
    // dall'indirizzo del pannello (per l'anteprima) sarebbero copie del sito vero, che serve Nginx.
    for (k, v) in [(header::X_FRAME_OPTIONS, "SAMEORIGIN"), (header::X_CONTENT_TYPE_OPTIONS, "nosniff"), (header::REFERRER_POLICY, "same-origin"), (header::HeaderName::from_static("x-robots-tag"), "noindex, nofollow")] {
        if !h.contains_key(&k) { h.insert(k, header::HeaderValue::from_static(v)); }
    }
    r
}

async fn same_origin(req: Request, next: Next) -> Response {
    // I commenti arrivano dal sito pubblico (un altro dominio): quella rotta non usa sessione né cookie del pannello.
    if req.uri().path().starts_with("/commenti/") || req.uri().path().starts_with("/newsletter/") || req.uri().path().starts_with("/push/") { return next.run(req).await }
    if !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        let h = req.headers();
        let get = |k: header::HeaderName| h.get(k).and_then(|v| v.to_str().ok()).map(str::to_ascii_lowercase);
        // "origine" = schema + dominio + porta. L'Origin arriva completo (https://admin.sito:443); l'Host ha
        // dominio e porta, e lo schema si ricava da X-Forwarded-Proto (dietro Nginx) o dalla connessione.
        let authority = |v: &str| v.split("://").last().unwrap_or(v).split('/').next().unwrap_or("").trim_end_matches(|c: char| c == '/').to_string();
        let fetch_site = get(header::HeaderName::from_static("sec-fetch-site"));
        let cross_site = fetch_site.as_deref().is_some_and(|v| v != "same-origin" && v != "none");
        let cross_origin = match (get(header::ORIGIN), get(header::HOST)) {
            (Some(o), Some(host)) => {
                let scheme = if h.get("x-forwarded-proto").is_some_and(|v| v == "https") { "https" } else { "http" };
                let want_default = if scheme == "https" { ":443" } else { ":80" };
                let norm = |a: String| a.trim_end_matches(want_default).to_string();
                o == "null" || norm(authority(&o)) != norm(format!("{}", authority(&host))) || !o.starts_with(&format!("{scheme}://"))
            }
            _ => false,
        };
        if cross_site || cross_origin {
            return (StatusCode::FORBIDDEN, "Richiesta rifiutata: non arriva dal pannello di Presstatic.").into_response();
        }
    }
    next.run(req).await
}

/// Indirizzo del visitatore. Dietro Nginx la connessione arriva da 127.0.0.1: allora vale l'ultimo indirizzo
/// aggiunto da Nginx in X-Forwarded-For (quelli precedenti li può scrivere chiunque).
/// Dietro Cloudflare, Nginx ricava l'indirizzo reale da CF-Connecting-IP (solo dagli indirizzi di Cloudflare) e lo passa in X-Real-IP.
/// Variabili d'ambiente che servono SOLO ai test automatici (servizi finti, controlli allentati, chiave di firma di prova).
/// Sotto systemd (in produzione) vengono ignorate: nessuno può usarle per cambiare la chiave degli aggiornamenti o
/// spegnere la protezione dalle richieste verso la rete interna. systemd imposta INVOCATION_ID in ogni servizio.
pub fn test_env(name: &str) -> Option<String> {
    if std::env::var_os("INVOCATION_ID").is_some() { return None }
    std::env::var(name).ok()
}

/// Impronta di un indirizzo IP per i limiti anti-abuso: HMAC-SHA256 con una chiave segreta del sito, così
/// dall'impronta non si risale all'IP (con uno SHA-256 semplice, i 4 miliardi di IPv4 si provano in pochi secondi).
/// L'indirizzo si usa intero: prima i punti venivano tolti, e 1.23.4.5 e 12.3.4.5 davano la stessa impronta.
pub fn ip_hash(app: &App, ip: &str) -> String {
    let secret = opt(&app.settings(), "ip_secret", "").to_string();
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret.as_bytes());
    ring::hmac::sign(&key, ip.trim().as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn client_ip(peer: SocketAddr, h: &HeaderMap) -> String {
    // Dietro il proxy locale (Nginx sullo stesso server), l'unico indirizzo attendibile è X-Real-IP, che Nginx
    // scrive da CF-Connecting-IP: il client non può falsificarlo. X-Forwarded-For NON si usa, perché in fondo
    // contiene il valore ricevuto da fuori, controllabile da chi attacca (aggirava i limiti anti-forza-bruta).
    if peer.ip().is_loopback() {
        if let Some(ip) = h.get("x-real-ip").and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty() && v.len() <= 64 && v.parse::<std::net::IpAddr>().is_ok()) {
            return ip.to_string();
        }
        // Proxy che non manda X-Real-IP: un unico "dietro-proxy" prudente, così il limite per-IP resta attivo per tutti insieme.
        return "proxy".to_string();
    }
    peer.ip().to_string()
}

const FAIL_WINDOW: i64 = 15 * 60;

/// Conta il tentativo PRIMA di controllare la password, nella stessa sezione critica del controllo:
/// anche cento richieste in parallelo dallo stesso indirizzo si fermano a 10 ogni 15 minuti.
/// Restituisce i tentativi recenti sull'account, oppure None se l'indirizzo è bloccato.
/// Chiavi: [indirizzo, indirizzo+account, account]. Si blocca dopo 10 errori sulla stessa coppia
/// indirizzo+account, o dopo 50 errori dallo stesso indirizzo su account diversi: così una redazione che esce
/// da un unico indirizzo (ufficio, Cloudflare mal configurato) non resta chiusa fuori per l'errore di uno solo.
fn login_reserve(app: &App, keys: &[String; 3]) -> Option<u32> {
    let t = now();
    let mut fails = app.login_fails.lock().unwrap_or_else(|e| e.into_inner());
    fails.retain(|_, v| t - v.1 < FAIL_WINDOW);
    if fails.get(&keys[0]).is_some_and(|v| v.0 >= 50) || fails.get(&keys[1]).is_some_and(|v| v.0 >= 10) { return None }
    for k in &keys[..2] { fails.entry(k.clone()).or_insert((0, t)).0 += 1; }
    let account = fails.entry(keys[2].clone()).or_insert((0, t));
    account.0 += 1;
    Some(account.0)
}

async fn login(State(app): S, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Form(f): Form<HashMap<String, String>>) -> Response {
    let email = f.get("email").map(|e| e.trim().to_lowercase()).unwrap_or_default();
    let pass = f.get("password").cloned().unwrap_or_default();
    if email.len() > 254 || pass.len() > 1024 { return back("/admin/login", "Email o password non corrette.") }
    let ip = client_ip(peer, &headers);
    let keys = [format!("ip:{ip}"), format!("pair:{ip}|{email}"), format!("account:{email}")];
    let Some(account_tries) = login_reserve(&app, &keys) else {
        return back("/admin/login", "Troppi tentativi sbagliati da questa connessione: riprova tra 15 minuti.");
    };
    // Molti errori sull'account da indirizzi diversi: si rallenta, ma il proprietario con la password giusta entra comunque.
    if account_tries > 30 { tokio::time::sleep(std::time::Duration::from_secs(3)).await }
    let user: Option<(i64, String)> = app.db.lock().unwrap_or_else(|e| e.into_inner())
        .query_row("SELECT id, pass FROM users WHERE email = ?1 AND role <> 'disabled'", [&email], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional().ok().flatten();
    // Anche per un'email inesistente si controlla una password: i tempi di risposta non rivelano quali account esistono.
    static DUMMY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| hash_password("presstatic-confronto-a-vuoto"));
    let (id, hash) = user.map(|(i, h)| (Some(i), h)).unwrap_or((None, DUMMY.clone()));
    let valid = {
        let _turn = app.login_gate.acquire().await.unwrap(); // Argon2 fuori dai thread del server e al massimo due alla volta
        tokio::task::spawn_blocking(move || check_password(&pass, &hash)).await.unwrap_or(false)
    };
    let valid = if valid { id } else { None };
    if let Some(id) = valid {
        {
            let mut fails = app.login_fails.lock().unwrap_or_else(|e| e.into_inner());
            fails.remove(&keys[1]);
            fails.remove(&keys[2]);
            if let Some(v) = fails.get_mut(&keys[0]) { v.0 = v.0.saturating_sub(1) } // un accesso riuscito non conta
        }
        let twofa: bool = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT totp_on FROM users WHERE id = ?1", [id], |r| r.get(0)).unwrap_or(false);
        if twofa {
            // Password giusta, ora il codice dell'app: si ha 5 minuti e al massimo 5 tentativi.
            let t = new_token();
            { let mut p = app.pending_2fa.lock().unwrap_or_else(|e| e.into_inner()); p.retain(|_, v| v.1 > now()); p.insert(t.clone(), (id, now() + 300, 0)); }
            let secure = if https(&headers) { "; Secure" } else { "" };
            return ([(header::SET_COOKIE, format!("ps2={t}; Path=/admin; HttpOnly; SameSite=Strict; Max-Age=300{secure}"))], Redirect::to("/admin/login/2fa")).into_response();
        }
        return start_session(&app, id, &headers);
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await; // rallenta i tentativi a forza bruta
    back("/admin/login", "Email o password non corrette.")
}

fn start_session(app: &App, id: i64, headers: &HeaderMap) -> Response {
    let t = new_token();
    {
        let mut ss = app.sessions.lock().unwrap_or_else(|e| e.into_inner());
        ss.retain(|_, v| v.1 > now());
        ss.insert(t.clone(), (id, now() + 12 * 3600));
    }
    ([(header::SET_COOKIE, session_cookie(&t, headers))], Redirect::to("/admin")).into_response()
}

fn pending_cookie(h: &HeaderMap) -> Option<String> {
    h.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';'))
        .find_map(|c| c.trim().strip_prefix("ps2=").map(String::from)).filter(|t| t.len() >= 32)
}

async fn login_2fa_form(State(app): S, headers: HeaderMap, Query(q): Msg) -> Response {
    let ok = pending_cookie(&headers).is_some_and(|t| app.pending_2fa.lock().unwrap_or_else(|e| e.into_inner()).get(&t).is_some_and(|v| v.1 > now()));
    if !ok { return back("/admin/login", "Il tempo per inserire il codice è scaduto: accedi di nuovo.") }
    render(&app, "admin/login_2fa.html", context! { msg => q.get("msg") })
}

/// Secondo passaggio dell'accesso: codice di 6 cifre dell'app, oppure uno dei codici di recupero.
async fn login_2fa(State(app): S, headers: HeaderMap, Form(f): Form<HashMap<String, String>>) -> Response {
    let Some(t) = pending_cookie(&headers) else { return back("/admin/login", "Accedi di nuovo.") };
    let uid = {
        let mut p = app.pending_2fa.lock().unwrap_or_else(|e| e.into_inner());
        match p.get_mut(&t) {
            Some(v) if v.1 > now() && v.2 < 5 => { v.2 += 1; v.0 }
            _ => { p.remove(&t); return back("/admin/login", "Troppi codici sbagliati o tempo scaduto: accedi di nuovo.") }
        }
    };
    // Limite per ACCOUNT, non per token: rifare l'accesso con la password non dà tentativi nuovi. Senza questo, chi
    // conosce la password poteva provare codici all'infinito (5 alla volta) e indovinarne uno valido in poche ore.
    let key2 = format!("2fa:{uid}");
    let blocked = {
        let mut fails = app.login_fails.lock().unwrap_or_else(|e| e.into_inner());
        fails.retain(|_, v| now() - v.1 < FAIL_WINDOW);
        fails.get(&key2).is_some_and(|v| v.0 >= TWOFA_MAX)
    };
    if blocked {
        app.pending_2fa.lock().unwrap_or_else(|e| e.into_inner()).remove(&t);
        return back("/admin/login", "Troppi codici sbagliati per questo account: il secondo passaggio è bloccato per 15 minuti.");
    }
    let code = f.get("code").map(|c| c.trim().to_string()).unwrap_or_default();
    let ok = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let (secret, last): (String, i64) = db.query_row("SELECT totp_secret, totp_last FROM users WHERE id = ?1 AND totp_on = 1 AND role <> 'disabled'", [uid], |r| Ok((r.get(0)?, r.get(1)?))).unwrap_or_default();
        if let Some(step) = twofa::verify(&secret, &code, last, now()) {
            db.execute("UPDATE users SET totp_last = ?1 WHERE id = ?2", params![step, uid]).is_ok()
        } else {
            // codice di recupero: vale una volta sola
            db.execute("UPDATE recovery_codes SET used = 1 WHERE user_id = ?1 AND code_hash = ?2 AND used = 0", params![uid, twofa::fingerprint(&code)]).unwrap_or(0) == 1
        }
    };
    if !ok {
        let n = {
            let mut fails = app.login_fails.lock().unwrap_or_else(|e| e.into_inner());
            let e = fails.entry(key2.clone()).or_insert((0, now()));
            e.0 += 1;
            e.0
        };
        // Al raggiungimento del limite, un avviso al titolare: qualcuno conosce la sua password.
        if n == TWOFA_MAX { twofa_alert(&app, uid); }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        return back("/admin/login/2fa", "Codice non corretto: controlla l'app e riprova.");
    }
    app.login_fails.lock().unwrap_or_else(|e| e.into_inner()).remove(&key2);
    app.pending_2fa.lock().unwrap_or_else(|e| e.into_inner()).remove(&t);
    let mut r = start_session(&app, uid, &headers);
    r.headers_mut().append(header::SET_COOKIE, "ps2=; Path=/admin; Max-Age=0".parse().unwrap());
    r
}

/// Errori consentiti sul secondo passaggio, per account, ogni 15 minuti.
const TWOFA_MAX: u32 = 10;

/// Email al titolare dell'account: password giusta ma troppi codici sbagliati.
fn twofa_alert(app: &App, uid: i64) {
    let st = app.settings();
    let Some(to) = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT email FROM users WHERE id = ?1", [uid], |r| r.get::<_, String>(0)).ok() else { return };
    let site_name = opt(&st, "site_name", "Presstatic").to_string();
    let text = "Qualcuno ha inserito la password giusta del tuo account ma ha sbagliato più volte il codice di verifica in due passaggi. Per sicurezza il secondo passaggio è bloccato per 15 minuti. Se non eri tu, cambia subito la password.".to_string();
    let html = mail::layout(&site_name, "Tentativi di accesso al tuo account", &[&text], None, "La verifica in due passaggi ha fermato l'accesso: il tuo account è ancora protetto.");
    std::thread::spawn(move || { let _ = mail::send(&st, &to, "Tentativi di accesso al tuo account", &text, &html); });
}

/// Indirizzo del pannello per i link nelle email. Mai dall'intestazione Host della richiesta, che chi attacca può falsificare.
pub(crate) fn panel_url(st: &Settings) -> String {
    let set = opt(st, "admin_url", "").trim().trim_end_matches('/').to_string();
    if !set.is_empty() { return set }
    let base = site::base(st);
    match base.split_once("://www.") { Some((scheme, rest)) => format!("{scheme}://admin.{rest}"), None => base.to_string() }
}

async fn recover_form(State(app): S, Query(q): Msg) -> Response {
    render(&app, "admin/recupero.html", context! { step => "ask", mail_on => mail::configured(&app.settings()), msg => q.get("msg") })
}

/// Richiesta del link: la risposta è sempre la stessa, così non si scopre quali email hanno un account.
async fn recover(State(app): S, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Form(f): Form<HashMap<String, String>>) -> Response {
    let email = f.get("email").map(|e| e.trim().to_lowercase()).unwrap_or_default();
    let ip = client_ip(peer, &headers);
    let same = "Se l'indirizzo corrisponde a un account, entro un paio di minuti arriva un'email con il link per scegliere una nuova password. Il link vale un'ora.";
    let reserved = if email.len() > 254 { None } else { login_reserve(&app, &[format!("reset-ip:{ip}"), format!("reset-pair:{ip}|{email}"), format!("reset:{email}")]) };
    // Oltre 3 richieste in 15 minuti per lo stesso indirizzo (da qualunque IP) non parte più nessuna email: niente
    // inondazione della casella di una vittima. La risposta resta identica, per non rivelare quali account esistono.
    if reserved.is_some_and(|n| n > 3) { return back("/admin/recupero", same) }
    if reserved.is_none() {
        return back("/admin/recupero", "Troppe richieste: riprova tra 15 minuti.");
    }
    let st = app.settings();
    let user: Option<(i64, String)> = app.db.lock().unwrap_or_else(|e| e.into_inner())
        .query_row("SELECT id, name FROM users WHERE email = ?1 AND role <> 'disabled'", [&email], |r| Ok((r.get(0)?, r.get(1)?))).optional().ok().flatten();
    if let Some((uid, name)) = user {
        let token = twofa::token();
        {
            let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
            let _ = db.execute("DELETE FROM password_resets WHERE expires < ?1 OR used = 1", [now()]); // link scaduti o usati: via
            let _ = db.execute("INSERT INTO password_resets(token_hash, user_id, expires) VALUES (?1, ?2, ?3)", params![twofa::fingerprint(&token), uid, now() + 3600]);
        }
        let link = format!("{}/admin/recupero/{token}", panel_url(&st));
        let site = opt(&st, "site_name", "Presstatic").to_string();
        let html = mail::layout(&site, "Scegli una nuova password", &[&format!("Ciao {name}, abbiamo ricevuto una richiesta per reimpostare la password del pannello."), "Premi il pulsante e scegli la nuova password. Il link vale un'ora e si può usare una volta sola."], Some(("Scegli una nuova password", &link)), "Se non sei stato tu, ignora questa email: la password resta quella di prima.");
        let text = format!("Ciao {name},

per scegliere una nuova password apri questo link (vale un'ora, una volta sola):
{link}

Se non sei stato tu, ignora questa email.");
        let (st2, to) = (st.clone(), email.clone());
        tokio::task::spawn_blocking(move || { if let Err(e) = mail::send(&st2, &to, "Nuova password per il pannello", &text, &html) { eprintln!("Email di recupero non inviata: {e}") } });
    }
    back("/admin/login", same)
}

fn reset_user(app: &App, token: &str) -> Option<i64> {
    app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT user_id FROM password_resets WHERE token_hash = ?1 AND used = 0 AND expires > ?2",
        params![twofa::fingerprint(token), now()], |r| r.get(0)).ok()
}

async fn reset_form(State(app): S, Path(token): Path<String>, Query(q): Msg) -> Response {
    if reset_user(&app, &token).is_none() { return back("/admin/recupero", "Il link non è più valido: è scaduto o è già stato usato. Chiedine uno nuovo.") }
    render(&app, "admin/recupero.html", context! { step => "new", token, msg => q.get("msg") })
}

async fn reset(State(app): S, Path(token): Path<String>, Form(f): Form<HashMap<String, String>>) -> Response {
    let Some(uid) = reset_user(&app, &token) else { return back("/admin/recupero", "Il link non è più valido: chiedine uno nuovo.") };
    let (p1, p2) = (f.get("password").cloned().unwrap_or_default(), f.get("password2").cloned().unwrap_or_default());
    if p1.chars().count() < 12 || p1.len() > 1024 { return back(&format!("/admin/recupero/{token}"), "La password deve avere almeno 12 caratteri.") }
    if p1 != p2 { return back(&format!("/admin/recupero/{token}"), "Le due password non coincidono.") }
    let hash = tokio::task::spawn_blocking(move || hash_password(&p1)).await.unwrap();
    let email: Option<(String, String)> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let _ = db.execute("UPDATE users SET pass = ?1, version = version + 1 WHERE id = ?2", params![hash, uid]);
        let _ = db.execute("UPDATE password_resets SET used = 1 WHERE user_id = ?1", [uid]); // anche gli altri link chiesti prima
        db.query_row("SELECT email, name FROM users WHERE id = ?1", [uid], |r| Ok((r.get(0)?, r.get(1)?))).ok()
    };
    app.sessions.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, v| v.0 != uid); // fuori da tutti i dispositivi
    if let Some((to, name)) = email {
        let st = app.settings();
        let site = opt(&st, "site_name", "Presstatic").to_string();
        let html = mail::layout(&site, "Password cambiata", &[&format!("Ciao {name}, la password del pannello è appena stata cambiata."), "Per sicurezza sei uscito da tutti i dispositivi: accedi di nuovo con la nuova password."], None, "Se non sei stato tu, avvisa subito un amministratore.");
        tokio::task::spawn_blocking(move || { let _ = mail::send(&st, &to, "La tua password è stata cambiata", "La password del pannello è stata cambiata. Se non sei stato tu, avvisa subito un amministratore.", &html); });
    }
    back("/admin/login", "Password cambiata: accedi con quella nuova.")
}

/// Pagina della verifica in due passaggi del proprio account.
async fn twofa_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    let (on, mut secret, codes_left): (bool, String, i64) = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let (on, secret) = db.query_row("SELECT totp_on, totp_secret FROM users WHERE id = ?1", [me.id], |r| Ok((r.get::<_, bool>(0)?, r.get::<_, String>(1)?))).unwrap_or_default();
        let left = db.query_row("SELECT COUNT(*) FROM recovery_codes WHERE user_id = ?1 AND used = 0", [me.id], |r| r.get(0)).unwrap_or(0);
        (on, secret, left)
    };
    if !on && secret.is_empty() {
        // chiave nuova, in attesa di conferma: diventa attiva solo quando l'utente inserisce un codice giusto
        secret = twofa::new_secret();
        let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("UPDATE users SET totp_secret = ?1 WHERE id = ?2", params![secret, me.id]);
    }
    let st = app.settings();
    let issuer = opt(&st, "site_name", "Presstatic").to_string();
    let (qr, key) = if on { (String::new(), String::new()) } else { (twofa::qr_svg(&twofa::otpauth(&issuer, &me.email, &secret)), secret.clone()) };
    let key_spaced: String = key.chars().collect::<Vec<_>>().chunks(4).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join(" ");
    admin(&app, &me, "admin/twofa.html", context! { on, qr, key => key_spaced, codes_left, codes => Vec::<String>::new(), msg => q.get("msg") })
}

async fn twofa_enable(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    let code = f.get("code").cloned().unwrap_or_default();
    let secret: String = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT totp_secret FROM users WHERE id = ?1 AND totp_on = 0", [me.id], |r| r.get(0)).unwrap_or_default();
    let Some(step) = (!secret.is_empty()).then(|| twofa::verify(&secret, &code, 0, now())).flatten() else {
        return back("/admin/2fa", "Codice non corretto: controlla che l'ora del telefono sia giusta e inserisci il codice che vedi adesso nell'app.");
    };
    let codes = twofa::recovery_codes();
    {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let _ = db.execute("UPDATE users SET totp_on = 1, totp_last = ?1 WHERE id = ?2", params![step, me.id]);
        let _ = db.execute("DELETE FROM recovery_codes WHERE user_id = ?1", [me.id]);
        for c in &codes { let _ = db.execute("INSERT INTO recovery_codes(user_id, code_hash) VALUES (?1, ?2)", params![me.id, twofa::fingerprint(c)]); }
    }
    admin(&app, &me, "admin/twofa.html", context! { on => true, codes, codes_left => 10, msg => "Verifica in due passaggi attivata." })
}

async fn twofa_disable(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    let pass = f.get("password").cloned().unwrap_or_default();
    let hash: String = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT pass FROM users WHERE id = ?1", [me.id], |r| r.get(0)).unwrap_or_default();
    if !tokio::task::spawn_blocking(move || check_password(&pass, &hash)).await.unwrap_or(false) {
        return back("/admin/2fa", "Password non corretta: la verifica in due passaggi resta attiva.");
    }
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let _ = db.execute("UPDATE users SET totp_on = 0, totp_secret = '', totp_last = 0 WHERE id = ?1", [me.id]);
    let _ = db.execute("DELETE FROM recovery_codes WHERE user_id = ?1", [me.id]);
    back("/admin/2fa", "Verifica in due passaggi disattivata.")
}

/// Un amministratore toglie la verifica in due passaggi a chi ha perso il telefono e i codici di recupero.
async fn twofa_reset(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    if !me.admin() { return deny() }
    {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let _ = db.execute("UPDATE users SET totp_on = 0, totp_secret = '', totp_last = 0 WHERE id = ?1", [id]);
        let _ = db.execute("DELETE FROM recovery_codes WHERE user_id = ?1", [id]);
    }
    app.sessions.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, v| v.0 != id);
    back(&format!("/admin/users/{id}"), "Verifica in due passaggi tolta: al prossimo accesso basterà la password, poi potrà riattivarla dal suo profilo.")
}

async fn email_test(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    let st = app.settings();
    let site = opt(&st, "site_name", "Presstatic").to_string();
    let html = mail::layout(&site, "Le email funzionano", &["Questa è un'email di prova inviata dal pannello.", "Da ora Presstatic può mandare i link per recuperare la password."], None, "Puoi ignorare questo messaggio.");
    let to = me.email.clone();
    let r = tokio::task::spawn_blocking(move || mail::send(&st, &to, "Prova delle email del pannello", "Questa è un'email di prova inviata dal pannello di Presstatic.", &html)).await.unwrap();
    back("/admin/integrations", &match r { Ok(()) => format!("Email di prova inviata a {}: controlla la casella (anche lo spam).", me.email), Err(e) => format!("Errore: {e}.") })
}

// ---------- libreria media ----------

fn media_json(app: &App, r: (i64, String, String, i64, i64, String, String, String, i64), tz: &jiff::tz::TimeZone) -> serde_json::Value {
    let (id, url, name, w, h, alt, caption, credit, at) = r;
    serde_json::json!({ "id": id, "thumb": site::media_thumb(app, &url), "url": url, "name": name, "w": w, "h": h, "alt": alt, "caption": caption, "credit": credit, "date": site::human(at, tz) })
}

/// Immagini della libreria, dalla più recente, con ricerca in nome, testo alternativo, didascalia e credito.
async fn media_list(State(app): S, Extension(_me): Me, Query(q): Msg) -> Response {
    let words = format!("%{}%", q.get("q").map(|x| x.trim()).unwrap_or("").replace(['%', '_'], ""));
    let page: i64 = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(0).clamp(0, 10_000);
    let tz = site::tz(&app.settings());
    let (rows, total): (Vec<_>, i64) = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut st = db.prepare("SELECT id, url, name, w, h, alt, caption, credit, created_at FROM media WHERE name LIKE ?1 OR alt LIKE ?1 OR caption LIKE ?1 OR credit LIKE ?1 ORDER BY id DESC LIMIT 48 OFFSET ?2").unwrap();
        let rows = st.query_map(params![words, page * 48], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?))).unwrap().filter_map(Result::ok).collect();
        let total = db.query_row("SELECT COUNT(*) FROM media WHERE name LIKE ?1 OR alt LIKE ?1 OR caption LIKE ?1 OR credit LIKE ?1", [&words], |r| r.get(0)).unwrap_or(0);
        (rows, total)
    };
    Json(serde_json::json!({ "items": rows.into_iter().map(|r| media_json(&app, r, &tz)).collect::<Vec<_>>(), "total": total, "page": page, "pages": (total + 47) / 48 })).into_response()
}

async fn media_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    let total: i64 = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT COUNT(*) FROM media", [], |r| r.get(0)).unwrap_or(0);
    admin(&app, &me, "admin/media.html", context! { total, msg => q.get("msg") })
}

/// Caricamento dalla libreria (anche più file insieme, anche trascinandoli): ogni immagine diventa WebP in più misure.
async fn media_upload(State(app): S, Extension(_me): Me, mp: Multipart) -> Response {
    let (_, files) = read_form(mp).await;
    let a = app.clone();
    let (items, errors) = tokio::task::spawn_blocking(move || {
        let tz = site::tz(&a.settings());
        let (mut items, mut errors) = (vec![], vec![]);
        for (_, name, bytes) in files.iter().filter(|f| !f.1.is_empty()) {
            match site::save_upload(&a, name, bytes) {
                Ok(url) => {
                    let row = a.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT id, url, name, w, h, alt, caption, credit, created_at FROM media WHERE url = ?1", [&url],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)));
                    if let Ok(r) = row { items.push(media_json(&a, r, &tz)) }
                }
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        (items, errors)
    }).await.unwrap();
    Json(serde_json::json!({ "items": items, "errors": errors })).into_response()
}

/// Testo alternativo, didascalia e credito. Le foto in evidenza che la usano si aggiornano da sole sul sito.
async fn media_save(State(app): S, Extension(me): Me, Path(id): Path<i64>, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let v = |k: &str| f.get(k).map(|x| x.trim().chars().take(if k == "caption" { 600 } else { 300 }).collect::<String>()).unwrap_or_default();
    let url: Option<String> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let _ = db.execute("UPDATE media SET alt = ?1, caption = ?2, credit = ?3 WHERE id = ?4", params![v("alt"), v("caption"), v("credit"), id]);
        db.query_row("SELECT url FROM media WHERE id = ?1", [id], |r| r.get(0)).ok()
    };
    if let Some(url) = url { let a = app.clone(); tokio::task::spawn_blocking(move || site::refresh_featured(&a, &url)); }
    Json(serde_json::json!({ "ok": true })).into_response()
}

async fn media_usage(State(app): S, Extension(_me): Me, Path(id): Path<i64>) -> Response {
    let st = app.settings();
    let used = site::media_usage(&app, id).into_iter().map(|(pid, title, slug)| serde_json::json!({ "id": pid, "title": title, "url": site::post_url(&st, &slug) })).collect::<Vec<_>>();
    Json(serde_json::json!({ "used": used })).into_response()
}

/// Elimina un'immagine con tutte le sue misure, solo se nessun articolo la usa.
async fn media_delete(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    if !me.editor() { return deny() }
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || site::media_delete(&a, id)).await.unwrap();
    Json(match r { Ok(()) => serde_json::json!({ "ok": true }), Err(e) => serde_json::json!({ "error": e }) }).into_response()
}

// ---------- importazione da WordPress ----------

async fn import_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let state = wpimport::STATE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    admin(&app, &me, "admin/importa.html", context! { state, msg => q.get("msg") })
}

async fn import_state(State(_app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    Json(wpimport::STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()).into_response()
}

async fn import_start(State(app): S, Extension(me): Me, mp: Multipart) -> Response {
    if !me.admin() { return deny() }
    let (f, files) = read_form(mp).await;
    let Some((_, _, bytes)) = files.into_iter().find(|x| x.0 == "wxr" && !x.2.is_empty()) else { return back("/admin/importa", "Errore: scegli il file di esportazione di WordPress (.xml).") };
    let Ok(xml) = String::from_utf8(bytes) else { return back("/admin/importa", "Errore: il file non è un XML di WordPress (testo UTF-8).") };
    {
        let mut s = wpimport::STATE.lock().unwrap_or_else(|e| e.into_inner());
        if s.running { return back("/admin/importa", "Errore: un'importazione è già in corso.") }
        *s = wpimport::State { running: true, phase: "Avvio".into(), ..Default::default() };
    }
    let o = wpimport::Options { images: f.get("images").is_some(), authors: f.get("authors").is_some(), owner: me.id };
    let a = app.clone();
    std::thread::spawn(move || wpimport::run(&a, xml, o));
    Redirect::to("/admin/importa").into_response()
}

async fn logout(State(app): S, headers: HeaderMap) -> Response {
    if let Some(t) = cookie(&headers) {
        app.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&t);
    }
    let clear = if https(&headers) { "__Host-ps=; Path=/; Secure; Max-Age=0" } else { "ps=; Path=/admin; Max-Age=0" };
    ([(header::SET_COOKIE, clear)], Redirect::to("/admin/login")).into_response()
}

// ---------- pannello ----------

/// Ogni pagina del pannello riceve un «nonce» nuovo: il browser esegue solo gli script scritti da noi con quel
/// lasciapassare, e quelli serviti dal pannello stesso. Uno script infilato da un attaccante (per un errore futuro nel
/// codice) non avrebbe il nonce giusto e non partirebbe: è la seconda rete, dopo l'escape automatico dei modelli.
fn render(app: &App, name: &str, ctx: Value) -> Response {
    let mut b = [0u8; 18];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b);
    let nonce = base64::engine::general_purpose::STANDARD.encode(b);
    let ctx = minijinja::value::merge_maps([ctx, context! { csp_nonce => nonce.clone() }]); // vince l'ultimo: il nonce
    match app.env.read().unwrap_or_else(|e| e.into_inner()).get_template(name).and_then(|t| t.render(ctx)) {
        Ok(h) => {
            let mut r = Html(h).into_response();
            let csp = format!("script-src 'self' 'nonce-{nonce}'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'self'");
            if let Ok(v) = header::HeaderValue::from_str(&csp) { r.headers_mut().insert(header::CONTENT_SECURITY_POLICY, v); }
            r
        }
        Err(e) => {
            eprintln!("Errore nel modello {name}: {e:#}"); // il dettaglio va nei log del server, non in pagina
            (StatusCode::INTERNAL_SERVER_ERROR, "Errore interno nella pagina.").into_response()
        }
    }
}

/// Pagina del pannello: ogni modello riceve l'utente collegato e, per i redattori, gli articoli da rivedere.
fn admin(app: &App, me: &User, name: &str, ctx: Value) -> Response {
    let pending: i64 = if me.editor() { app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT COUNT(*) FROM posts WHERE status = 'pending'", [], |r| r.get(0)).unwrap_or(0) } else { 0 };
    let update = me.admin() && app.update.lock().unwrap_or_else(|e| e.into_inner()).is_some();
    let to_moderate: i64 = if me.editor() { app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT COUNT(*) FROM comments WHERE status = 'pending'", [], |r| r.get(0)).unwrap_or(0) } else { 0 };
    let forms_unread: i64 = if me.editor() { app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT COUNT(*) FROM form_entries WHERE seen = 0", [], |r| r.get(0)).unwrap_or(0) } else { 0 };
    render(app, name, context! { me, pending, update, to_moderate, forms_unread, ..ctx })
}

fn deny() -> Response {
    (StatusCode::FORBIDDEN, Html("<p style=\"font:16px system-ui;margin:3rem\">Non hai i permessi per questa pagina. <a href=\"/admin\">Torna agli articoli</a></p>")).into_response()
}

fn back(path: &str, msg: &str) -> Response {
    let enc: String = msg.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    Redirect::to(&format!("{path}?msg={enc}")).into_response()
}

/// Legge un modulo multipart: campi di testo e file (nome campo, nome file, contenuto).
async fn read_form(mut mp: Multipart) -> (HashMap<String, String>, Vec<(String, String, Vec<u8>)>) {
    let (mut fields, mut files) = (HashMap::new(), vec![]);
    while let Ok(Some(field)) = mp.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        match field.file_name().map(String::from) {
            Some(file) => if let Ok(b) = field.bytes().await { if !b.is_empty() { files.push((name, file, b.to_vec())) } },
            None => if let Ok(t) = field.text().await { fields.insert(name, t); },
        }
    }
    (fields, files)
}

async fn posts(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    let st = app.settings();
    let tz = site::tz(&st);
    let kind = if q.get("k").map(String::as_str) == Some("page") && me.editor() { "page" } else { "post" };
    // Ricerca nel titolo, nel sommario e nel testo (indice FTS5, senza accenti, parole anche iniziate): «ponte can» trova «ponte sul canale».
    let words: Vec<String> = q.get("q").map(|x| x.split_whitespace().take(8).map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>()).filter(|w| !w.is_empty()).collect()).unwrap_or_default();
    let search = words.iter().map(|w| format!("\"{w}\"*")).collect::<Vec<_>>().join(" ");
    let matching = if words.is_empty() { "?1 = ?1" } else { "p.id IN (SELECT rowid FROM posts_fts WHERE posts_fts MATCH ?1)" };
    let tab = q.get("s").map(String::as_str).unwrap_or("");
    let filter = match tab {
        "draft" => "p.status = 'draft'",
        "pending" => "p.status = 'pending'",
        "sched" => "p.status = 'published' AND p.published_at > ?2",
        "pub" => "p.status = 'published' AND p.published_at <= ?2",
        _ => "?2 = ?2",
    };
    // Gli autori vedono solo i propri articoli.
    let mine = if me.editor() { "?3 = ?3".to_string() } else { "p.author_id = ?3".to_string() };
    let t = now();
    let (list, counts, editing, notes) = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let list = site::query(&db, false, &format!("WHERE p.kind = '{kind}' AND {matching} AND {filter} AND {mine} ORDER BY p.published_at DESC LIMIT 300"), &[&search, &t, &me.id]);
        let mut c = db.prepare(&format!("SELECT CASE WHEN status IN ('draft', 'pending') THEN status WHEN published_at > ?1 THEN 'sched' ELSE 'pub' END, COUNT(*) FROM posts p WHERE kind = '{kind}' AND {} GROUP BY 1", mine.replace("?3", "?2"))).unwrap();
        let counts: HashMap<String, i64> = c.query_map(params![t, me.id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect();
        let mut l = db.prepare("SELECT l.post_id, u.name FROM post_locks l JOIN users u ON u.id = l.user_id WHERE l.at > ?1 AND l.user_id <> ?2").unwrap();
        let editing: HashMap<i64, String> = l.query_map(params![t - site::LOCK_WINDOW, me.id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect();
        let mut nq = db.prepare("SELECT post_id, COUNT(*) FROM post_notes GROUP BY post_id").unwrap();
        let notes: HashMap<i64, i64> = nq.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect();
        (list, counts, editing, notes)
    };
    let rows: Vec<_> = list.iter().map(|p| serde_json::json!({
        "id": p.id, "title": p.title, "category": p.category, "date": site::human(p.published_at, &tz), "author": p.author,
        "url": site::post_url(&st, &p.slug), "image": p.image, "editing": editing.get(&p.id), "notes": notes.get(&p.id),
        "state": match p.status.as_str() { "draft" => "Bozza", "pending" => "In revisione", _ if p.published_at > t => "Programmato", _ => "Pubblicato" },
    })).collect();
    let total: i64 = counts.values().sum();
    admin(&app, &me, "admin/posts.html", context! { rows, counts, total, tab, kind, q => q.get("q"), msg => q.get("msg") })
}

async fn edit_form(State(app): S, Extension(me): Me, Path(id): Path<i64>, Query(q): Msg) -> Response {
    let st = app.settings();
    let kind = if q.get("k").map(String::as_str) == Some("page") && me.editor() { "page" } else { "post" };
    let p = if id == 0 {
        site::Post { schema_type: "NewsArticle".into(), schema_data: "{}".into(), status: "draft".into(), published_at: now(), kind: kind.into(), author_id: me.id, ..Default::default() }
    } else {
        match site::get_post(&app, id) { Some(p) => p, None => return (StatusCode::NOT_FOUND, "Articolo non trovato").into_response() }
    };
    if id > 0 && !me.can_edit(&p) {
        return back("/admin", "Questo articolo è già pubblicato o non è tuo: per modificarlo chiedi a un redattore.");
    }
    let (cats, authors, revisions) = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let col = |sql: &str| -> Vec<String> { db.prepare(sql).unwrap().query_map([], |r| r.get(0)).unwrap().filter_map(Result::ok).collect() };
        let cats = col("SELECT name FROM categories UNION SELECT DISTINCT category FROM posts WHERE category <> '' ORDER BY 1 COLLATE NOCASE");
        let mut a = db.prepare("SELECT id, name FROM users WHERE role <> 'disabled' ORDER BY name").unwrap();
        let authors: Vec<(i64, String)> = a.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect();
        let mut r = db.prepare("SELECT r.id, r.created_at, COALESCE(u.name, '') FROM revisions r LEFT JOIN users u ON u.id = r.user_id WHERE r.post_id = ?1 ORDER BY r.id DESC LIMIT 15").unwrap();
        let tz = site::tz(&st);
        let revisions: Vec<(i64, String, String)> = r.query_map([id], |r| Ok((r.get(0)?, site::human(r.get(1)?, &tz), r.get(2)?))).unwrap().filter_map(Result::ok).collect();
        (cats, authors, revisions)
    };
    let sd: serde_json::Value = serde_json::from_str(&p.schema_data).unwrap_or_default();
    let date_local = site::local(p.published_at, &site::tz(&st));
    let url = site::post_url(&st, &p.slug);
    // Blocco come in WordPress: se un'altra persona ha l'articolo aperto, lo dico prima che inizi a scrivere;
    // altrimenti l'articolo passa a me finché l'editor resta aperto.
    let locked = if id > 0 { site::lock_holder(&app, id, me.id) } else { None };
    if id > 0 && locked.is_none() { site::take_lock(&app, id, me.id) }
    let (locked_by, locked_secs) = locked.map(|(n, s)| (Some(n), s)).unwrap_or((None, 0));
    let version = if id > 0 { site::post_version(&app, id) } else { 0 };
    let live = id > 0 && p.status == "published" && p.published_at <= now(); // online adesso: all'eliminazione si chiede dove portare i lettori
    let notes = if id > 0 { site::notes(&app, id, &site::tz(&st)) } else { vec![] };
    let cat_tree = { let a = app.clone(); tokio::task::spawn_blocking(move || site::category_tree(&a)).await.unwrap() };
    let (diretta, aggiornamenti): (i64, Vec<serde_json::Value>) = if id == 0 { (0, vec![]) } else {
        let tz = site::tz(&st);
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let d: i64 = db.query_row("SELECT diretta FROM posts WHERE id = ?1", [id], |r| r.get(0)).unwrap_or(0);
        let list = db.prepare("SELECT id, title, body, key, created_at FROM diretta WHERE post_id = ?1 ORDER BY id DESC LIMIT 300").and_then(|mut q| q.query_map([id], |r| Ok(serde_json::json!({
            "id": r.get::<_, i64>(0)?, "title": r.get::<_, String>(1)?, "body": r.get::<_, String>(2)?, "key": r.get::<_, i64>(3)? == 1, "at": site::human(r.get::<_, i64>(4)?, &tz)}))).map(|r| r.filter_map(Result::ok).collect())).unwrap_or_default();
        (d, list)
    };
    admin(&app, &me, "admin/edit.html", context! {
        diretta, aggiornamenti,
        post => p, sd, cats, authors, revisions, schemas => &app.schemas, date_local, url, msg => q.get("msg"),
        base => site::base(&st), site_name => opt(&st, "site_name", ""), locked_by, locked_secs, version,
        live, notes, cat_tree,
    })
}

// ---------- dirette ----------
fn live_json(v: serde_json::Value) -> Response { axum::Json(v).into_response() }

async fn live_add(State(app): S, Extension(me): Me, Path(id): Path<i64>, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let body = f.get("body").map(|s| s.trim().replace("\r\n", "\n")).unwrap_or_default();
    let title: String = f.get("title").map(|s| s.trim().chars().take(140).collect()).unwrap_or_default();
    if body.is_empty() || body.chars().count() > 5000 { return live_json(serde_json::json!({"ok": false, "error": "Scrivi l'aggiornamento (al massimo 5000 caratteri)."})) }
    let key = f.get("key").is_some_and(|v| v == "on");
    let uid = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let d: i64 = db.query_row("SELECT diretta FROM posts WHERE id = ?1", [id], |r| r.get(0)).unwrap_or(-1);
        if d != 1 { return live_json(serde_json::json!({"ok": false, "error": "La diretta non è in corso."})) }
        let _ = db.execute("INSERT INTO diretta(post_id, title, body, key, author_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![id, title, body, key as i64, me.id, now()]);
        db.last_insert_rowid()
    };
    let a = app.clone();
    let _ = tokio::task::spawn_blocking(move || site::live_touch(&a, id)).await;
    live_json(serde_json::json!({"ok": true, "id": uid, "at": site::human(now(), &site::tz(&app.settings()))}))
}

async fn live_state(State(app): S, Extension(me): Me, Path(id): Path<i64>, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let (state, end) = match f.get("action").map(String::as_str) { Some("start") | Some("reopen") => (1, 0), Some("end") => (2, now()), _ => return live_json(serde_json::json!({"ok": false, "error": "Azione non valida."})) };
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("UPDATE posts SET diretta = ?1, diretta_fine = ?2 WHERE id = ?3", params![state, end, id]);
    let a = app.clone();
    let _ = tokio::task::spawn_blocking(move || site::live_touch(&a, id)).await;
    live_json(serde_json::json!({"ok": true, "state": state}))
}

async fn live_delete(State(app): S, Extension(me): Me, Path(uid): Path<i64>) -> Response {
    if !me.editor() { return deny() }
    let post: i64 = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let p = db.query_row("SELECT post_id FROM diretta WHERE id = ?1", [uid], |r| r.get(0)).unwrap_or(0);
        let _ = db.execute("DELETE FROM diretta WHERE id = ?1", [uid]);
        p
    };
    if post > 0 { let a = app.clone(); let _ = tokio::task::spawn_blocking(move || site::live_touch(&a, post)).await; }
    live_json(serde_json::json!({"ok": true}))
}

/// Articoli e pagine online che contengono le parole nel titolo: per scegliere dove portare i lettori di un articolo eliminato.
async fn suggest(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.editor() { return deny() }
    let words = format!("%{}%", q.get("q").map(|x| x.trim()).unwrap_or("").replace(['%', '_'], ""));
    let except: i64 = q.get("except").and_then(|x| x.parse().ok()).unwrap_or(0);
    let st = app.settings();
    let list = site::query(&app.db.lock().unwrap_or_else(|e| e.into_inner()), false,
        "WHERE p.status = 'published' AND p.published_at <= ?1 AND p.title LIKE ?2 AND p.id <> ?3 ORDER BY p.published_at DESC LIMIT 8", &[&now(), &words, &except]);
    Json(list.iter().map(|p| serde_json::json!({ "id": p.id, "title": p.title, "url": site::post_url(&st, &p.slug), "page": p.kind == "page" })).collect::<Vec<_>>()).into_response()
}

// ---------- commenti ----------

/// Commento inviato dal sito pubblico: si salva in attesa di approvazione e si torna all'articolo.
async fn comment_submit(State(app): S, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Path(id): Path<i64>, Form(f): Form<HashMap<String, String>>) -> Response {
    let ip = client_ip(peer, &headers);
    let a = app.clone();
    match tokio::task::spawn_blocking(move || site::comment_submit(&a, id, &f, &ip)).await.unwrap() {
        Ok((back_to, notify)) => {
            let st = app.settings();
            if let (Some((title, name)), true) = (notify, mail::configured(&st)) {
                let to = opt(&st, "comments_notify", "").trim().to_string();
                if to.contains('@') {
                    let link = format!("{}/admin/commenti", panel_url(&st));
                    let site = opt(&st, "site_name", "Presstatic").to_string();
                    let html = mail::layout(&site, "Un commento da approvare", &[&format!("{name} ha commentato «{title}».")], Some(("Apri i commenti", &link)), "Finché non lo approvi, il commento non compare sul sito.");
                    tokio::task::spawn_blocking(move || { let _ = mail::send(&st, &to, &format!("Nuovo commento su «{title}»"), &format!("{name} ha commentato «{title}». Moderazione: {link}"), &html); });
                }
            }
            Redirect::to(&back_to).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Html(format!("<!doctype html><html lang=\"it\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Commento non inviato</title><body style=\"font-family:system-ui,sans-serif;max-width:36rem;margin:3rem auto;padding:0 1rem;line-height:1.5\"><h1>Commento non inviato</h1><p>{}.</p><p><a href=\"javascript:history.back()\">Torna indietro</a> per correggerlo: il testo che hai scritto è ancora lì.</p></body></html>", site::esc(&e)))).into_response(),
    }
}

async fn comments_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.editor() { return deny() }
    let tab = match q.get("s").map(String::as_str) { Some("approved") => "approved", Some("spam") => "spam", _ => "pending" };
    let st = app.settings();
    let tz = site::tz(&st);
    let (rows, counts) = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut qy = db.prepare("SELECT c.id, c.name, c.email, c.body, c.created_at, c.staff, c.parent_id, p.id, p.title, p.slug FROM comments c JOIN posts p ON p.id = c.post_id WHERE c.status = ?1 ORDER BY c.id DESC LIMIT 200").unwrap();
        let rows: Vec<serde_json::Value> = qy.query_map([tab], |r| Ok(serde_json::json!({
            "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "email": r.get::<_, String>(2)?, "body": r.get::<_, String>(3)?,
            "date": site::human(r.get(4)?, &tz), "staff": r.get::<_, bool>(5)?, "reply": r.get::<_, i64>(6)? > 0,
            "post_id": r.get::<_, i64>(7)?, "post": r.get::<_, String>(8)?, "url": site::post_url(&st, &r.get::<_, String>(9)?),
        }))).unwrap().filter_map(Result::ok).collect();
        let mut c = db.prepare("SELECT status, COUNT(*) FROM comments GROUP BY status").unwrap();
        let counts: HashMap<String, i64> = c.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect();
        (rows, counts)
    };
    admin(&app, &me, "admin/commenti.html", context! { rows, counts, tab, on => site::comments_on(&st), msg => q.get("msg") })
}

async fn comments_action(State(app): S, Extension(me): Me, body: axum::body::Bytes) -> Response {
    if !me.editor() { return deny() }
    let pairs: Vec<(String, String)> = form_urlencoded::parse(&body).into_owned().collect();
    let ids: Vec<i64> = pairs.iter().filter(|(k, _)| k == "ids").filter_map(|(_, v)| v.parse().ok()).collect();
    let get = |k: &str| pairs.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let (action, tab) = (get("action"), get("s"));
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || site::comment_action(&a, &ids, &action)).await.unwrap();
    back(&format!("/admin/commenti{}", if tab.is_empty() { String::new() } else { format!("?s={}", urlencoding(&tab)) }), &r.unwrap_or_else(|e| format!("Errore: {e}.")))
}

async fn comment_reply(State(app): S, Extension(me): Me, Path(id): Path<i64>, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let body = f.get("body").cloned().unwrap_or_default();
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || site::comment_reply(&a, &me, id, &body)).await.unwrap();
    back("/admin/commenti?s=approved", &r.unwrap_or_else(|e| format!("Errore: {e}.")))
}

// ---------- page builder ----------

/// Quale pagina si costruisce: (nome interno, nome nell'indirizzo, titolo, modelli disponibili).
/// «p12» è la pagina singola con id 12 (per esempio «Chi siamo»).
fn which(app: &App, q: &HashMap<String, String>) -> (String, String, String, serde_json::Value) {
    let s = |a: &str, b: &str, c: &str, t: serde_json::Value| (a.to_string(), b.to_string(), c.to_string(), t);
    match q.get("pagina").map(String::as_str) {
        Some("testata") => s("header", "testata", "Testata e menu", serde_json::json!([["classica", "Logo a sinistra, ricerca a destra, menu sotto"], ["centrata", "Logo al centro, menu su fascia scura"]])),
        Some("piede") => s("footer", "piede", "Piè di pagina", serde_json::json!([["scuro", "Tre colonne su fondo scuro, copyright sotto"]])),
        Some(x) if x.starts_with('p') && x[1..].parse::<i64>().is_ok() => match site::get_post(app, x[1..].parse().unwrap_or(0)).filter(|p| p.kind == "page") {
            Some(p) => (format!("page:{}", p.id), x.to_string(), format!("Pagina «{}»", p.title), serde_json::json!([["pagina", "Chi siamo: titolo, testo, foto, numeri, domande"], ["vuoto", "Solo il titolo"]])),
            None => s("home", "home", "Home", serde_json::json!([["giornale", "Giornale"]])),
        },
        _ => s("home", "home", "Home", serde_json::json!([["giornale", "Giornale: apertura e ultime notizie"], ["rivista", "Rivista: foto grande e griglie"], ["vuoto", "Pagina vuota"]])),
    }
}

fn layout(app: &App, name: &str, col: &str) -> Option<serde_json::Value> {
    let q = format!("SELECT {col} FROM layouts WHERE name = ?1");
    app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row(&q, [name], |r| r.get::<_, String>(0)).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// Struttura valida: un oggetto con le sezioni, di dimensione ragionevole.
fn layout_ok(v: &serde_json::Value) -> bool { v["sections"].is_array() && v.to_string().len() < 512 * 1024 }

async fn builder_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let (name, qname, title, templates) = which(&app, &q);
    let published = layout(&app, &name, "published").is_some();
    let first = templates[0][0].as_str().unwrap_or("").to_string();
    let doc = layout(&app, &name, "draft").or_else(|| layout(&app, &name, "published")).unwrap_or_else(|| {
        let mut d = builder::template(&first, &name);
        // pagina nuova: il titolo del modello diventa quello vero della pagina
        if let Some(p) = name.strip_prefix("page:").and_then(|x| x.parse().ok()).and_then(|id| site::get_post(&app, id)) { d["sections"][0]["columns"][0]["widgets"][0]["settings"]["text"] = serde_json::json!(p.title); }
        d
    });
    let a = app.clone();
    let cats: Vec<String> = tokio::task::spawn_blocking(move || site::category_tree(&a)).await.unwrap().iter().filter_map(|c| c["name"].as_str().map(String::from)).collect();
    let forms_list: Vec<serde_json::Value> = { let db = app.db.lock().unwrap_or_else(|e| e.into_inner()); forms::all(&db).iter().map(|f| serde_json::json!([f.id, f.name])).collect() };
    let data = serde_json::json!({ "doc": doc, "cats": cats, "name": name, "qname": qname, "templates": templates, "blocks": blocks(&app), "forms": forms_list }).to_string().replace("</", "<\\/");
    admin(&app, &me, "admin/builder.html", context! { data, published, name, qname, title, base => site::base(&app.settings()), msg => q.get("msg") })
}

/// Anteprima dentro l'editor: la pagina vera nel tema del sito, con lo script dell'editor aggiunto (solo qui).
async fn builder_frame(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let (name, _, _, t) = which(&app, &q);
    let doc = layout(&app, &name, "draft").or_else(|| layout(&app, &name, "published")).unwrap_or_else(|| builder::template(t[0][0].as_str().unwrap_or(""), &name));
    let a = app.clone();
    match tokio::task::spawn_blocking(move || site::builder_preview(&a, &doc, true, &name)).await.unwrap() {
        Ok(html) => {
            let tools = format!("<link rel=\"stylesheet\" href=\"/admin/assets/{v}/pb-frame.css\"><script src=\"/admin/assets/{v}/pb-frame.js\" defer></script></body>", v = update::VERSION);
            // L'anteprima è sul dominio del pannello: qui può girare SOLO lo script dell'editor (servito dal pannello).
            // Script del tema, annunci, statistiche o codice incollato non partono: avrebbero i privilegi dell'amministratore.
            ([(header::CONTENT_SECURITY_POLICY, "script-src 'self'; object-src 'none'; base-uri 'self'; form-action 'none'")], Html(html.replacen("</body>", &tools, 1))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn builder_render(State(app): S, Extension(me): Me, Query(q): Msg, Json(doc): Json<serde_json::Value>) -> Response {
    if !me.admin() || !layout_ok(&doc) { return deny() }
    let (name, ..) = which(&app, &q);
    let a = app.clone();
    match tokio::task::spawn_blocking(move || site::builder_preview(&a, &doc, false, &name)).await.unwrap() {
        Ok(html) => Json(serde_json::json!({ "html": html })).into_response(),
        Err(e) => Json(serde_json::json!({ "error": e })).into_response(),
    }
}

/// Salva la bozza; con «Pubblica» il sito cambia: la home si rifà da sola, testata e piè di pagina rifanno tutte le pagine.
async fn builder_save(State(app): S, Extension(me): Me, Query(q): Msg, Json(v): Json<serde_json::Value>) -> Response {
    if !me.admin() { return deny() }
    let (name, ..) = which(&app, &q);
    let doc = v["doc"].clone();
    if !layout_ok(&doc) { return Json(serde_json::json!({ "error": "Struttura della pagina non valida." })).into_response() }
    let publish = v["publish"].as_bool().unwrap_or(false);
    {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let _ = db.execute("INSERT INTO layouts(name, draft, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(name) DO UPDATE SET draft = ?2, updated_at = ?3", params![name, doc.to_string(), now()]);
        if publish { let _ = db.execute("UPDATE layouts SET published = draft WHERE name = ?1", [&name]); }
    }
    if !publish { return Json(serde_json::json!({ "msg": "Bozza salvata: il sito non cambia finché non pubblichi." })).into_response() }
    let a = app.clone();
    let n2 = name.clone();
    let r = tokio::task::spawn_blocking(move || republish(&a, &n2)).await.unwrap();
    let done = if name == "home" { "la home del sito è stata aggiornata" } else if name.starts_with("page:") { "la pagina è stata aggiornata" } else { "tutte le pagine del sito sono state aggiornate" };
    Json(match r { Ok(()) => serde_json::json!({ "msg": format!("Pubblicata: {done}.") }), Err(e) => serde_json::json!({ "error": e }) }).into_response()
}

/// Torna alla versione del tema (la composizione resta salvata come bozza).
async fn builder_off(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.admin() { return deny() }
    let (name, qname, ..) = which(&app, &f);
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("UPDATE layouts SET published = '' WHERE name = ?1", [&name]);
    let a = app.clone();
    let _ = tokio::task::spawn_blocking(move || republish(&a, &name)).await;
    back(&format!("/admin/builder?pagina={qname}"), "Il sito è tornato alla versione del tema. La tua composizione resta salvata qui come bozza.")
}

/// Sezioni salvate («I miei blocchi»), dalla più recente.
fn blocks(app: &App) -> Vec<serde_json::Value> {
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(mut q) = db.prepare("SELECT id, name, data FROM layout_blocks ORDER BY id DESC LIMIT 200") else { return vec![] };
    let v = q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).map(|r| r.filter_map(Result::ok)
        .filter_map(|(id, name, data)| Some(serde_json::json!({ "id": id, "name": name, "data": serde_json::from_str::<serde_json::Value>(&data).ok()? }))).collect()).unwrap_or_default();
    v
}

/// Salva una sezione con un nome, per riusarla in qualsiasi pagina costruita.
async fn block_save(State(app): S, Extension(me): Me, Json(v): Json<serde_json::Value>) -> Response {
    if !me.admin() { return deny() }
    let name: String = v["name"].as_str().unwrap_or("").trim().chars().take(80).collect();
    let section = v["section"].clone();
    if name.is_empty() || !section["columns"].is_array() || section.to_string().len() > 128 * 1024 {
        return Json(serde_json::json!({ "error": "Dai un nome al blocco (e scegli una sezione)." })).into_response();
    }
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT INTO layout_blocks(name, data, created_at) VALUES (?1, ?2, ?3)", params![name, section.to_string(), now()]);
    Json(serde_json::json!({ "blocks": blocks(&app) })).into_response()
}

async fn block_delete(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    if !me.admin() { return deny() }
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("DELETE FROM layout_blocks WHERE id = ?1", [id]);
    Json(serde_json::json!({ "blocks": blocks(&app) })).into_response()
}

/// Dopo la pubblicazione: la home si rifà da sola, una pagina singola da sola, testata e piè di pagina rifanno tutto il sito.
fn republish(app: &App, name: &str) -> R<()> {
    match name.strip_prefix("page:").and_then(|x| x.parse::<i64>().ok()) {
        Some(id) => site::republish_page(app, id),
        None if name == "home" => site::rebuild_home(app),
        None => site::rebuild_all(app).map(|_| ()),
    }
}

async fn builder_template(State(app): S, Extension(me): Me, Path(t): Path<String>, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let (name, ..) = which(&app, &q);
    Json(builder::template(&t, &name)).into_response()
}

// ---------- backup ----------

const BACKUP_KEYS: &[&str] = &["backup_on", "backup_hour", "backup_keep", "backup_local_keep", "s3_on", "s3_provider", "s3_endpoint", "s3_region", "s3_bucket", "s3_key", "s3_secret", "s3_prefix"];

async fn backup_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let mut st = app.settings();
    let tz = site::tz(&st);
    let has_secret = st.remove("s3_secret").is_some_and(|v| !v.is_empty());
    let log: Vec<serde_json::Value> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut h = db.prepare("SELECT name, size, ok, note, created_at FROM backups ORDER BY id DESC LIMIT 15").unwrap();
        let v = h.query_map([], |r| Ok(serde_json::json!({"name": r.get::<_, String>(0)?, "mb": format!("{:.1}", r.get::<_, i64>(1)? as f64 / 1_048_576.0), "ok": r.get::<_, bool>(2)?, "note": r.get::<_, String>(3)?, "date": site::human(r.get(4)?, &tz)}))).unwrap().filter_map(Result::ok).collect();
        v
    };
    let mut local: Vec<(String, String)> = fs::read_dir(backup::DIR).into_iter().flatten().flatten()
        .filter_map(|e| { let n = e.file_name().to_string_lossy().to_string(); (n.starts_with("presstatic-backup-") && n.ends_with(".tar.gz")).then(|| (n, format!("{:.1}", e.metadata().map(|m| m.len()).unwrap_or(0) as f64 / 1_048_576.0))) }).collect();
    local.sort(); local.reverse();
    let state = backup::STATE.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let sv = opt(&st, "_v_backup", "0").to_string();
    admin(&app, &me, "admin/backup.html", context! { s => st, sv, has_secret, log, local, state, msg => q.get("msg") })
}

async fn backup_save(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.admin() { return deny() }
    let r = store(&app.db.lock().unwrap_or_else(|e| e.into_inner()), BACKUP_KEYS, &f, Some("_v_backup"));
    back("/admin/backup", &r.map(|_| "Impostazioni dei backup salvate.".to_string()).unwrap_or_else(|e| format!("Errore: {e}.")))
}

async fn backup_now(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    let a = app.clone();
    std::thread::spawn(move || { let _ = backup::run(&a); });
    back("/admin/backup", "Backup avviato: l'avanzamento è qui sotto.")
}

async fn backup_state(State(_app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    Json(backup::STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()).into_response()
}

async fn backup_test_s3(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    let st = app.settings();
    let r = tokio::task::spawn_blocking(move || backup::S3::from(&st).and_then(|s3| s3.test())).await.unwrap();
    back("/admin/backup", &match r { Ok(()) => "Collegamento riuscito: Presstatic può scrivere e cancellare nel bucket.".to_string(), Err(e) => format!("Errore: {e}.") })
}

/// Scarica una copia locale (a flusso: anche un archivio di gigabyte non passa tutto dalla memoria).
/// Aspetto: i temi con l'anteprima, il colore principale, la modalità chiara, scura o automatica e il pulsante sole/luna.
async fn appearance_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let st = app.settings();
    let list: Vec<serde_json::Value> = themes().into_iter().map(|(id, label)| {
        let (name, desc) = label.split_once(':').map(|(a, b)| (a.trim().to_string(), b.trim().to_string())).unwrap_or((label.clone(), String::new()));
        let preview = if THEMES.iter().any(|t| t.0 == id) { format!("/admin/assets/{}/tema-{id}.webp", update::VERSION) } else { String::new() };
        serde_json::json!({"id": id, "name": name, "desc": desc, "preview": preview})
    }).collect();
    admin(&app, &me, "admin/aspetto.html", context! { themes => list, theme => opt(&st, "theme", "classico"), accent => opt(&st, "accent", ""), color_mode => opt(&st, "color_mode", "auto"),
        logo => opt(&st, "logo", ""), favicon => opt(&st, "favicon", ""), site_name => opt(&st, "site_name", ""),
        mode_toggle => opt(&st, "mode_toggle", "") == "on", base => site::base(&st), msg => q.get("msg") })
}

async fn appearance_save(State(app): S, Extension(me): Me, mut mp: Multipart) -> Response {
    if !me.admin() { return deny() }
    let (mut f, mut files): (HashMap<String, String>, Vec<(String, String, Vec<u8>)>) = (HashMap::new(), vec![]);
    while let Ok(Some(field)) = mp.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match field.file_name().map(str::to_string) {
            Some(file) => { let b = field.bytes().await.unwrap_or_default(); if !b.is_empty() { files.push((name, file, b.to_vec())) } }
            None => { f.insert(name, field.text().await.unwrap_or_default()); }
        }
    }
    let g = |k: &str| f.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
    let theme = g("theme");
    if !themes().iter().any(|t| t.0 == theme) { return back("/admin/aspetto", "Tema non valido.") }
    let accent = g("accent");
    if !(accent.is_empty() || (accent.len() == 7 && accent.starts_with('#') && accent[1..].chars().all(|c| c.is_ascii_hexdigit()))) { return back("/admin/aspetto", "Colore non valido: usa il formato #rrggbb.") }
    let mode = match g("color_mode").as_str() { "light" => "light", "dark" => "dark", _ => "auto" };
    let toggle = if g("mode_toggle") == "on" { "on" } else { "" };
    {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        for (k, v) in [("theme", theme.as_str()), ("accent", accent.as_str()), ("color_mode", mode), ("mode_toggle", toggle)] {
            let _ = db.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?1, ?2)", [k, v]);
        }
    }
    // Logo e icona del sito: un file nuovo sostituisce quello di prima; «togli» torna al nome scritto / all'iniziale.
    for (field, key) in [("logo_upload", "logo"), ("favicon_upload", "favicon")] {
        let url = match files.iter().find(|x| x.0 == field) {
            Some((_, file, bytes)) => match site::save_upload(&app, file, bytes) { Ok(u) => Some(u), Err(e) => return back("/admin/aspetto", &format!("{} non caricato: {e}.", if key == "logo" { "Logo" } else { "Icona" })) },
            None => (g(&format!("{key}_remove")) == "on").then(String::new),
        };
        if let Some(u) = url { let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?1, ?2)", [key, u.as_str()]); }
    }
    let a = app.clone();
    let m = tokio::task::spawn_blocking(move || site::rebuild_all(&a)).await.unwrap().unwrap_or_default();
    back("/admin/aspetto", &format!("Aspetto aggiornato. {m}"))
}

/// Ripristino di una copia che è già sul server.
async fn backup_restore(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.admin() { return deny() }
    let name = f.get("name").cloned().unwrap_or_default();
    if !(name.starts_with("presstatic-backup-") && name.ends_with(".tar.gz")) || name.contains('/') || name.contains("..") { return back("/admin/backup", "Copia non valida.") }
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || backup::restore_live(&a, &std::path::Path::new(backup::DIR).join(&name))).await.unwrap();
    back("/admin/backup", &r.unwrap_or_else(|e| format!("Ripristino non riuscito: {e}.")))
}

/// Ripristino di un archivio caricato dal computer: si scrive su disco a pezzi (niente archivi interi in memoria).
async fn backup_restore_upload(State(app): S, Extension(me): Me, mut mp: Multipart) -> Response {
    if !me.admin() { return deny() }
    let _ = fs::create_dir_all(backup::DIR);
    let path = std::path::Path::new(backup::DIR).join(format!("caricato-{}.tar.gz", now()));
    let mut got = false;
    while let Ok(Some(mut field)) = mp.next_field().await {
        if field.name() != Some("archivio") { continue }
        let Ok(mut out) = fs::File::create(&path) else { return back("/admin/backup", "Archivio non salvato sul server.") };
        while let Ok(Some(chunk)) = field.chunk().await { if std::io::Write::write_all(&mut out, &chunk).is_err() { let _ = fs::remove_file(&path); return back("/admin/backup", "Archivio non salvato sul server.") } }
        got = true;
    }
    if !got { return back("/admin/backup", "Scegli un archivio .tar.gz da ripristinare.") }
    let (a, p) = (app.clone(), path.clone());
    let r = tokio::task::spawn_blocking(move || backup::restore_live(&a, &p)).await.unwrap();
    let _ = fs::remove_file(&path);
    back("/admin/backup", &r.unwrap_or_else(|e| format!("Ripristino non riuscito: {e}.")))
}

async fn backup_download(State(_app): S, Extension(me): Me, Path(name): Path<String>) -> Response {
    if !me.admin() { return deny() }
    if !(name.starts_with("presstatic-backup-") && name.ends_with(".tar.gz") && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))) { return StatusCode::NOT_FOUND.into_response() }
    let Ok(f) = tokio::fs::File::open(std::path::Path::new(backup::DIR).join(&name)).await else { return StatusCode::NOT_FOUND.into_response() };
    let len = f.metadata().await.map(|m| m.len()).unwrap_or(0);
    ([(header::CONTENT_TYPE, "application/gzip".to_string()), (header::CONTENT_LENGTH, len.to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\""))],
        axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(f))).into_response()
}

// Backup automatico, una volta al giorno all'ora scelta.
async fn backup_loop(app: Arc<App>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let st = app.settings();
        if !backup::due(&st) { continue }
        let today = jiff::Timestamp::from_second(now()).unwrap_or(jiff::Timestamp::UNIX_EPOCH).to_zoned(site::tz(&st)).strftime("%Y-%m-%d").to_string();
        let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('backup_day', ?1)", [today]);
        let a = app.clone();
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || backup::run(&a)).await { eprintln!("Backup automatico: {e}"); }
    }
}

// ---------- notifiche push ----------

// Invia le notifiche messe in coda dalle pubblicazioni.
async fn push_loop(app: Arc<App>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let items: Vec<(String, String, String)> = std::mem::take(&mut *push::QUEUE.lock().unwrap_or_else(|e| e.into_inner()));
        for (title, body, url) in items {
            let a = app.clone();
            if let Ok(Err(e)) = tokio::task::spawn_blocking(move || push::broadcast(&a, &title, &body, &url)).await { eprintln!("Notifica push non inviata: {e}"); }
        }
    }
}

async fn push_subscribe(State(app): S, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let ip = client_ip(peer, &headers);
    match push::subscribe(&app, &body, &ip) { Ok(()) => StatusCode::NO_CONTENT.into_response(), Err(e) => (StatusCode::BAD_REQUEST, e).into_response() }
}

async fn push_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let st = app.settings();
    let tz = site::tz(&st);
    let (subs, sends) = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let subs: i64 = db.query_row("SELECT COUNT(*) FROM push_subs", [], |r| r.get(0)).unwrap_or(0);
        let mut h = db.prepare("SELECT title, body, sent, failed, created_at FROM push_sends ORDER BY id DESC LIMIT 10").unwrap();
        let sends: Vec<serde_json::Value> = h.query_map([], |r| Ok(serde_json::json!({"title": r.get::<_, String>(0)?, "body": r.get::<_, String>(1)?, "sent": r.get::<_, i64>(2)?, "failed": r.get::<_, i64>(3)?, "date": site::human(r.get(4)?, &tz)}))).unwrap().filter_map(Result::ok).collect();
        (subs, sends)
    };
    let sv = opt(&st, "_v_push", "0").to_string();
    admin(&app, &me, "admin/push.html", context! { s => st.clone(), sv, subs, sends, on => push::on(&st), https => site::base(&st).starts_with("https://"), msg => q.get("msg") })
}

async fn push_save(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.admin() { return deny() }
    if f.get("push_on").is_some() { if let Err(e) = push::ensure_keys(&app) { return back("/admin/push", &format!("Errore: {e}.")) } }
    if let Err(e) = store(&app.db.lock().unwrap_or_else(|e| e.into_inner()), &["push_on", "push_on_publish", "push_label"], &f, Some("_v_push")) { return back("/admin/push", &format!("Errore: {e}.")) }
    // Il pulsante sta negli articoli e il service worker alla radice del sito: si rifà tutto.
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || site::rebuild_all(&a)).await.unwrap();
    back("/admin/push", &match r { Ok(m) => format!("Notifiche salvate. {m}"), Err(e) => format!("Errore: {e}.") })
}

async fn push_send(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.admin() { return deny() }
    let g = |k: &str, max: usize| f.get(k).map(|x| x.trim().chars().take(max).collect::<String>()).unwrap_or_default();
    let (title, body, url) = (g("title", 80), g("body", 200), g("url", 500));
    if title.is_empty() || body.is_empty() { return back("/admin/push", "Errore: scrivi titolo e testo della notifica.") }
    let url = if url.starts_with("https://") || url.starts_with("http://") { url } else { format!("{}/", site::base(&app.settings())) };
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || push::broadcast(&a, &title, &body, &url)).await.unwrap();
    back("/admin/push", &r.unwrap_or_else(|e| format!("Errore: {e}.")))
}

// ---------- newsletter ----------

const NEWSLETTER_KEYS: &[&str] = &["newsletter_on", "newsletter_title", "newsletter_pitch", "newsletter_every", "newsletter_hour"];

fn nl_html(app: &App, code: StatusCode, title: &str, text: &str) -> Response { (code, Html(newsletter::page(&app.settings(), title, text))).into_response() }

async fn nl_subscribe(State(app): S, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Form(f): Form<HashMap<String, String>>) -> Response {
    let ip = client_ip(peer, &headers);
    let g = |k: &str| f.get(k).cloned().unwrap_or_default();
    let (a, email, consent, trap) = (app.clone(), g("email"), g("consent") == "on", g("website"));
    match tokio::task::spawn_blocking(move || newsletter::subscribe(&a, &email, consent, &trap, &ip)).await.unwrap() {
        Ok(()) => nl_html(&app, StatusCode::OK, "Controlla la tua email", "Ti abbiamo mandato un'email con un link: premilo per confermare l'iscrizione. Se non la trovi, guarda nello spam."),
        Err(e) => nl_html(&app, StatusCode::BAD_REQUEST, "Iscrizione non riuscita", &format!("{e}. Torna indietro e riprova.")),
    }
}

async fn nl_confirm(State(app): S, Path(token): Path<String>) -> Response {
    match newsletter::confirm(&app, &token) {
        Ok(()) => nl_html(&app, StatusCode::OK, "Iscrizione confermata", "Grazie! Da ora ricevi la newsletter. In ogni email trovi il link per annullare l'iscrizione con un clic."),
        Err(e) => nl_html(&app, StatusCode::NOT_FOUND, "Link non valido", &format!("{e}.")),
    }
}

async fn nl_unsub_page(State(app): S, Path(_token): Path<String>) -> Response {
    // Pagina con un pulsante: i programmi che aprono i link nelle email (antivirus, anteprime) non disiscrivono nessuno per sbaglio.
    let st = app.settings();
    let site = site::esc(opt(&st, "site_name", "Il sito"));
    Html(format!("<!doctype html><html lang=\"it\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"robots\" content=\"noindex\"><title>Annulla l'iscrizione | {site}</title><body style=\"font-family:system-ui,sans-serif;max-width:34rem;margin:4rem auto;padding:0 1.2rem;line-height:1.55\"><p style=\"font-weight:700;color:#67728c\">{site}</p><h1>Annullare l'iscrizione alla newsletter?</h1><form method=\"post\"><button style=\"font:inherit;font-weight:700;padding:.7rem 1.2rem;border:0;border-radius:6px;background:#c6283a;color:#fff;cursor:pointer\">Sì, annulla l'iscrizione</button></form></body></html>")).into_response()
}

async fn nl_unsub(State(app): S, Path(token): Path<String>) -> Response {
    match newsletter::unsubscribe(&app, &token) {
        Ok(()) => nl_html(&app, StatusCode::OK, "Iscrizione annullata", "Non riceverai più la newsletter. Se cambi idea, puoi iscriverti di nuovo dal sito."),
        Err(e) => nl_html(&app, StatusCode::NOT_FOUND, "Link non valido", &format!("{e}.")),
    }
}

async fn nl_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let st = app.settings();
    let tz = site::tz(&st);
    let (counts, subs, sends) = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut c = db.prepare("SELECT status, COUNT(*) FROM subscribers GROUP BY status").unwrap();
        let counts: HashMap<String, i64> = c.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect();
        let search = format!("%{}%", q.get("q").map(|x| x.trim()).unwrap_or("").replace(['%', '_'], ""));
        let mut s = db.prepare("SELECT id, email, status, created_at FROM subscribers WHERE email LIKE ?1 ORDER BY id DESC LIMIT 200").unwrap();
        let subs: Vec<serde_json::Value> = s.query_map([&search], |r| Ok(serde_json::json!({"id": r.get::<_, i64>(0)?, "email": r.get::<_, String>(1)?, "status": r.get::<_, String>(2)?, "date": site::human(r.get(3)?, &tz)}))).unwrap().filter_map(Result::ok).collect();
        let mut h = db.prepare("SELECT subject, posts, sent, failed, created_at FROM newsletter_sends ORDER BY id DESC LIMIT 10").unwrap();
        let sends: Vec<serde_json::Value> = h.query_map([], |r| Ok(serde_json::json!({"subject": r.get::<_, String>(0)?, "posts": r.get::<_, i64>(1)?, "sent": r.get::<_, i64>(2)?, "failed": r.get::<_, i64>(3)?, "date": site::human(r.get(4)?, &tz)}))).unwrap().filter_map(Result::ok).collect();
        (counts, subs, sends)
    };
    let sv = opt(&st, "_v_newsletter", "0").to_string();
    let sending = newsletter::SENDING.lock().unwrap_or_else(|e| e.into_inner()).clone();
    admin(&app, &me, "admin/newsletter.html", context! { s => st.clone(), sv, counts, subs, sends, sending, mail_on => mail::configured(&st), q => q.get("q"), msg => q.get("msg") })
}

async fn nl_save(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.admin() { return deny() }
    let before = { let st = app.settings(); (opt(&st, "newsletter_on", "").to_string(), opt(&st, "newsletter_title", "").to_string(), opt(&st, "newsletter_pitch", "").to_string()) };
    if let Err(e) = store(&app.db.lock().unwrap_or_else(|e| e.into_inner()), NEWSLETTER_KEYS, &f, Some("_v_newsletter")) { return back("/admin/newsletter", &format!("Errore: {e}.")) }
    let after = { let st = app.settings(); (opt(&st, "newsletter_on", "").to_string(), opt(&st, "newsletter_title", "").to_string(), opt(&st, "newsletter_pitch", "").to_string()) };
    // Il modulo di iscrizione sta negli articoli: se cambia, si rifanno le pagine.
    let note = if before != after { let a = app.clone(); tokio::task::spawn_blocking(move || site::rebuild_all(&a)).await.unwrap().map(|m| format!(" {m}")).unwrap_or_default() } else { String::new() };
    back("/admin/newsletter", &format!("Newsletter salvata.{note}"))
}

async fn nl_send(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    let a = app.clone();
    std::thread::spawn(move || { if let Err(e) = newsletter::send_digest(&a) { newsletter::SENDING.lock().unwrap_or_else(|e| e.into_inner()).last = format!("Errore: {e}."); } });
    back("/admin/newsletter", "Invio avviato: l'avanzamento è qui sotto.")
}

async fn nl_state(State(_app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    Json(newsletter::SENDING.lock().unwrap_or_else(|e| e.into_inner()).clone()).into_response()
}

async fn nl_remove(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.admin() { return deny() }
    let id: i64 = f.get("id").and_then(|x| x.parse().ok()).unwrap_or(0);
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("DELETE FROM subscribers WHERE id = ?1", [id]);
    back("/admin/newsletter", "Iscritto eliminato.")
}

/// Cella CSV sicura: tra virgolette (le virgolette interne raddoppiate) e, se comincia con = + - @ o un
/// carattere di controllo, preceduta da un apostrofo: così Excel e i fogli di calcolo non la eseguono come formula.
// ---------- moduli ----------

/// Dove tornare dopo l'invio: la pagina da cui arriva il modulo, se è del sito; altrimenti la home.
fn form_back(st: &Settings, headers: &HeaderMap) -> String {
    let base = site::base(st).to_string();
    let r = headers.get(header::REFERER).and_then(|v| v.to_str().ok()).unwrap_or("").split('#').next().unwrap_or("").to_string();
    if !base.is_empty() && (r == base || r.starts_with(&format!("{base}/"))) { r } else { format!("{base}/") }
}

async fn form_submit(State(app): S, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Path(id): Path<i64>, Form(f): Form<HashMap<String, String>>) -> Response {
    let ip = client_ip(peer, &headers);
    let back = form_back(&app.settings(), &headers);
    let a = app.clone();
    match tokio::task::spawn_blocking(move || forms::submit(&a, id, &f, &ip)).await.unwrap() {
        Ok(sent) => {
            let st = app.settings();
            if let (Some(m), true) = (sent, mail::configured(&st)) {
                if m.to.contains('@') {
                    let link = format!("{}/admin/moduli/{id}/messaggi", panel_url(&st));
                    tokio::task::spawn_blocking(move || {
                        let site = opt(&st, "site_name", "Presstatic").to_string();
                        let lines: Vec<String> = m.lines.iter().map(|(k, v)| format!("{k}: {v}")).collect();
                        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
                        let html = mail::layout(&site, &format!("Nuovo messaggio: {}", m.form), &refs, Some(("Apri i messaggi", &link)), "Rispondi a questa email per scrivere direttamente a chi ha compilato il modulo.");
                        let _ = mail::send_reply(&st, &m.to, m.reply_to.as_deref(), &format!("Nuovo messaggio da «{}»", m.form), &format!("{}\n\nTutti i messaggi: {link}", lines.join("\n")), &html);
                    });
                }
            }
            Redirect::to(&format!("{back}#modulo-{id}-inviato")).into_response()
        }
        Err(e) => {
            let e = site::esc(&e);
            let msg = e.chars().next().map(|c| c.to_uppercase().collect::<String>() + &e[c.len_utf8()..]).unwrap_or_default();
            (StatusCode::BAD_REQUEST, Html(format!("<!doctype html><html lang=\"it\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Messaggio non inviato</title>\
<style>body{{font:17px/1.6 system-ui,sans-serif;max-width:34rem;margin:14vh auto;padding:0 1.2rem;color:#16202f}}h1{{font-size:1.6rem}}a{{color:#1e6bff;font-weight:700}}</style>\
<h1>Messaggio non inviato</h1><p>{msg}.</p><p><a href=\"{}\">← Torna al modulo</a>: con il tasto Indietro del browser ritrovi quello che avevi scritto.</p>", site::esc(&back)))).into_response()
        }
    }
}

async fn forms_page(State(app): S, Extension(me): Me, Query(q): Query<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let st = app.settings();
    let list: Vec<serde_json::Value> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        forms::cleanup(&db, &st);
        forms::all(&db).iter().map(|f| {
            let (n, unread): (i64, i64) = db.query_row("SELECT COUNT(*), COALESCE(SUM(seen = 0), 0) FROM form_entries WHERE form_id = ?1", [f.id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap_or((0, 0));
            serde_json::json!({"id": f.id, "name": f.name, "fields": f.fields.len(), "entries": n, "unread": unread, "notify": f.notify, "consent": f.consent})
        }).collect()
    };
    admin(&app, &me, "admin/moduli.html", context! { forms => list, keep => opt(&st, "forms_keep_days", "180"), msg => q.get("msg") })
}

async fn forms_settings(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let days: i64 = f.get("keep").and_then(|v| v.trim().parse().ok()).unwrap_or(180).clamp(0, 3650);
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('forms_keep_days', ?1)", [days.to_string()]);
    back("/admin/moduli", if days == 0 { "I messaggi si conservano finché non li elimini." } else { "Conservazione dei messaggi aggiornata." })
}

async fn form_edit(State(app): S, Extension(me): Me, Path(id): Path<i64>, Query(q): Query<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let form = if id == 0 { None } else {
        match forms::get(&app.db.lock().unwrap_or_else(|e| e.into_inner()), id) { Some(f) => Some(f), None => return back("/admin/moduli", "Modulo non trovato.") }
    };
    let starter = serde_json::json!([{"id": "nome", "label": "Nome e cognome", "type": "text", "required": true, "options": ""},
        {"id": "email", "label": "Email", "type": "email", "required": true, "options": ""}, {"id": "messaggio", "label": "Messaggio", "type": "textarea", "required": true, "options": ""}]);
    let fields = form.as_ref().map(|f| serde_json::to_string(&f.fields).unwrap_or_default()).unwrap_or_else(|| starter.to_string()).replace("</", "<\\/");
    let types_json = serde_json::to_string(&forms::TYPES).unwrap_or_default().replace("</", "<\\/");
    let ctx = match &form {
        Some(f) => context! { name => f.name, notify => f.notify, success => f.success, button => f.button, consent => f.consent, consent_text => f.consent_text, privacy_url => f.privacy_url },
        None => context! { name => "Contatti", notify => me.email.clone(), success => "", button => "", consent => false, consent_text => "", privacy_url => "" },
    };
    admin(&app, &me, "admin/modulo.html", context! { id, fields, types_json, msg => q.get("msg"), ..ctx })
}

async fn form_save(State(app): S, Extension(me): Me, Path(id): Path<i64>, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let r = forms::save(&app.db.lock().unwrap_or_else(|e| e.into_inner()), id, &f);
    match r {
        Ok(nid) => {
            // Il modulo può stare in più pagine (widget o [modulo N]): si rigenera il sito.
            let a = app.clone();
            let m = tokio::task::spawn_blocking(move || site::rebuild_all(&a)).await.unwrap().unwrap_or_default();
            back(&format!("/admin/moduli/{nid}"), &format!("Modulo salvato. {m}"))
        }
        Err(e) => back(&format!("/admin/moduli/{id}"), &format!("Errore: {e}.")),
    }
}

async fn form_delete(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    if !me.editor() { return deny() }
    {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let _ = db.execute("DELETE FROM form_entries WHERE form_id = ?1", [id]);
        let _ = db.execute("DELETE FROM forms WHERE id = ?1", [id]);
    }
    let a = app.clone();
    let _ = tokio::task::spawn_blocking(move || site::rebuild_all(&a)).await;
    back("/admin/moduli", "Modulo eliminato, con i suoi messaggi.")
}

async fn form_entries_page(State(app): S, Extension(me): Me, Path(id): Path<i64>, Query(q): Query<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let st = app.settings();
    let tz = site::tz(&st);
    let (form, list) = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        forms::cleanup(&db, &st);
        let Some(form) = forms::get(&db, id) else { return back("/admin/moduli", "Modulo non trovato.") };
        let list = forms::entries(&db, id);
        let _ = db.execute("UPDATE form_entries SET seen = 1 WHERE form_id = ?1 AND seen = 0", [id]); // aperti: non più «nuovi»
        (form, list)
    };
    let entries: Vec<serde_json::Value> = list.iter().map(|(eid, at, seen, lines)| serde_json::json!({"id": eid, "date": site::human(*at, &tz), "seen": seen, "lines": lines})).collect();
    admin(&app, &me, "admin/modulo-messaggi.html", context! { id, name => form.name, notify => form.notify, entries, keep => opt(&st, "forms_keep_days", "180"), msg => q.get("msg") })
}

async fn form_entries_delete(State(app): S, Extension(me): Me, Path(id): Path<i64>, body: axum::body::Bytes) -> Response {
    if !me.editor() { return deny() }
    let ids: Vec<i64> = form_urlencoded::parse(&body).filter(|(k, _)| k == "ids").filter_map(|(_, v)| v.parse().ok()).collect();
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    for e in &ids { let _ = db.execute("DELETE FROM form_entries WHERE id = ?1 AND form_id = ?2", params![e, id]); }
    back(&format!("/admin/moduli/{id}/messaggi"), &format!("Eliminati: {}.", ids.len()))
}

async fn form_csv(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    if !me.editor() { return deny() }
    let tz = site::tz(&app.settings());
    let list = forms::entries(&app.db.lock().unwrap_or_else(|e| e.into_inner()), id);
    let mut cols: Vec<String> = vec![];
    for (_, _, _, lines) in &list { for (k, _) in lines { if !cols.contains(k) { cols.push(k.clone()) } } }
    let mut out = String::from("\u{feff}");
    out += &std::iter::once("Data".to_string()).chain(cols.iter().cloned()).map(|c| csv_cell(&c)).collect::<Vec<_>>().join(",");
    out += "\r\n";
    for (_, at, _, lines) in list.iter().rev() {
        let row: Vec<String> = std::iter::once(site::human(*at, &tz)).chain(cols.iter().map(|c| lines.iter().find(|(k, _)| k == c).map(|(_, v)| v.clone()).unwrap_or_default())).map(|v| csv_cell(&v)).collect();
        out += &row.join(","); out += "\r\n";
    }
    ([(header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"modulo-{id}.csv\""))], out).into_response()
}

fn csv_cell(v: &str) -> String {
    let v = v.replace(['\r', '\n'], " ");
    let v = if v.starts_with(['=', '+', '-', '@', '\t']) { format!("'{v}") } else { v };
    format!("\"{}\"", v.replace('"', "\"\""))
}

async fn nl_export(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    let st = app.settings();
    let tz = site::tz(&st);
    let mut csv = String::from("email,stato,iscritto il\n");
    let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(mut q) = db.prepare("SELECT email, status, created_at FROM subscribers ORDER BY id") {
        for (e, s, t) in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))).unwrap().filter_map(Result::ok) {
            csv += &format!("{},{},{}\n", csv_cell(&e), csv_cell(&s), csv_cell(&site::human(t, &tz)));
        }
    }
    ([(header::CONTENT_TYPE, "text/csv; charset=utf-8"), (header::CONTENT_DISPOSITION, "attachment; filename=\"iscritti-newsletter.csv\"")], csv).into_response()
}

// Invio automatico del riepilogo, all'ora scelta. Il giorno si segna prima di inviare: niente doppioni anche se l'invio è lungo.
async fn newsletter_loop(app: Arc<App>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let st = app.settings();
        if !newsletter::due(&st) { continue }
        let today = jiff::Timestamp::from_second(now()).unwrap_or(jiff::Timestamp::UNIX_EPOCH).to_zoned(site::tz(&st)).strftime("%Y-%m-%d").to_string();
        let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('newsletter_day', ?1)", [today]);
        let a = app.clone();
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || newsletter::send_digest(&a)).await { eprintln!("Newsletter non inviata: {e}"); }
    }
}

// ---------- categorie ----------

async fn categories_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.editor() { return deny() }
    let a = app.clone();
    let tree = tokio::task::spawn_blocking(move || site::category_tree(&a)).await.unwrap();
    admin(&app, &me, "admin/categorie.html", context! { tree, base => site::base(&app.settings()), msg => q.get("msg") })
}

async fn category_save(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let g = |k: &str| f.get(k).cloned().unwrap_or_default();
    let (a, name, parent, desc) = (app.clone(), g("name"), g("parent"), g("description"));
    let r = tokio::task::spawn_blocking(move || site::category_save(&a, &name, &parent, &desc)).await.unwrap();
    back("/admin/categorie", &r.unwrap_or_else(|e| format!("Errore: {e}.")))
}

async fn category_rename(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let g = |k: &str| f.get(k).cloned().unwrap_or_default();
    let (a, old, new) = (app.clone(), g("old"), g("new"));
    let r = tokio::task::spawn_blocking(move || site::category_rename(&a, &old, &new)).await.unwrap();
    back("/admin/categorie", &r.unwrap_or_else(|e| format!("Errore: {e}.")))
}

async fn category_delete(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.editor() { return deny() }
    let g = |k: &str| f.get(k).cloned().unwrap_or_default();
    let (a, name, to) = (app.clone(), g("name"), g("to"));
    let r = tokio::task::spawn_blocking(move || site::category_delete(&a, &name, &to)).await.unwrap();
    back("/admin/categorie", &r.unwrap_or_else(|e| format!("Errore: {e}.")))
}

/// Azione in blocco sugli articoli scelti nell'elenco.
async fn bulk(State(app): S, Extension(me): Me, body: axum::body::Bytes) -> Response {
    let pairs: Vec<(String, String)> = form_urlencoded::parse(&body).into_owned().collect();
    let ids: Vec<i64> = pairs.iter().filter(|(k, _)| k == "ids").filter_map(|(_, v)| v.parse().ok()).collect();
    let get = |k: &str| pairs.iter().find(|(x, _)| x == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let (action, value, back_to) = (get("action"), get("value"), get("back"));
    let back_to = if back_to.starts_with("/admin") && !back_to.starts_with("//") { back_to } else { "/admin".into() };
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || site::bulk(&a, &me, &ids, &action, &value)).await.unwrap();
    let sep = if back_to.contains('?') { "&" } else { "?" };
    Redirect::to(&format!("{back_to}{sep}msg={}", urlencoding(&r.unwrap_or_else(|e| format!("Errore: {e}."))))).into_response()
}

fn urlencoding(v: &str) -> String { form_urlencoded::byte_serialize(v.as_bytes()).collect() }

/// Nota della redazione: si aggiunge dall'editor; l'autore dell'articolo riceve un'email (se le email sono configurate).
async fn note_add(State(app): S, Extension(me): Me, headers: HeaderMap, Path(id): Path<i64>, Form(f): Form<HashMap<String, String>>) -> Response {
    let json = headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|v| v.contains("application/json"));
    let body = f.get("body").cloned().unwrap_or_default();
    let a = app.clone();
    let m = me.clone();
    let b2 = body.clone();
    match tokio::task::spawn_blocking(move || site::add_note(&a, &m, id, &b2)).await.unwrap() {
        Ok(p) => {
            let st = app.settings();
            let author: Option<(String, String)> = (p.author_id != me.id).then(|| app.db.lock().unwrap_or_else(|e| e.into_inner())
                .query_row("SELECT email, name FROM users WHERE id = ?1 AND role <> 'disabled'", [p.author_id], |r| Ok((r.get(0)?, r.get(1)?))).ok()).flatten();
            if let (Some((to, name)), true) = (author, mail::configured(&st)) {
                let link = format!("{}/admin/edit/{id}", panel_url(&st));
                let site = opt(&st, "site_name", "Presstatic").to_string();
                let html = mail::layout(&site, &format!("Una nota su «{}»", p.title), &[&format!("Ciao {name}, {} ha lasciato una nota sul tuo articolo:", me.name), &body], Some(("Apri l'articolo", &link)), "Le note restano nel pannello e non compaiono sul sito.");
                let text = format!("{} ha lasciato una nota su «{}»:\n\n{body}\n\n{link}", me.name, p.title);
                let subject = format!("Nota su «{}»", p.title);
                tokio::task::spawn_blocking(move || { let _ = mail::send(&st, &to, &subject, &text, &html); });
            }
            if json { Json(serde_json::json!({ "ok": true })).into_response() } else { back(&format!("/admin/edit/{id}"), "Nota aggiunta.") }
        }
        Err(e) if json => Json(serde_json::json!({ "error": format!("Nota non aggiunta: {e}.") })).into_response(),
        Err(e) => back(&format!("/admin/edit/{id}"), &format!("Errore: {e}.")),
    }
}

async fn note_delete(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    let post: Option<i64> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let row: Option<(i64, i64)> = db.query_row("SELECT post_id, user_id FROM post_notes WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).ok();
        match row {
            Some((pid, uid)) if uid == me.id || me.admin() => { let _ = db.execute("DELETE FROM post_notes WHERE id = ?1", [id]); Some(pid) }
            Some(_) => return deny(),
            None => None,
        }
    };
    back(&post.map(|p| format!("/admin/edit/{p}")).unwrap_or_else(|| "/admin".into()), "Nota eliminata.")
}

/// Segnale dell'editor aperto, ogni 30 secondi: tiene l'articolo a me, o dice chi è subentrato.
async fn lock_ping(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    if !site::get_post(&app, id).is_some_and(|p| me.can_edit(&p)) { return StatusCode::FORBIDDEN.into_response() }
    Json(serde_json::json!({ "taken_by": site::heartbeat(&app, id, me.id) })).into_response()
}

/// «Subentra»: l'articolo passa a me, e chi lo aveva aperto viene avvisato al suo prossimo segnale.
async fn lock_take(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    if !site::get_post(&app, id).is_some_and(|p| me.can_edit(&p)) { return StatusCode::FORBIDDEN.into_response() }
    site::take_lock(&app, id, me.id);
    Redirect::to(&format!("/admin/edit/{id}")).into_response()
}

async fn lock_release(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    site::release_lock(&app, id, me.id);
    StatusCode::NO_CONTENT.into_response()
}

async fn save(State(app): S, Extension(me): Me, Path(id): Path<i64>, mp: Multipart) -> Response {
    let (f, files) = read_form(mp).await;
    let upload = files.into_iter().find(|x| x.0 == "upload").map(|x| (x.1, x.2));
    let a = app.clone();
    match tokio::task::spawn_blocking(move || site::save_post(&a, &me, id, f, upload)).await.unwrap() {
        Ok((id, msg)) => back(&format!("/admin/edit/{id}"), &msg),
        Err(e) => back(&format!("/admin/edit/{id}"), &format!("Errore: {e}")),
    }
}

// Caricamento immagini dall'editor: risponde con l'indirizzo del file.
async fn upload(State(app): S, mp: Multipart) -> Response {
    let (_, files) = read_form(mp).await;
    let Some((_, name, bytes)) = files.into_iter().next() else {
        return Json(serde_json::json!({ "error": "Nessun file ricevuto." })).into_response();
    };
    let a = app.clone();
    match tokio::task::spawn_blocking(move || site::save_upload(&a, &name, &bytes)).await.unwrap() {
        Ok(url) => Json(serde_json::json!({ "url": url })),
        Err(e) => Json(serde_json::json!({ "error": format!("Immagine non caricata: {e}.") })),
    }.into_response()
}

async fn delete(State(app): S, Extension(me): Me, Path(id): Path<i64>, body: axum::body::Bytes) -> Response {
    // Il corpo si legge a mano: una richiesta di eliminazione senza campi (e senza Content-Type) resta valida, come prima.
    let f: HashMap<String, String> = form_urlencoded::parse(&body).into_owned().collect();
    // Dove porta l'indirizzo dell'articolo eliminato: home, un altro articolo o pagina, o «pagina non trovata».
    let to = match f.get("redirect").map(String::as_str) {
        Some("home") => Some(0),
        Some("post") => match f.get("target_id").and_then(|x| x.parse::<i64>().ok()).filter(|&t| t > 0) {
            Some(t) => Some(t),
            None => return back(&format!("/admin/edit/{id}"), "Errore: scegli l'articolo o la pagina verso cui portare i lettori."),
        },
        _ => None,
    };
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || site::delete_post(&a, &me, id, to)).await.unwrap();
    back("/admin", &r.unwrap_or_else(|e| format!("Errore: {e}")))
}

// L'anteprima gira in una sandbox: un eventuale script nel testo non può agire sul pannello.
const SANDBOX: (header::HeaderName, &str) = (header::CONTENT_SECURITY_POLICY, "sandbox allow-scripts allow-popups allow-popups-to-escape-sandbox");

async fn preview(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    let cx = site::Cx::new(app.settings());
    match site::get_post(&app, id).filter(|p| me.editor() || p.author_id == me.id).map(|p| site::render_post(&app, &cx, &p, site::related(&app, &cx, &p))) {
        Some(Ok(h)) => ([SANDBOX], Html(h)).into_response(),
        Some(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        None => deny(),
    }
}

async fn revision(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    let st = app.settings();
    let rev = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row(
        "SELECT r.post_id, r.title, r.description, r.body, r.created_at, COALESCE(u.name, '') FROM revisions r LEFT JOIN users u ON u.id = r.user_id WHERE r.id = ?1",
        [id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?, r.get::<_, String>(5)?))).optional().ok().flatten();
    let Some((post_id, title, description, body, at, user)) = rev else { return StatusCode::NOT_FOUND.into_response() };
    let Some(p) = site::get_post(&app, post_id).filter(|p| me.editor() || p.author_id == me.id) else { return deny() };
    admin(&app, &me, "admin/revision.html", context! {
        id, post_id, title, description, body, user, date => site::human(at, &site::tz(&st)), can_restore => me.can_edit(&p), post_title => p.title,
    })
}

async fn restore(State(app): S, Extension(me): Me, Path(id): Path<i64>) -> Response {
    let a = app.clone();
    match tokio::task::spawn_blocking(move || site::restore_revision(&a, &me, id)).await.unwrap() {
        Ok((post, msg)) => back(&format!("/admin/edit/{post}"), &msg),
        Err(e) => back("/admin", &format!("Errore: {e}")),
    }
}

// ---------- utenti ----------

async fn users(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let list: Vec<serde_json::Value> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut u = db.prepare("SELECT u.id, u.name, u.email, u.role, u.photo, (SELECT COUNT(*) FROM posts p WHERE p.author_id = u.id), u.totp_on FROM users u ORDER BY u.name").unwrap();
        let v = u.query_map([], |r| Ok(serde_json::json!({"id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "email": r.get::<_, String>(2)?,
            "role": r.get::<_, String>(3)?, "photo": r.get::<_, String>(4)?, "posts": r.get::<_, i64>(5)?, "twofa": r.get::<_, bool>(6)?}))).unwrap().filter_map(Result::ok).collect();
        v
    };
    let twofa_required = opt(&app.settings(), "twofa_required", "") == "on";
    admin(&app, &me, "admin/users.html", context! { users => list, roles => site::ROLES, twofa_required, msg => q.get("msg") })
}

async fn user_form(State(app): S, Extension(me): Me, Path(id): Path<i64>, Query(q): Msg) -> Response {
    if !(me.admin() || me.id == id) { return deny() }
    let user = if id == 0 { site::User { role: "author".into(), ..Default::default() } } else {
        match site::get_user(&app.db.lock().unwrap_or_else(|e| e.into_inner()), id) { Some(u) => u, None => return StatusCode::NOT_FOUND.into_response() }
    };
    let url = if user.slug.is_empty() { String::new() } else { site::author_url(&app.settings(), &user.slug) };
    let (uv, twofa): (i64, bool) = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT version, totp_on FROM users WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap_or((0, false));
    admin(&app, &me, "admin/user.html", context! { user, url, uv, twofa, roles => site::ROLES, msg => q.get("msg") })
}

async fn user_save(State(app): S, Extension(me): Me, headers: HeaderMap, Path(id): Path<i64>, mp: Multipart) -> Response {
    let (f, files) = read_form(mp).await;
    let new_password = f.get("password").is_some_and(|p| !p.trim().is_empty());
    // Cambiare la propria email (nome di accesso) o password richiede la password attuale: una sessione rubata
    // non basta a prendersi l'account.
    let new_email = f.get("email").map(|e| e.trim().to_lowercase()).is_some_and(|e| !e.is_empty() && Some(e) != site::get_user(&app.db.lock().unwrap_or_else(|e| e.into_inner()), id).map(|u| u.email));
    if (new_password || new_email) && me.id == id {
        let current = f.get("current_password").cloned().unwrap_or_default();
        let hash: String = app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT pass FROM users WHERE id = ?1", [id], |r| r.get(0)).unwrap_or_default();
        if !tokio::task::spawn_blocking(move || check_password(&current, &hash)).await.unwrap_or(false) {
            return back(&format!("/admin/users/{id}"), "Errore: la password attuale non è corretta.");
        }
    }
    let photo = files.into_iter().find(|x| x.0 == "photo").map(|x| x.2);
    let a = app.clone();
    match tokio::task::spawn_blocking(move || site::save_user(&a, &me, id, f, photo)).await.unwrap() {
        Ok((id, msg)) => {
            if new_password { // le altre sessioni aperte con la vecchia password si chiudono
                let current = cookie(&headers);
                app.sessions.lock().unwrap_or_else(|e| e.into_inner()).retain(|t, v| v.0 != id || Some(t) == current.as_ref());
            }
            back(&format!("/admin/users/{id}"), &msg)
        }
        Err(e) => back(&format!("/admin/users/{id}"), &format!("Errore: {e}")),
    }
}

async fn profile(Extension(me): Me) -> Response { Redirect::to(&format!("/admin/users/{}", me.id)).into_response() }

// ---------- impostazioni ----------

fn themes() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = THEMES.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    for e in fs::read_dir("themes").into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if e.path().is_dir() && !v.iter().any(|t| t.0 == name) && site::valid_theme(&name) {
            v.push((name.clone(), format!("Personalizzato: cartella themes/{name}")));
        }
    }
    v
}

async fn settings_form(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let mut st = app.settings();
    let has_token = st.remove("cf_token").is_some_and(|t| !t.is_empty()); // il token non torna mai nel browser
    for k in SECRETS { st.remove(*k); }
    let sv = opt(&st, "_v_settings", "0").to_string();
    admin(&app, &me, "admin/settings.html", context! { s => st, has_token, themes => themes(), sv, msg => q.get("msg") })
}

async fn settings_save(State(app): S, Extension(me): Me, mp: Multipart) -> Response {
    if !me.admin() { return deny() }
    let (mut f, files) = read_form(mp).await;
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || {
        let mut notes = String::new();
        for (field, name, bytes) in files {
            let key = field.trim_end_matches("_upload").to_string();
            // logo e icona del sito, e i loghi ufficiali dei social caricati dalla sezione Condivisione
            let key = key.strip_suffix("_file").filter(|k| k.starts_with("share_logo_")).map(String::from).unwrap_or(key);
            if key != "logo" && key != "favicon" && !key.starts_with("share_logo_") { continue }
            match site::save_upload(&a, &name, &bytes) {
                Ok(url) => { f.insert(key, url); }
                Err(e) => notes += &format!(" Immagine per {key} non caricata: {e}."),
            }
        }
        {
            let db = a.db.lock().unwrap_or_else(|e| e.into_inner());
            store(&db, SETTING_KEYS, &f, Some("_v_settings"))?;
            if let Some(m) = f.get("menu") { site::menu_categories(&db, m); } // sezioni nuove nel menu = categorie nuove
        }
        // Ogni modifica finisce subito sul sito: rigenera tutte le pagine e svuota la cache.
        site::rebuild_all(&a).map(|m| format!("Impostazioni salvate. {m}{notes}"))
    }).await.unwrap();
    back("/admin/settings", &r.unwrap_or_else(|e| format!("Errore: {e}")))
}

/// `version`: chiave del numero di versione di questo modulo (impostazioni o integrazioni). Se il modulo porta
/// il numero visto all'apertura (`sv`) e nel frattempo un altro amministratore ha salvato, non si scrive niente.
fn store(db: &Connection, keys: &[&str], f: &HashMap<String, String>, version: Option<&str>) -> R<()> {
    // Prima si controllano tutti i campi, poi si scrivono tutti in un'unica transazione: un campo sbagliato
    // non lascia più le impostazioni salvate a metà (e il sito disallineato, perché la rigenerazione non parte).
    let mut rows = vec![];
    for k in keys {
        let mut v = f.get(*k).map(|x| x.trim().to_string()).unwrap_or_default();
        if SECRETS.contains(k) && v.is_empty() { continue } // campo vuoto = mantieni il valore salvato
        if *k == "theme" && !themes().iter().any(|t| t.0 == v) { continue }
        if *k == "base_url" { v = v.trim_end_matches('/').to_string() }
        if *k == "cf_zone" && !v.is_empty() && !(v.len() == 32 && v.chars().all(|c| c.is_ascii_hexdigit())) {
            return Err("lo Zone ID di Cloudflare deve essere un codice di 32 caratteri (lettere a-f e numeri): lo trovi nella pagina Panoramica del dominio".into());
        }
        rows.push((*k, v));
    }
    db.execute_batch("BEGIN IMMEDIATE").map_err(s)?;
    if let Some(vk) = version {
        let current: i64 = db.query_row("SELECT value FROM settings WHERE key = ?1", [vk], |r| r.get::<_, String>(0)).ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        if f.get("sv").and_then(|x| x.trim().parse::<i64>().ok()).is_some_and(|seen| seen != current) {
            let _ = db.execute_batch("ROLLBACK");
            return Err("nel frattempo un altro amministratore ha salvato questa pagina: le tue modifiche non sono state salvate, per non sovrascrivere le sue. Ricarica la pagina e rifalle".into());
        }
        rows.push((vk, (current + 1).to_string()));
    }
    for (k, v) in &rows {
        if let Err(e) = db.execute("INSERT OR REPLACE INTO settings(key, value) VALUES (?1, ?2)", params![k, v]) {
            let _ = db.execute_batch("ROLLBACK");
            return Err(e.to_string());
        }
    }
    db.execute_batch("COMMIT").map_err(s)
}

// ---------- installazione guidata ----------

const SETUP_KEYS: &[&str] = &["site_name", "base_url", "description", "lang", "timezone", "theme", "accent", "menu", "cf_zone", "cf_token", "anthropic_key", "openai_key"];

async fn setup_form(State(app): S, headers: HeaderMap, Query(q): Msg) -> Response {
    if app.setup.lock().unwrap_or_else(|e| e.into_inner()).is_none() { return Redirect::to("/admin/login").into_response() }
    // Indirizzo del sito suggerito dal dominio con cui è aperto il pannello: admin.miosito.it -> https://www.miosito.it
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("").to_string();
    let https = headers.get("x-forwarded-proto").is_some_and(|v| v == "https");
    let domain = host.strip_prefix("admin.").map(|d| if d.starts_with("www.") { d.to_string() } else { format!("www.{d}") }).unwrap_or(host);
    let guess = format!("{}://{domain}", if https { "https" } else { "http" });
    render(&app, "admin/setup.html", context! { token => q.get("token"), guess, themes => themes(), msg => q.get("msg") })
}

async fn setup_save(State(app): S, headers: HeaderMap, mp: Multipart) -> Response {
    // Due invii contemporanei (doppio clic, due persone con il codice) non devono creare due amministratori.
    static SETUP_BUSY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let Ok(_one) = SETUP_BUSY.try_lock() else { return back("/admin/setup", "Installazione già in corso: attendi qualche secondo e ricarica la pagina.") };
    let expected = app.setup.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let Some(expected) = expected else { return Redirect::to("/admin/login").into_response() };
    let (f, files) = read_form(mp).await;
    if f.get("token").map(|t| t.trim()) != Some(expected.as_str()) {
        return back("/admin/setup", "Il codice di installazione non è corretto: lo trovi nel terminale del server o nel file setup-token.txt.");
    }
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || -> R<(i64, String)> {
        let v = |k: &str| f.get(k).map(|x| x.trim().to_string()).unwrap_or_default();
        let (name, email, pass) = (v("name"), v("email").to_lowercase(), v("password"));
        if v("site_name").is_empty() || !v("base_url").starts_with("http") { return Err("servono il nome e l'indirizzo del sito".into()) }
        if name.is_empty() || !email.contains('@') || pass.chars().count() < 12 { return Err("servono nome, email e una password di almeno 12 caratteri".into()) }
        let mut f = f.clone();
        if let Some((_, file, bytes)) = files.iter().find(|x| x.0 == "logo_upload") {
            if let Ok(url) = site::save_upload(&a, file, bytes) { f.insert("logo".into(), url); }
        }
        let id = {
            let db = a.db.lock().unwrap_or_else(|e| e.into_inner());
            store(&db, SETUP_KEYS, &f, None)?;
            site::menu_categories(&db, &v("menu")); // le sezioni scelte diventano anche categorie
            if let Some(l) = f.get("logo") { db.execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('logo', ?1)", [l]).map_err(s)?; }
            let slug = site::unique_user_slug(&db, &name, 0);
            db.execute("INSERT INTO users(name, email, pass, role, slug) VALUES (?1, ?2, ?3, 'admin', ?4)", params![name, email, hash_password(&pass), slug]).map_err(s)?;
            db.last_insert_rowid()
        };
        let built = site::rebuild_all(&a)?;
        let st = a.settings();
        let cf = if opt(&st, "cf_zone", "").is_empty() { String::new() } else {
            cloudflare::setup(&st).map(|m| format!(" {m}")).unwrap_or_else(|e| format!(" Cloudflare non configurato: {e}. Puoi riprovare dalle Impostazioni."))
        };
        let _ = built; Ok((id, format!("Installazione completata: il sito è online su {}. Ora scrivi il primo articolo, oppure fallo scrivere all'IA partendo dalle fonti.{cf}", site::base(&st))))
    }).await.unwrap();
    match r {
        Ok((id, msg)) => {
            *app.setup.lock().unwrap_or_else(|e| e.into_inner()) = None;
            let _ = fs::remove_file("setup-token.txt");
            // Accesso automatico: alla fine dell'installazione si è già dentro il pannello.
            let t = new_token();
            app.sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(t.clone(), (id, now() + 12 * 3600));
            let c = session_cookie(&t, &headers);
            let enc: String = msg.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
            ([(header::SET_COOKIE, c)], Redirect::to(&format!("/admin?msg={enc}"))).into_response()
        }
        Err(e) => back(&format!("/admin/setup?token={expected}"), &format!("Errore: {e}.")),
    }
}

// ---------- file manager e aggiornamenti (solo amministratori) ----------

fn enc(s: &str) -> String { s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'/' { (b as char).to_string() } else { format!("%{b:02X}") }).collect() }

async fn files_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let rel = q.get("path").cloned().unwrap_or_default();
    match files::list(&app, &rel) {
        Ok(l) => admin(&app, &me, "admin/files.html", context! { l, msg => q.get("msg") }),
        Err(e) => back("/admin/files", &format!("Errore: {e}")),
    }
}

async fn files_action(State(app): S, Extension(me): Me, mp: Multipart) -> Response {
    if !me.admin() { return deny() }
    let (f, files) = read_form(mp).await;
    let v = |k: &str| f.get(k).cloned().unwrap_or_default();
    let (action, path, name) = (v("action"), v("path"), v("name"));
    let (a, act, p) = (app.clone(), action.clone(), path.clone());
    let r = tokio::task::spawn_blocking(move || -> R<String> {
        // In coda con pubblicazioni e rigenerazione: un tema non viene letto mentre è estratto a metà,
        // e una cartella non sparisce mentre ci si stanno scrivendo pagine.
        let _gen = site::gen_lock();
        match act.as_str() {
            "upload" => {
                let ups: Vec<&(String, String, Vec<u8>)> = files.iter().filter(|x| x.0 == "files" && !x.1.is_empty()).collect();
                if ups.is_empty() { return Err("nessun file scelto".into()) }
                ups.iter().map(|x| files::upload(&a, &p, &x.1, &x.2)).collect::<R<Vec<String>>>().map(|m| m.join(" "))
            }
            "mkdir" => files::mkdir(&a, &p, &name),
            "delete" => files::delete(&a, &p),
            "extract" => files::extract(&a, &p),
            _ => Err("azione sconosciuta".into()),
        }
    }).await.unwrap();
    // Dopo elimina ed estrai si torna alla cartella che contiene il file.
    let dir = if matches!(action.as_str(), "delete" | "extract") { path.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or(path) } else { path };
    let msg = match r { Ok(m) => m, Err(e) => format!("Errore: {e}") };
    Redirect::to(&format!("/admin/files?path={}&msg={}", enc(&dir), enc(&msg))).into_response()
}

async fn files_download(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    match files::download(&app, q.get("path").map(String::as_str).unwrap_or("")) {
        Ok((name, bytes)) => {
            let disposition = format!("attachment; filename=\"{}\"", name.replace(['"', '\\'], "_"));
            ([(header::CONTENT_TYPE, files::content_type(&name).to_string()), (header::CONTENT_DISPOSITION, disposition), (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into())], bytes).into_response()
        }
        Err(e) => back("/admin/files", &format!("Errore: {e}")),
    }
}

async fn update_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let rel = app.update.lock().unwrap_or_else(|e| e.into_inner()).clone();
    admin(&app, &me, "admin/update.html", context! { rel, version => update::VERSION, repo => update::REPO.trim(), unavailable => update::unavailable(), fingerprint => update::fingerprint(), msg => q.get("msg") })
}

async fn update_check(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    match tokio::task::spawn_blocking(update::check).await.unwrap() {
        Ok(Some(r)) => { let v = r.version.clone(); *app.update.lock().unwrap_or_else(|e| e.into_inner()) = Some(r); back("/admin/update", &format!("Disponibile la versione {v}.")) }
        Ok(None) => { *app.update.lock().unwrap_or_else(|e| e.into_inner()) = None; back("/admin/update", &format!("Presstatic {} è aggiornato: non ci sono versioni più recenti.", update::VERSION)) }
        Err(e) => back("/admin/update", &format!("Errore: {e}")),
    }
}

async fn update_apply(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    let Some(rel) = app.update.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return back("/admin/update", "Nessun aggiornamento da installare: premi prima «Controlla ora».") };
    match tokio::task::spawn_blocking(move || update::apply(&rel)).await.unwrap() {
        Ok(msg) => {
            *app.update.lock().unwrap_or_else(|e| e.into_inner()) = None;
            // Il processo termina dopo aver risposto: systemd (Restart=always) lo riavvia con la versione nuova.
            // Prima di uscire aspetta che finisca l'eventuale pubblicazione in corso, così nessuna pagina resta a metà.
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                let _ = tokio::task::spawn_blocking(|| { let _idle = site::wait_generation(); std::process::exit(0) }).await;
            });
            admin(&app, &me, "admin/update.html", context! { restarting => true, msg, version => update::VERSION })
        }
        Err(e) => back("/admin/update", &format!("Errore: {e}")),
    }
}

// ---------- integrazioni: Google e intelligenza artificiale ----------

/// Pagina «Indicizzazione Google»: chiave del service account, invii automatici, registro degli ultimi invii.
const GOOGLE_KEYS: &[&str] = &["google_sa", "google_on", "google_updates", "indexnow_on"];

async fn google_page(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let mut st = app.settings();
    let sa_email = serde_json::from_str::<serde_json::Value>(opt(&st, "google_sa", "")).ok().and_then(|v| v["client_email"].as_str().map(String::from)).unwrap_or_default();
    let has: HashMap<&str, bool> = SECRETS.iter().map(|k| (*k, st.remove(*k).is_some_and(|v| !v.is_empty()))).collect();
    let tz = site::tz(&st);
    let log: Vec<serde_json::Value> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut q = db.prepare("SELECT url, kind, ok, note, at FROM index_log ORDER BY id DESC LIMIT 20").unwrap();
        let v = q.query_map([], |r| Ok(serde_json::json!({"url": r.get::<_, String>(0)?, "kind": r.get::<_, String>(1)?, "ok": r.get::<_, bool>(2)?, "note": r.get::<_, String>(3)?, "at": site::human(r.get(4)?, &tz)}))).unwrap().filter_map(Result::ok).collect();
        v
    };
    let sv = opt(&st, "_v_google", "0").to_string();
    admin(&app, &me, "admin/google.html", context! { s => st.clone(), has, sa_email, log, sv, base => site::base(&st).to_string(), msg => q.get("msg") })
}

/// Prova IndexNow: invia l'indirizzo della home.
async fn indexnow_test(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    let st = app.settings();
    if !indexnow::on(&st) { return back("/admin/indicizzazione", "Errore: attiva IndexNow e salva, poi riprova.") }
    let home = format!("{}/", site::base(&st));
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || indexnow::submit(&a, &[home])).await.unwrap();
    back("/admin/indicizzazione", if r.contains("non avvisato") { r.trim().replace("IndexNow non avvisato:", "Errore: IndexNow non avvisato:") } else { format!("Prova riuscita:{r}") }.as_str())
}

async fn google_save(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.admin() { return deny() }
    if f.get("indexnow_on").is_some() { indexnow::ensure_key(&app); }
    let r = store(&app.db.lock().unwrap_or_else(|e| e.into_inner()), GOOGLE_KEYS, &f, Some("_v_google"));
    indexnow::write_key_file(&app); // il file della chiave compare (o sparisce) subito sul sito
    back("/admin/indicizzazione", &r.map(|_| "Indicizzazione rapida salvata.".to_string()).unwrap_or_else(|e| format!("Errore: {e}.")))
}

async fn integrations_form(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !me.admin() { return deny() }
    let mut st = app.settings();
    let sa_email = serde_json::from_str::<serde_json::Value>(opt(&st, "google_sa", "")).ok().and_then(|v| v["client_email"].as_str().map(String::from)).unwrap_or_default();
    let has: HashMap<&str, bool> = SECRETS.iter().map(|k| (*k, st.remove(*k).is_some_and(|v| !v.is_empty()))).collect();
    let tz = site::tz(&st);
    let log: Vec<serde_json::Value> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut q = db.prepare("SELECT url, kind, ok, note, at FROM index_log ORDER BY id DESC LIMIT 20").unwrap();
        let v = q.query_map([], |r| Ok(serde_json::json!({"url": r.get::<_, String>(0)?, "kind": r.get::<_, String>(1)?, "ok": r.get::<_, bool>(2)?, "note": r.get::<_, String>(3)?, "at": site::human(r.get(4)?, &tz)}))).unwrap().filter_map(Result::ok).collect();
        v
    };
    let sv = opt(&st, "_v_integrations", "0").to_string();
    admin(&app, &me, "admin/integrations.html", context! { s => st, has, sa_email, log, sv, models => ai::TEXT_MODELS, image_models => ai::IMAGE_MODELS, base => site::base(&app.settings()), msg => q.get("msg") })
}

async fn integrations_save(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !me.admin() { return deny() }
    if let Some(sa) = f.get("google_sa").filter(|v| !v.trim().is_empty()) {
        if serde_json::from_str::<serde_json::Value>(sa).ok().and_then(|v| v["private_key"].as_str().map(|_| ())).is_none() {
            return back("/admin/integrations", "Errore: la chiave di Google non è un file JSON di service account valido.");
        }
    }
    *app.google_token.lock().unwrap_or_else(|e| e.into_inner()) = None; // una chiave nuova richiede un token nuovo
    let r = store(&app.db.lock().unwrap_or_else(|e| e.into_inner()), INTEGRATION_KEYS, &f, Some("_v_integrations"));
    back("/admin/integrations", &r.map(|_| "Integrazioni salvate.".to_string()).unwrap_or_else(|e| format!("Errore: {e}")))
}

async fn google_test(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || google::test(&a)).await.unwrap();
    back("/admin/indicizzazione", &r.unwrap_or_else(|e| format!("Errore: {e}")))
}

fn ai_page(app: &App, me: &User, f: &HashMap<String, String>, msg: Option<String>) -> Response {
    let st = app.settings();
    let models = ai::models(&st);
    let model = f.get("model").cloned().unwrap_or_else(|| opt(&st, "ai_text_model", "").to_string());
    admin(app, me, "admin/ai.html", context! { models, model, f, can_image => !opt(&st, "openai_key", "").is_empty(), msg })
}

/// Per impostazione predefinita l'IA è per redattori e amministratori; l'amministratore può aprirla anche agli autori.
fn ai_allowed(app: &App, me: &User) -> bool { me.editor() || opt(&app.settings(), "ai_roles", "editor") == "all" }

async fn ai_form(State(app): S, Extension(me): Me, Query(q): Msg) -> Response {
    if !ai_allowed(&app, &me) { return deny() }
    ai_page(&app, &me, &HashMap::new(), q.get("msg").cloned())
}

async fn ai_write(State(app): S, Extension(me): Me, Form(f): Form<HashMap<String, String>>) -> Response {
    if !ai_allowed(&app, &me) { return deny() }
    // Al massimo 30 bozze al giorno per utente e 2 in contemporanea in tutto il sito: le chiamate costano.
    {
        let t = now();
        let mut uses = app.ai_uses.lock().unwrap_or_else(|e| e.into_inner());
        let mine = uses.entry(me.id).or_default();
        mine.retain(|x| t - x < 86_400);
        if mine.len() >= 30 { drop(uses); return ai_page(&app, &me, &f, Some("Hai raggiunto il limite di 30 bozze con l'IA nelle ultime 24 ore.".into())) }
        mine.push(t);
    }
    let Ok(_turn) = app.ai_gate.try_acquire() else {
        return ai_page(&app, &me, &f, Some("L'IA sta già scrivendo altre bozze: riprova tra un minuto.".into()));
    };
    let (a, m, form) = (app.clone(), me.clone(), f.clone());
    match tokio::task::spawn_blocking(move || ai::write(&a, &m, &form)).await.unwrap() {
        Ok((id, msg)) => back(&format!("/admin/edit/{id}"), &msg),
        Err(e) => ai_page(&app, &me, &f, Some(format!("Errore: {e}."))),
    }
}

async fn rebuild(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || site::rebuild_all(&a)).await.unwrap();
    back("/admin/settings", &r.unwrap_or_else(|e| format!("Errore: {e}")))
}

async fn cf_setup(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    let st = app.settings();
    let r = tokio::task::spawn_blocking(move || cloudflare::setup(&st)).await.unwrap();
    back("/admin/settings", &r.unwrap_or_else(|e| format!("Cloudflare non configurato: {e}")))
}

/// Ricorda fin dove sono stati controllati gli articoli programmati (serve dopo un riavvio).
fn mark_scheduled(app: &App, t: i64) {
    let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('sched_last', ?1)", [t.to_string()]);
}

// Ogni 30 secondi: pubblica gli articoli programmati e, se il sito è cambiato, aggiorna l'indice di ricerca.
async fn scheduler(app: Arc<App>) {
    let (mut last, mut last_update) = (now(), now() - 12 * 3600 + 20);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let (t, a) = (now(), app.clone());
        if t - last_update >= 12 * 3600 && update::unavailable().is_none() {
            last_update = t;
            let a = app.clone();
            if let Ok(Ok(Some(r))) = tokio::task::spawn_blocking(update::check).await { *a.update.lock().unwrap_or_else(|e| e.into_inner()) = Some(r) }
        }
        // La finestra avanza solo se la pubblicazione è riuscita: se fallisce (disco pieno, errore di un modello)
        // gli stessi articoli si riprovano al giro successivo, invece di restare senza pagina.
        match tokio::task::spawn_blocking(move || site::publish_due(&a, last, t)).await {
            Ok(Ok(())) => { last = t; mark_scheduled(&app, t); }
            Ok(Err(e)) => eprintln!("Pubblicazione programmata non riuscita, riprovo tra 30 secondi: {e}"),
            Err(e) => eprintln!("Pubblicazione programmata non riuscita, riprovo tra 30 secondi: {e}"),
        }
    }
}

// Più letti da Google Analytics, ogni 30 minuti (solo se attivi): non tocca né rallenta chi legge il sito.
async fn most_read_loop(app: Arc<App>) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        let st = app.settings();
        let due = now() - opt(&st, "most_read_at", "0").parse::<i64>().unwrap_or(0) >= 1800;
        if !(st.get("most_read_on").is_some_and(|v| v == "on") && due) { continue }
        let a = app.clone();
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || site::update_most_read(&a)).await {
            eprintln!("Più letti non aggiornati, riprovo tra 30 minuti: {e}");
            let _ = app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("INSERT OR REPLACE INTO settings(key, value) VALUES ('most_read_at', ?1)", [now().to_string()]);
        }
    }
}

async fn most_read_now(State(app): S, Extension(me): Me) -> Response {
    if !me.admin() { return deny() }
    let a = app.clone();
    let r = tokio::task::spawn_blocking(move || site::update_most_read(&a)).await.unwrap();
    back("/admin/integrations", &r.unwrap_or_else(|e| format!("Errore: {e}.")))
}

// Indice di ricerca, in un ciclo separato: su archivi grandi richiede qualche decina di secondi e non deve
// ritardare gli articoli programmati. Al massimo ogni 2 minuti; se non riesce, si riprova al giro dopo.
async fn search_indexer(app: Arc<App>) {
    let mut wait = 30; // il primo indice poco dopo l'avvio, poi ogni 2 minuti
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        wait = 120;
        if !app.search_dirty.swap(false, Ordering::Relaxed) { continue }
        let a = app.clone();
        let failed = match tokio::task::spawn_blocking(move || index_search(&a)).await {
            Ok(Ok(_)) => None,
            Ok(Err(e)) => Some(e),
            Err(e) => Some(e.to_string()),
        };
        if let Some(e) = failed {
            eprintln!("Indice di ricerca non aggiornato, riprovo tra 2 minuti: {e}");
            app.search_dirty.store(true, Ordering::Relaxed);
        }
    }
}

/// Scambia due cartelle in un'unica operazione del sistema operativo (renameat2 con RENAME_EXCHANGE).
fn swap_dirs(a: &std::path::Path, b: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &std::path::Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e));
    let (ca, cb) = (c(a)?, c(b)?);
    // SAFETY: due percorsi validi terminati da zero; la chiamata non trattiene i puntatori.
    let r = unsafe { libc::renameat2(libc::AT_FDCWD, ca.as_ptr(), libc::AT_FDCWD, cb.as_ptr(), libc::RENAME_EXCHANGE) };
    if r == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

/// Ricerca con Pagefind, incluso nel programma come libreria: niente da installare.
/// L'indice si scrive in una cartella nuova che poi prende il posto della vecchia, così non restano file superati.
fn index_search(app: &App) -> R<usize> {
    if !app.public.join("index.html").exists() { return Ok(0) } // sito non ancora generato
    let public = app.public.canonicalize().map_err(s)?;
    let (dir, tmp, old) = (public.join("pagefind"), public.join("pagefind.new"), public.join("pagefind.old"));
    let _ = fs::remove_dir_all(&tmp);
    // Gira in un thread di spawn_blocking: usa lo stesso runtime del server, come richiede Pagefind.
    let pages = tokio::runtime::Handle::current().block_on(async {
        let mut index = pagefind::api::PagefindIndex::new(None).map_err(s)?;
        let pages = index.add_directory(public.to_string_lossy().into(), None).await.map_err(s)?;
        index.write_files(Some(tmp.to_string_lossy().into())).await.map_err(s)?;
        Ok::<_, String>(pages)
    })?;
    // L'indice nuovo prende il posto del vecchio in un colpo solo (scambio atomico delle due cartelle): nemmeno per
    // un istante /pagefind/ manca. Se il file system non lo permette, si torna ai due spostamenti di prima.
    if dir.exists() && swap_dirs(&tmp, &dir).is_ok() {
        let _ = fs::remove_dir_all(&tmp); // dopo lo scambio qui c'è l'indice vecchio
    } else {
        let _ = fs::remove_dir_all(&old);
        let _ = fs::rename(&dir, &old);
        fs::rename(&tmp, &dir).map_err(s)?;
        let _ = fs::remove_dir_all(&old);
    }
    let st = app.settings();
    cloudflare::purge(&st, &[format!("{}/pagefind/pagefind-entry.json", site::base(&st))]);
    Ok(pages)
}

// Editor visuale (Quill 2, licenza BSD) incluso nel binario: il pannello non dipende da CDN esterne.
/// I file del pannello con la versione nel percorso (/admin/assets/1.0.4/admin.css): a ogni versione cambia l'indirizzo,
/// quindi browser e Cloudflare li riscaricano anche con «Ignore query string», e fino ad allora li possono tenere un anno.
async fn editor_asset_v(Path((_ver, file)): Path<(String, String)>) -> Response {
    let mut r = editor_asset(Path(file)).await;
    if r.status().is_success() { r.headers_mut().insert(header::CACHE_CONTROL, header::HeaderValue::from_static("public, max-age=31536000, immutable")); }
    r
}

async fn editor_asset(Path(file): Path<String>) -> Response {
    let (ct, body): (&str, &[u8]) = match file.as_str() {
        "quill.js" => ("text/javascript", include_bytes!("../assets/editor/quill.js")),
        "pb-editor.js" => ("text/javascript", include_bytes!("../assets/admin/pb-editor.js")),
        "tema-classico.webp" => ("image/webp", include_bytes!("../assets/admin/tema-classico.webp")),
        "tema-moderno.webp" => ("image/webp", include_bytes!("../assets/admin/tema-moderno.webp")),
        "pb-frame.js" => ("text/javascript", include_bytes!("../assets/admin/pb-frame.js")),
        "pb-frame.css" => ("text/css", include_bytes!("../assets/admin/pb-frame.css")),
        "quill.snow.css" => ("text/css", include_bytes!("../assets/editor/quill.snow.css")),
        // Stile, caratteri e logo del pannello: tutto dentro il programma, nessuna richiesta a servizi esterni.
        "admin.css" => ("text/css", include_bytes!("../assets/admin/admin.css")),
        "jakarta.woff2" => ("font/woff2", include_bytes!("../assets/admin/jakarta.woff2")),
        "newsreader.woff2" => ("font/woff2", include_bytes!("../assets/fonts/newsreader-5.3.woff2")),
        "logo.webp" => ("image/webp", include_bytes!("../assets/admin/logo.webp")),
        "icona.webp" => ("image/webp", include_bytes!("../assets/admin/icona.webp")),
        "favicon.png" => ("image/png", include_bytes!("../assets/admin/favicon.png")),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    ([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "public, max-age=604800")], body).into_response()
}

// Serve la cartella public solo per l'anteprima in locale: in produzione lo fa Nginx o Apache.
async fn static_file(State(app): S, uri: Uri) -> Response {
    let p = uri.path();
    // La radice dell'indirizzo del pannello (admin.dominio/) porta all'accesso, non a una copia della home del sito.
    if p == "/" { return Redirect::to("/admin").into_response() }
    let mut path = app.public.join(p.trim_start_matches('/'));
    if p.ends_with('/') { path.push("index.html") }
    let ct = match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8", "xml" => "application/xml", "txt" => "text/plain; charset=utf-8",
        "jpg" | "jpeg" => "image/jpeg", "png" => "image/png", "webp" => "image/webp", "gif" => "image/gif",
        "avif" => "image/avif", "woff2" => "font/woff2", "js" => "text/javascript", "css" => "text/css",
        "json" => "application/json", _ => "application/octet-stream",
    };
    // Le pagine del sito contengono script di terzi (pubblicità, statistiche, contenuti incorporati): sul dominio
    // del pannello si aprono in sandbox, così quegli script non possono leggere né usare il pannello.
    // La sandbox copre tutto ciò che il browser potrebbe eseguire (HTML, XML/XHTML, SVG, ecc.); solo immagini,
    // font e file scaricabili — che non eseguono codice — ne restano fuori, per poterli mostrare al pannello.
    let nosniff = (header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    let inert = matches!(ct, c if c.starts_with("image/") && !c.contains("svg")) || ct.starts_with("font/");
    // Solo file regolari sotto i 50 MB: un collegamento a /dev/zero o simili non viene mai letto.
    let readable = !p.contains("..") && fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= 50 * 1024 * 1024);
    match readable.then(|| fs::read(&path).ok()).flatten() {
        Some(b) if inert => ([(header::CONTENT_TYPE, ct), (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"), nosniff], b).into_response(),
        // Pagine e script si ricontrollano a ogni visita: dopo un aggiornamento nessuna copia vecchia resta nel browser.
        Some(b) => ([(header::CONTENT_TYPE, ct), SANDBOX, nosniff, (header::CACHE_CONTROL, "no-cache")], b).into_response(),
        None => {
            let page = fs::read(app.public.join("404.html")).unwrap_or_else(|_| b"Pagina non trovata".to_vec());
            (StatusCode::NOT_FOUND, [(header::CONTENT_TYPE, "text/html; charset=utf-8"), SANDBOX, nosniff, (header::CACHE_CONTROL, "no-cache")], page).into_response()
        }
    }
}
