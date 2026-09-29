//! Importazione da WordPress: il file di esportazione (Strumenti > Esporta > Tutti i contenuti, formato WXR).
//! Porta articoli, pagine, autori, categorie, tag, immagini (scaricate dal vecchio sito e rifatte in WebP) e crea
//! i reindirizzamenti dai vecchi indirizzi (per esempio /2024/05/12/titolo/) a quelli nuovi.
//! Gira in sottofondo: il pannello mostra l'avanzamento. Gli articoli restano nascosti finché l'importazione non
//! finisce, poi il sito si rigenera una volta sola.
use crate::{now, site, App, R};
use quick_xml::{events::Event, Reader};
use regex::Regex;
use rusqlite::params;
use serde::Serialize;
use std::{collections::HashMap, net::{SocketAddr, ToSocketAddrs}, sync::{LazyLock, Mutex}, time::Duration};

#[derive(Default, Clone, Serialize)]
pub struct State {
    pub running: bool,
    pub phase: String,
    pub total: usize,
    pub done: usize,
    pub posts: usize,
    pub pages: usize,
    pub users: usize,
    pub images: usize,
    pub redirects: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub finished: bool,
}
pub static STATE: LazyLock<Mutex<State>> = LazyLock::new(Default::default);
fn st<F: FnOnce(&mut State)>(f: F) { f(&mut STATE.lock().unwrap_or_else(|e| e.into_inner())) }
fn warn(msg: String) { st(|s| if s.errors.len() < 200 { s.errors.push(msg) }) }

#[derive(Default, Debug)]
struct Item {
    id: i64, typ: String, status: String, title: String, link: String, creator: String, content: String, excerpt: String,
    slug: String, date_gmt: String, cats: Vec<(String, String)>, meta: HashMap<String, String>, attachment_url: String,
}
#[derive(Default)]
struct Wxr { base: String, authors: HashMap<String, (String, String)>, terms: HashMap<String, String>, tree: Vec<(String, String, String)>, items: Vec<Item> }

pub struct Options { pub images: bool, pub authors: bool, pub owner: i64 }

/// Legge il file WXR (XML) senza caricarlo in un albero: va bene anche per archivi di centinaia di megabyte.
fn parse(xml: &str) -> R<Wxr> {
    let mut r = Reader::from_str(xml);
    r.config_mut().trim_text(false);
    let (mut w, mut stack, mut buf) = (Wxr::default(), Vec::<String>::new(), String::new());
    let (mut item, mut author, mut term, mut domain) = (None::<Item>, None::<HashMap<String, String>>, None::<HashMap<String, String>>, String::new());
    let mut meta_key = String::new();
    loop {
        match r.read_event().map_err(|e| format!("il file non è un'esportazione di WordPress valida ({e})"))? {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "item" => item = Some(Item::default()),
                    "wp:author" => author = Some(HashMap::new()),
                    "wp:category" if item.is_none() => term = Some(HashMap::new()),
                    "category" => domain = e.try_get_attribute("domain").ok().flatten().and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok()).map(|v| v.to_string()).unwrap_or_default(),
                    _ => {}
                }
                stack.push(name);
                buf.clear();
            }
            // Dalla versione 0.38 di quick-xml le entità (&amp;, &#233;…) arrivano come eventi a parte, non dentro il testo.
            Event::Text(t) => buf.push_str(&t.decode().map(|c| c.to_string()).unwrap_or_default()),
            Event::GeneralRef(r) => {
                let name = r.decode().map(|c| c.to_string()).unwrap_or_default();
                match r.resolve_char_ref() {
                    Ok(Some(ch)) => buf.push(ch),
                    _ => match quick_xml::escape::resolve_predefined_entity(&name) {
                        Some(v) => buf.push_str(v),
                        None => { buf.push('&'); buf.push_str(&name); buf.push(';'); } // entità sconosciuta: resta com'era, non sparisce
                    },
                }
            }
            Event::CData(c) => buf.push_str(&String::from_utf8_lossy(&c.into_inner())),
            Event::End(_) => {
                let name = stack.pop().unwrap_or_default();
                let v = std::mem::take(&mut buf);
                if let Some(it) = item.as_mut() {
                    match name.as_str() {
                        "item" => { w.items.push(item.take().unwrap()); }
                        "title" => it.title = v,
                        "link" => it.link = v.trim().to_string(),
                        "dc:creator" => it.creator = v,
                        "content:encoded" => it.content = v,
                        "excerpt:encoded" => it.excerpt = v,
                        "wp:post_id" => it.id = v.trim().parse().unwrap_or(0),
                        "wp:post_date_gmt" => it.date_gmt = v,
                        "wp:post_name" => it.slug = v.trim().to_string(),
                        "wp:status" => it.status = v,
                        "wp:post_type" => it.typ = v,
                        "wp:attachment_url" => it.attachment_url = v.trim().to_string(),
                        "category" => it.cats.push((std::mem::take(&mut domain), v)),
                        "wp:meta_key" => meta_key = v,
                        "wp:meta_value" => { it.meta.insert(std::mem::take(&mut meta_key), v); }
                        _ => {}
                    }
                } else if let Some(a) = author.as_mut() {
                    if name == "wp:author" { let a = author.take().unwrap(); w.authors.insert(a.get("wp:author_login").cloned().unwrap_or_default(), (a.get("wp:author_email").cloned().unwrap_or_default(), a.get("wp:author_display_name").cloned().unwrap_or_default())); }
                    else { a.insert(name, v.trim().to_string()); }
                } else if let Some(t) = term.as_mut() {
                    if name == "wp:category" {
                        let t = term.take().unwrap();
                        if let (Some(id), Some(n)) = (t.get("wp:term_id"), t.get("wp:cat_name")) { w.terms.insert(id.clone(), n.clone()); }
                        // (nome, nome breve, nome breve della madre): per ricostruire l'albero delle categorie
                        if let Some(n) = t.get("wp:cat_name") { w.tree.push((n.clone(), t.get("wp:category_nicename").cloned().unwrap_or_default(), t.get("wp:category_parent").cloned().unwrap_or_default())); }
                    }
                    else { t.insert(name, v.trim().to_string()); }
                } else if name == "link" && stack.last().is_some_and(|p| p == "channel") && w.base.is_empty() {
                    w.base = v.trim().trim_end_matches('/').to_string();
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if w.items.is_empty() && w.authors.is_empty() { return Err("nel file non ci sono contenuti di WordPress: esporta da Strumenti > Esporta > Tutti i contenuti".into()) }
    Ok(w)
}

fn re(p: &str) -> Regex { Regex::new(p).expect("espressione regolare") }
static BLOCK_COMMENT: LazyLock<Regex> = LazyLock::new(|| re(r"<!--\s*/?wp:[^>]*-->"));
static CAPTION: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)\[caption[^\]]*\](.*?)\[/caption\]"));
static GALLERY: LazyLock<Regex> = LazyLock::new(|| re(r#"\[gallery[^\]]*?ids="([\d,\s]+)"[^\]]*\]"#));
static EMBED_SC: LazyLock<Regex> = LazyLock::new(|| re(r"\[embed[^\]]*\](.*?)\[/embed\]"));
static SHORTCODE: LazyLock<Regex> = LazyLock::new(|| re(r"\[/?[a-zA-Z_][\w-]*(?:\s[^\]]*)?\]"));
static WP_FIGURE: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<figure[^>]*class="[^"]*wp-block-image[^"]*"[^>]*>(.*?)</figure>"#));
static WP_EMBED: LazyLock<Regex> = LazyLock::new(|| re(r#"(?s)<figure[^>]*class="[^"]*wp-block-embed[^"]*"[^>]*>.*?(https?://[^\s<]+).*?</figure>"#));
static FIGCAPTION: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<figcaption[^>]*>(.*?)</figcaption>"));
static IMG: LazyLock<Regex> = LazyLock::new(|| re(r"<img\s[^>]*>"));
static SRC: LazyLock<Regex> = LazyLock::new(|| re(r#"\ssrc=["']([^"']+)["']"#));
static ALT: LazyLock<Regex> = LazyLock::new(|| re(r#"\salt=["']([^"']*)["']"#));
static YOUTUBE: LazyLock<Regex> = LazyLock::new(|| re(r"(?:youtube\.com/watch\?v=|youtu\.be/|youtube\.com/embed/|youtube\.com/shorts/)([\w-]{11})"));
static VIMEO: LazyLock<Regex> = LazyLock::new(|| re(r"vimeo\.com/(?:video/)?(\d+)"));
static EMPTY_P: LazyLock<Regex> = LazyLock::new(|| re(r"<p>(?:\s|&nbsp;|<br\s*/?>)*</p>"));
static BLOCK_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)<(p|div|h[1-6]|ul|ol|blockquote|figure|table)\b"));
static TAGS: LazyLock<Regex> = LazyLock::new(|| re(r"<[^>]+>"));
static SIZED: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(.+)-\d+x\d+(\.(?:jpe?g|png|gif|webp))(\?.*)?$"));

/// Entità HTML più comuni nei titoli di WordPress (&#8217; &amp; …).
fn unentity(s: &str) -> String {
    let named = s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#039;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ");
    let num = re(r"&#(x?[0-9a-fA-F]+);");
    num.replace_all(&named, |c: &regex::Captures| {
        let n = &c[1];
        let v = if let Some(h) = n.strip_prefix('x') { u32::from_str_radix(h, 16).ok() } else { n.parse().ok() };
        v.and_then(char::from_u32).map(String::from).unwrap_or_default()
    }).to_string()
}
fn text(html: &str) -> String { unentity(&TAGS.replace_all(html, " ")).split_whitespace().collect::<Vec<_>>().join(" ") }

fn video(url: &str) -> Option<String> {
    YOUTUBE.captures(url).map(|c| format!("https://www.youtube-nocookie.com/embed/{}", &c[1]))
        .or_else(|| VIMEO.captures(url).map(|c| format!("https://player.vimeo.com/video/{}", &c[1])))
}
fn video_html(url: &str) -> String {
    match video(url) {
        Some(src) => format!("<iframe class=\"ql-video\" frameborder=\"0\" allowfullscreen=\"true\" src=\"{src}\"></iframe>"),
        None => format!("<p><a href=\"{u}\">{u}</a></p>", u = site::esc(url)),
    }
}
fn figure(src: &str, alt: &str, caption: &str) -> String {
    let cap = if caption.trim().is_empty() { String::new() } else { format!("<figcaption>{}</figcaption>", site::esc(caption.trim())) };
    format!("<figure class=\"ps-figure\"><img src=\"{}\" alt=\"{}\">{cap}</figure>", site::esc(src), site::esc(alt))
}

/// Scarica le immagini del vecchio sito: solo indirizzi pubblici (mai la rete interna del server), al massimo 25 MB.
fn download(url: &str) -> R<Vec<u8>> {
    let local = crate::test_env("PRESSTATIC_TEST_LOCAL_FETCH").is_some(); // solo per i test automatici
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).redirects(5)
        .resolver(move |netloc: &str| -> std::io::Result<Vec<SocketAddr>> {
            let ok: Vec<SocketAddr> = netloc.to_socket_addrs()?.filter(|a| local || (crate::ai::public(a.ip()) && matches!(a.port(), 80 | 443))).collect();
            if ok.is_empty() { Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "indirizzo non consentito")) } else { Ok(ok) }
        }).build();
    let r = agent.get(url).set("User-Agent", "Mozilla/5.0 (compatible; Presstatic-import/1.0)").call().map_err(|e| e.to_string())?;
    let mut b = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(r.into_reader(), 25 * 1024 * 1024 + 1), &mut b).map_err(|e| e.to_string())?;
    if b.len() > 25 * 1024 * 1024 { return Err("immagine oltre 25 MB".into()) }
    Ok(b)
}

struct Images<'a> { app: &'a App, base: String, host: String, on: bool, done: HashMap<String, String> }
impl Images<'_> {
    fn abs(&self, u: &str) -> String { if u.starts_with("//") { format!("https:{u}") } else if u.starts_with('/') { format!("{}{u}", self.base) } else { u.to_string() } }
    /// Indirizzo nuovo per un'immagine del vecchio sito (scaricata una volta sola). Le altre restano com'erano.
    fn get(&mut self, url: &str, alt: &str, caption: &str) -> String {
        let u = unentity(&self.abs(url.trim()));
        if !self.on || !(u.contains(&self.host) || u.contains("/wp-content/uploads/")) { return u }
        // WordPress mette nel testo le versioni ridotte (foto-1024x683.jpg): si scarica l'originale (foto.jpg), una volta sola
        // e alla qualità migliore; se l'originale non c'è, la versione ridotta.
        let original = SIZED.replace(&u, "$1$2").to_string();
        if let Some(n) = self.done.get(&original).or_else(|| self.done.get(&u)) { return n.clone() }
        let name = original.split('?').next().unwrap_or("").rsplit('/').next().unwrap_or("immagine.jpg").to_string();
        let got = if original != u { download(&original).or_else(|_| download(&u)) } else { download(&u) };
        match got.and_then(|b| site::save_upload(self.app, &name, &b)) {
            Ok(n) => {
                if !alt.is_empty() || !caption.is_empty() {
                    let _ = self.app.db.lock().unwrap_or_else(|e| e.into_inner()).execute("UPDATE media SET alt = ?1, caption = ?2 WHERE url = ?3", params![alt, caption, n]);
                }
                st(|s| s.images += 1);
                self.done.insert(u, n.clone());
                self.done.insert(original, n.clone());
                n
            }
            Err(e) => { warn(format!("Immagine non scaricata, lasciato l'indirizzo originale: {u} ({e})")); u }
        }
    }
}

/// Converte il contenuto di WordPress (blocchi di Gutenberg, editor classico, shortcode) nell'HTML di Presstatic.
fn convert(content: &str, img: &mut Images, attachments: &HashMap<i64, (String, String, String)>) -> String {
    let mut h = BLOCK_COMMENT.replace_all(content, "").to_string();
    // Editor classico: si capisce dal testo originale (dopo, [caption] e [gallery] diventano già blocchi HTML).
    let classic = !BLOCK_TAG.is_match(&h);
    h = CAPTION.replace_all(&h, |c: &regex::Captures| {
        let inner = &c[1];
        let src = IMG.find(inner).and_then(|m| SRC.captures(m.as_str()).map(|s| s[1].to_string())).unwrap_or_default();
        let alt = IMG.find(inner).and_then(|m| ALT.captures(m.as_str()).map(|s| unentity(&s[1]))).unwrap_or_default();
        let caption = text(&IMG.replace_all(inner, ""));
        if src.is_empty() { String::new() } else { figure(&img.get(&src, &alt, &caption), &alt, &caption) }
    }).to_string();
    h = GALLERY.replace_all(&h, |c: &regex::Captures| {
        let figs: String = c[1].split(',').filter_map(|id| attachments.get(&id.trim().parse::<i64>().ok()?)).map(|(u, alt, cap)| figure(&img.get(u, alt, cap), alt, cap)).collect();
        if figs.is_empty() { String::new() } else { format!("<div class=\"ps-gallery cols-3\">{figs}</div>") }
    }).to_string();
    h = EMBED_SC.replace_all(&h, |c: &regex::Captures| video_html(c[1].trim())).to_string();
    h = WP_EMBED.replace_all(&h, |c: &regex::Captures| video_html(&c[1])).to_string();
    h = WP_FIGURE.replace_all(&h, |c: &regex::Captures| {
        let inner = &c[1];
        let tag = IMG.find(inner).map(|m| m.as_str().to_string()).unwrap_or_default();
        let src = SRC.captures(&tag).map(|s| s[1].to_string()).unwrap_or_default();
        let alt = ALT.captures(&tag).map(|s| unentity(&s[1])).unwrap_or_default();
        let caption = FIGCAPTION.captures(inner).map(|f| text(&f[1])).unwrap_or_default();
        if src.is_empty() { String::new() } else { figure(&img.get(&src, &alt, &caption), &alt, &caption) }
    }).to_string();
    h = SHORTCODE.replace_all(&h, "").to_string();
    // Immagini rimaste: attributi di WordPress tolti (srcset, classi, misure), le varianti le rifà Presstatic.
    h = IMG.replace_all(&h, |c: &regex::Captures| {
        let tag = &c[0];
        let (src, alt) = (SRC.captures(tag).map(|s| s[1].to_string()), ALT.captures(tag).map(|s| unentity(&s[1])).unwrap_or_default());
        if tag.contains("class=\"ps-") { return tag.to_string() }
        match src { Some(s) if !s.starts_with("/media/") => format!("<img src=\"{}\" alt=\"{}\">", site::esc(&img.get(&s, &alt, "")), site::esc(&alt)), Some(s) => format!("<img src=\"{s}\" alt=\"{}\">", site::esc(&alt)), None => String::new() }
    }).to_string();
    // Editor classico: paragrafi separati da righe vuote, senza <p> (è la funzione wpautop di WordPress).
    if classic {
        h = h.split("\n\n").map(str::trim).filter(|p| !p.is_empty())
            .map(|p| if BLOCK_TAG.find(p).is_some_and(|m| m.start() == 0) || p.starts_with("<iframe") { p.to_string() } else { format!("<p>{}</p>", p.replace('\n', "<br>")) })
            .collect::<Vec<_>>().join("\n");
    }
    EMPTY_P.replace_all(&h, "").trim().to_string()
}

fn epoch(date_gmt: &str) -> Option<i64> {
    let d = date_gmt.trim();
    if d.is_empty() || d.starts_with("0000") { return None }
    let dt: jiff::civil::DateTime = d.replace(' ', "T").parse().ok()?;
    dt.to_zoned(jiff::tz::TimeZone::UTC).ok().map(|z| z.timestamp().as_second())
}

/// Percorso del vecchio indirizzo, se diverso da /slug/ (quello di Presstatic): diventa un reindirizzamento.
fn old_path(link: &str, base: &str, slug: &str) -> Option<String> {
    let path = link.strip_prefix(base).unwrap_or(link);
    // decodificato: il server web cerca la cartella «città-vecchia», non «citt%c3%a0-vecchia»
    let path = urlish(path.split(['?', '#']).next().unwrap_or("")).trim_matches('/').to_lowercase();
    let ok = !path.is_empty() && path != slug && path.len() < 300 && !path.contains("..") && !path.contains("//")
        && path.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'));
    ok.then_some(path)
}

/// Esegue l'importazione (in un thread a parte): legge, crea autori, articoli e pagine, poi rigenera il sito.
pub fn run(app: &App, xml: String, o: Options) {
    let result = (|| -> R<()> {
        st(|s| s.phase = "Lettura del file".into());
        let w = parse(&xml)?;
        drop(xml);
        let host = w.base.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("").trim_start_matches("www.").to_string();
        let attachments: HashMap<i64, (String, String, String)> = w.items.iter().filter(|i| i.typ == "attachment" && !i.attachment_url.is_empty())
            .map(|i| (i.id, (i.attachment_url.clone(), i.meta.get("_wp_attachment_image_alt").cloned().unwrap_or_default(), text(&i.excerpt)))).collect();
        let content: Vec<&Item> = w.items.iter().filter(|i| matches!(i.typ.as_str(), "post" | "page") && matches!(i.status.as_str(), "publish" | "future" | "draft" | "pending" | "private")).collect();
        st(|s| { s.total = content.len(); s.phase = "Autori".into() });
        // Albero delle categorie: Sport > Calcio resta così anche in Presstatic.
        {
            let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
            let skip = |n: &str| matches!(n.to_lowercase().as_str(), "uncategorized" | "senza categoria");
            for (name, _, parent) in &w.tree {
                let name = unentity(name);
                if skip(&name) { continue }
                let parent = w.tree.iter().find(|t| !parent.is_empty() && t.1 == *parent).map(|t| unentity(&t.0)).unwrap_or_default();
                let _ = db.execute("INSERT INTO categories(name, parent) VALUES (?1, ?2) ON CONFLICT(name) DO UPDATE SET parent = CASE WHEN parent = '' THEN ?2 ELSE parent END", params![name, parent]);
            }
        }

        // Autori: chi ha già un account lo mantiene; gli altri diventano autori e scelgono la password con «Password dimenticata?».
        let mut users: HashMap<String, i64> = HashMap::new();
        if o.authors {
            for (login, (email, name)) in &w.authors {
                let email = email.trim().to_lowercase();
                if email.is_empty() || !email.contains('@') { continue }
                let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
                let id = match db.query_row("SELECT id FROM users WHERE email = ?1", [&email], |r| r.get::<_, i64>(0)) {
                    Ok(id) => id,
                    Err(_) => {
                        let name = if name.trim().is_empty() { login.clone() } else { unentity(name.trim()) };
                        let slug = site::unique_user_slug(&db, &name, 0);
                        let pass = crate::hash_password(&crate::twofa::token());
                        if db.execute("INSERT INTO users(name, email, pass, role, slug) VALUES (?1, ?2, ?3, 'author', ?4)", params![name, email, pass, slug]).is_err() { continue }
                        st(|s| s.users += 1);
                        db.last_insert_rowid()
                    }
                };
                users.insert(login.clone(), id);
            }
        }

        let mut img = Images { app, base: w.base.clone(), host, on: o.images, done: HashMap::new() };
        for (n, it) in content.iter().enumerate() {
            st(|s| { s.done = n; s.phase = format!("Articoli e pagine: {} di {}", n + 1, s.total) });
            if app.db.lock().unwrap_or_else(|e| e.into_inner()).query_row("SELECT 1 FROM wp_import WHERE wp_id = ?1", [it.id], |_| Ok(())).is_ok() {
                st(|s| s.skipped += 1); // già importato in un giro precedente
                continue;
            }
            let title = unentity(it.title.trim());
            // Prima la foto in evidenza: nella libreria entrano testo alternativo e didascalia della sua scheda in WordPress
            // (la stessa foto usata nel testo tiene la didascalia scritta lì, dentro l'articolo).
            let image = it.meta.get("_thumbnail_id").and_then(|t| t.parse::<i64>().ok()).and_then(|t| attachments.get(&t))
                .map(|(u, alt, cap)| img.get(u, alt, cap)).filter(|u| u.starts_with("/media/")).unwrap_or_default();
            let body = convert(&it.content, &mut img, &attachments);
            let mut desc = text(&it.excerpt);
            if desc.is_empty() { desc = it.meta.get("_yoast_wpseo_metadesc").map(|d| unentity(d)).unwrap_or_default() }
            let desc: String = desc.chars().take(300).collect();
            let cat = it.meta.get("_yoast_wpseo_primary_category").and_then(|t| w.terms.get(t)).cloned()
                .or_else(|| it.cats.iter().find(|c| c.0 == "category").map(|c| unentity(&c.1)))
                .filter(|c| !matches!(c.to_lowercase().as_str(), "uncategorized" | "senza categoria")).unwrap_or_default();
            let tags: Vec<String> = it.cats.iter().filter(|c| c.0 == "post_tag").map(|c| unentity(&c.1)).collect();
            // le altre categorie dell'articolo diventano categorie aggiuntive
            let extra: Vec<String> = it.cats.iter().filter(|c| c.0 == "category").map(|c| unentity(&c.1))
                .filter(|c| *c != cat && !matches!(c.to_lowercase().as_str(), "uncategorized" | "senza categoria")).take(10).collect();
            let status: &'static str = match it.status.as_str() { "publish" | "future" => "published", "pending" => "pending", _ => "draft" };
            let at = epoch(&it.date_gmt).unwrap_or_else(now);
            let author = users.get(&it.creator).copied().unwrap_or(o.owner);
            let kind = if it.typ == "page" { "page" } else { "post" };
            let wanted = site::slugify(&if it.slug.is_empty() { title.clone() } else { unentity(&urlish(&it.slug)) });
            let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
            let slug = site::free_slug(app, &db, &wanted);
            // Nascosto (bozza) finché l'importazione non finisce: nessun elenco del sito lo mostra prima che abbia la sua pagina.
            let r = db.execute("INSERT INTO posts(slug, title, description, body, category, image, schema_type, schema_data, status, published_at, updated_at, tags, author_id, kind, categories) VALUES (?1,?2,?3,?4,?5,?6,'NewsArticle','{}','draft',?7,?8,?9,?10,?11,?12)",
                params![slug, title, desc, body, cat, image, at, now(), tags.join(", "), author, kind, extra.join(", ")]);
            if let Err(e) = r { warn(format!("«{title}» non importato: {e}")); continue }
            let id = db.last_insert_rowid();
            let _ = db.execute("INSERT INTO revisions(post_id, user_id, title, description, body, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)", params![id, o.owner, title, desc, body, now()]);
            let _ = site::fts_put(&db, id, &title, &desc, &body);
            for c in std::iter::once(&cat).chain(extra.iter()).filter(|c| !c.is_empty()) { let _ = db.execute("INSERT OR IGNORE INTO categories(name) VALUES (?1)", [c]); }
            let _ = db.execute("INSERT INTO wp_import(wp_id, post_id, status) VALUES (?1, ?2, ?3)", params![it.id, id, status]);
            if status == "published" {
                if let Some(path) = old_path(&it.link, &w.base, &slug) {
                    if db.execute("INSERT OR IGNORE INTO redirects(slug, post_id) VALUES (?1, ?2)", params![path, id]).unwrap_or(0) == 1 { st(|s| s.redirects += 1) }
                }
            }
            st(|s| if kind == "page" { s.pages += 1 } else { s.posts += 1 });
        }
        st(|s| { s.done = s.total; s.phase = "Pubblicazione e rigenerazione del sito".into() });
        {
            // Tutti insieme online, compresi quelli rimasti nascosti da un'importazione interrotta prima della fine.
            let _gen = site::gen_lock();
            let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
            let _ = db.execute_batch("BEGIN; UPDATE posts SET status = (SELECT i.status FROM wp_import i WHERE i.post_id = posts.id) WHERE id IN (SELECT post_id FROM wp_import WHERE applied = 0); UPDATE wp_import SET applied = 1 WHERE applied = 0; COMMIT;");
        }
        site::rebuild_all(app)?;
        app.search_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    })();
    st(|s| {
        s.running = false; s.finished = true;
        match &result { Ok(()) => s.phase = "Importazione completata".into(), Err(e) => { s.phase = "Importazione interrotta".into(); s.errors.insert(0, e.clone()) } }
    });
}

/// Gli slug di WordPress possono contenere caratteri codificati (%c3%a0): si decodificano prima di rifarli.
fn urlish(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let (mut out, mut i) = (Vec::with_capacity(b.len()), 0);
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) { out.push(h * 16 + l); i += 3; continue }
        }
        out.push(b[i]); i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}
