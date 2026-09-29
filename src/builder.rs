//! Page builder: la redazione compone la pagina con sezioni, colonne e widget (come Elementor); Presstatic salva la
//! struttura in JSON e alla pubblicazione la trasforma in HTML e CSS statici. Sul sito non arriva nessuno script
//! dell'editor: la pagina resta veloce come quelle disegnate dal tema.
use crate::{opt, site::{self, esc, Post}, Settings};
use serde_json::{json, Value};
use std::collections::HashSet;

pub struct Ctx<'a> {
    pub st: &'a Settings,
    pub posts: &'a [Post],
    pub items: &'a [Value],
    pub editing: bool,
    pub newsletter: String,
    pub name: &'a str,                  // "home", "header" o "footer"
    pub cats: Vec<(String, String)>,    // sezioni principali del giornale: (nome, indirizzo)
    pub forms: std::collections::HashMap<i64, String>, // moduli pronti (pannello > Moduli), per il widget «Modulo»
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str { v["settings"][k].as_str().unwrap_or("") }
fn n(v: &Value, k: &str, d: f64) -> f64 { v["settings"][k].as_f64().or_else(|| v["settings"][k].as_str().and_then(|x| x.parse().ok())).unwrap_or(d) }
fn on(v: &Value, k: &str) -> bool { matches!(&v["settings"][k], Value::Bool(true)) || v["settings"][k].as_str() == Some("on") }
/// Solo colori CSS semplici (#abc, #aabbcc, rgb(…)): niente valori che possano uscire dalla regola.
fn color(c: &str) -> Option<&str> { let c = c.trim(); (!c.is_empty() && c.len() < 40 && c.chars().all(|x| x.is_ascii_alphanumeric() || "#(),. %".contains(x))).then_some(c) }
fn id(v: &Value) -> String { v["id"].as_str().unwrap_or("x").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(24).collect() }

/// Regole CSS per computer, tablet (fino a 1024 px) e telefono (fino a 767 px).
#[derive(Default)]
struct Css { d: String, t: String, m: String }
impl Css {
    /// Proprietà che cambia per dispositivo: `key`, `key_t`, `key_m` nelle impostazioni.
    fn resp(&mut self, sel: &str, v: &Value, key: &str, prop: &str, unit: &str) {
        for (suffix, out) in [("", &mut self.d), ("_t", &mut self.t), ("_m", &mut self.m)] {
            if let Some(x) = v["settings"][format!("{key}{suffix}")].as_f64().or_else(|| v["settings"][format!("{key}{suffix}")].as_str().and_then(|x| x.parse().ok())) {
                out.push_str(&format!("{sel}{{{prop}:{x}{unit}}}"));
            }
        }
    }
    fn hide(&mut self, sel: &str, v: &Value) {
        if on(v, "hide_d") { self.d += &format!("{sel}{{display:none!important}}") }
        if on(v, "hide_t") { self.t += &format!("{sel}{{display:none!important}}") }
        if on(v, "hide_m") { self.m += &format!("{sel}{{display:none!important}}") }
    }
    /// Spaziature e colori comuni a tutti gli elementi (scheda «Avanzate» dell'editor).
    fn box_(&mut self, sel: &str, v: &Value) {
        self.resp(sel, v, "mt", "margin-top", "px");
        self.resp(sel, v, "mb", "margin-bottom", "px");
        self.resp(sel, v, "pad", "padding", "px");
        if let Some(c) = color(s(v, "bg")) { self.d += &format!("{sel}{{background:{c}}}") }
        if let Some(c) = color(s(v, "color")) { self.d += &format!(":where({sel}),:where({sel}) a{{color:{c}}}") }
        if n(v, "radius", 0.0) > 0.0 { self.d += &format!("{sel}{{border-radius:{}px;overflow:hidden}}", n(v, "radius", 0.0)) }
        if matches!(s(v, "align"), "left" | "center" | "right") { self.d += &format!("{sel}{{text-align:{}}}", s(v, "align")) }
        self.hide(sel, v);
    }
    fn out(self) -> String {
        format!("{}@media (max-width:1024px){{{}}}@media (max-width:767px){{{}}}", self.d, self.t, self.m)
    }
}

/// Stile di base del builder: griglia delle sezioni e dei blocchi di articoli. Usa le variabili del tema (colori, bordi).
const BASE: &str = ".pb{display:block}.pb-s{padding:24px 0}.pb-in{max-width:var(--pb-max,1200px);margin:0 auto;padding:0 16px;display:flex;gap:var(--pb-gap,24px);align-items:flex-start}\
.pb-full>.pb-in{max-width:none}.pb-c{flex:1 1 0;min-width:0;display:flex;flex-direction:column}.pb-w{min-width:0}.pb-w+.pb-w{margin-top:20px}\
.pb-h{margin:0;line-height:1.15}.pb-t p{margin:0 0 .8em}.pb-t p:last-child{margin:0}.pb-img img{display:block;max-width:100%;height:auto}.pb-img figcaption{font-size:.85em;opacity:.8;margin-top:.4em}\
.pb-btn{display:inline-block;padding:.75em 1.4em;border-radius:8px;background:var(--accent,#1e6bff);color:#fff;font-weight:700;text-decoration:none}\
.pb-grid{display:grid;grid-template-columns:repeat(var(--cols,3),minmax(0,1fr));gap:20px}.pb-grid .teaser img{width:100%;height:auto;aspect-ratio:16/10;object-fit:cover}\
.pb-title{font-size:1.15rem;margin:0 0 .8rem;padding-bottom:.4rem;border-bottom:2px solid var(--accent,#1e6bff)}\
.pb-list{list-style:none;margin:0;padding:0}.pb-list li{padding:.55rem 0;border-bottom:1px solid var(--line,#e5e5e5)}.pb-list time{font-size:.8em;opacity:.75;margin-right:.5em}\
.pb-num{counter-reset:pb}.pb-num li{counter-increment:pb;display:flex;gap:.7rem}.pb-num li::before{content:counter(pb);font-weight:800;font-size:1.5rem;line-height:1;color:var(--accent,#1e6bff);min-width:1.3rem}\
.pb-hero{display:grid;grid-template-columns:3fr 2fr;gap:24px;align-items:center}.pb-hero img{width:100%;height:auto;aspect-ratio:16/9;object-fit:cover}.pb-hero h2{font-size:clamp(1.5rem,3vw,2.4rem);margin:.3rem 0}\
.pb-over{position:relative;display:block}.pb-over .pb-cap{position:absolute;inset:auto 0 0 0;padding:28px;background:linear-gradient(transparent,rgba(0,0,0,.8));color:#fff}.pb-over .pb-cap a,.pb-over .pb-cap h2{color:#fff}\
.pb-video{position:relative;aspect-ratio:16/9}.pb-video iframe{position:absolute;inset:0;width:100%;height:100%;border:0}.pb-hr{border:0;margin:0}\
.pb-logo{display:inline-flex;align-items:center;font-weight:800;font-size:1.6rem;text-decoration:none;color:inherit}.pb-logo img{display:block;height:var(--h,48px);width:auto}.pb-tag{margin:.3rem 0 0;opacity:.8;font-size:.9rem}\
.pb-menu{display:flex;flex-wrap:wrap;gap:.2rem 1.4rem}.pb-menu a{text-decoration:none;font-weight:700;padding:.35rem 0}.pb-menu a:hover{color:var(--accent,#1e6bff)}.pb-up a{text-transform:uppercase;letter-spacing:.04em;font-size:.88em}\
.pb-burger{display:none}.pb-burger summary{list-style:none;cursor:pointer;display:inline-flex;align-items:center;gap:.5rem;font-weight:800;padding:.5rem .9rem;border:1px solid var(--line,#ddd);border-radius:8px}.pb-burger summary::-webkit-details-marker{display:none}.pb-burger nav{display:grid;margin-top:.6rem}.pb-burger nav a{padding:.6rem 0;border-bottom:1px solid var(--line,#eee);text-decoration:none;font-weight:700}\
.pb-search{display:inline-flex;align-items:center;gap:.45rem;font-weight:700;text-decoration:none;padding:.5rem .9rem;border:1px solid var(--line,#ddd);border-radius:999px}\
.pb-soc{display:flex;flex-wrap:wrap;gap:.5rem}.pb-soc a{display:inline-flex;padding:.45rem .9rem;border-radius:999px;background:var(--c,var(--wash,#eef1f6));color:var(--t,inherit);font-weight:700;font-size:.88rem;text-decoration:none}\
.pb-copy{font-size:.85rem;opacity:.9}.pb-copy p{margin:.2rem 0}\
.pb-tabs{display:flex;flex-wrap:wrap;gap:0 .3rem}.pb-tabs>input{position:absolute;opacity:0;pointer-events:none}.pb-tabs>label{padding:.65rem 1rem;cursor:pointer;font-weight:700;border-bottom:3px solid transparent}\
.pb-tabs>input:focus-visible+label{outline:2px solid var(--accent,#1e6bff);outline-offset:2px}.pb-panels{flex-basis:100%;border-top:1px solid var(--line,#ddd)}.pb-panel{display:none;padding:1rem 0}.pb-panel p{margin:0 0 .8em}\
.pb-acc details{border-bottom:1px solid var(--line,#ddd)}.pb-acc summary{cursor:pointer;padding:.9rem 0;font-weight:700;list-style:none;display:flex;justify-content:space-between;gap:1rem}.pb-acc summary::-webkit-details-marker{display:none}\
.pb-acc summary::after{content:\"+\";font-size:1.3em;line-height:1}.pb-acc details[open] summary::after{content:\"–\"}.pb-acc details>div{padding:0 0 1rem}.pb-acc details>div p{margin:0 0 .7em}\
.pb-car{display:grid;grid-auto-flow:column;grid-auto-columns:min(80%,300px);gap:18px;overflow-x:auto;scroll-snap-type:x mandatory;padding-bottom:10px;overscroll-behavior-x:contain}.pb-car>*{scroll-snap-align:start}.pb-car .teaser img{width:100%;height:auto;aspect-ratio:16/10;object-fit:cover}\
.pb-gal{display:grid;grid-template-columns:repeat(var(--cols,3),minmax(0,1fr));gap:10px}.pb-gal img{display:block;width:100%;height:auto;aspect-ratio:1;object-fit:cover}\
.pb-num{font-weight:800;line-height:1;font-size:3rem;color:var(--accent,#1e6bff)}.pb-num-l{margin:.4rem 0 0;font-weight:600}\
.pb-quote{margin:0;padding:1.4rem 1.6rem;border-left:4px solid var(--accent,#1e6bff);background:var(--soft,#f4f6fa);border-radius:0 12px 12px 0}.pb-quote blockquote{margin:0;font-size:1.2rem;line-height:1.5;font-style:italic}\
.pb-quote figcaption{display:flex;align-items:center;gap:.7rem;margin-top:1rem;font-weight:700}.pb-quote figcaption img{width:44px;height:44px;border-radius:50%;object-fit:cover}.pb-quote small{display:block;font-weight:500;opacity:.8}\
.pb-card{border:1px solid var(--line,#e2e5ec);border-radius:12px;overflow:hidden;background:var(--bg,#fff)}.pb-card img{display:block;width:100%;height:auto;aspect-ratio:16/10;object-fit:cover}.pb-card>div{padding:1.1rem 1.2rem}.pb-card h3{margin:0 0 .4rem}.pb-card p{margin:0 0 .9rem}\
.pb-ul{margin:0;padding-left:1.2em}.pb-check{list-style:none;padding:0}.pb-check li::before{content:\"✓ \";color:var(--accent,#1e6bff);font-weight:800}\
@media (max-width:1024px){.pb-grid{--cols:2}}@media (max-width:767px){.pb-menu.pb-collapse{display:none}.pb-burger{display:block}.pb-in{flex-direction:column}.pb-c{width:100%}.pb-grid{--cols:1}.pb-hero{grid-template-columns:1fr}}";

struct Render<'a> { cx: &'a Ctx<'a>, css: Css, shown: HashSet<i64>, dedupe: bool, ghost: Option<String> }

impl Render<'_> {
    /// Articoli per un blocco: categoria (anche aggiuntive e sottocategorie), quanti, da quale saltare; senza ripetere quelli già mostrati.
    fn pick(&mut self, v: &Value, default: usize, featured_first: bool) -> Vec<Value> {
        let cat = site::slugify(s(v, "category"));
        let want = (n(v, "count", default as f64) as usize).clamp(1, 30);
        let skip = n(v, "skip", 0.0) as usize;
        let mut idx: Vec<usize> = (0..self.cx.posts.len()).filter(|&i| cat.is_empty() || site::post_cats(&self.cx.posts[i]).iter().any(|c| site::slugify(c) == cat)).collect();
        if featured_first { idx.sort_by_key(|&i| !self.cx.posts[i].featured) }
        let all = idx.clone();
        let out: Vec<usize> = idx.into_iter().filter(|i| !self.dedupe || !self.shown.contains(&self.cx.posts[*i].id)).skip(skip).take(want).collect();
        // Nell'editor, se gli articoli ci sono ma sono già mostrati più in alto, il blocco li fa vedere lo stesso
        // (in trasparenza, con una nota): così si capisce l'impaginazione anche con pochi articoli pubblicati.
        if out.is_empty() && self.cx.editing && self.dedupe {
            let alt: Vec<usize> = all.into_iter().skip(skip).take(want).collect();
            if !alt.is_empty() { self.ghost = Some("Anteprima: questi articoli sono già mostrati più in alto. Sul sito qui compariranno i successivi.".into()); return alt.into_iter().map(|i| self.cx.items[i].clone()).collect() }
        }
        // …e se la sezione scelta non ha ancora articoli ma il sito sì, l'anteprima mostra gli ultimi pubblicati.
        if out.is_empty() && self.cx.editing && !cat.is_empty() && !self.cx.posts.is_empty() {
            self.ghost = Some(format!("Anteprima: in «{}» non ci sono ancora articoli, qui vedi gli ultimi pubblicati. Sul sito compariranno quelli della sezione.", esc(s(v, "category"))));
            return (0..self.cx.posts.len()).take(want).map(|i| self.cx.items[i].clone()).collect();
        }
        for &i in &out { self.shown.insert(self.cx.posts[i].id); }
        out.into_iter().map(|i| self.cx.items[i].clone()).collect()
    }

    fn card(&self, p: &Value, v: &Value, sizes: &str) -> String {
        let img = if on(v, "no_img") || p["img"].as_str().unwrap_or("").is_empty() { String::new() } else { format!("<a class=\"pic\" href=\"{}\" tabindex=\"-1\" aria-hidden=\"true\"><img {} sizes=\"{sizes}\" alt=\"\" loading=\"lazy\"></a>", esc(p["url"].as_str().unwrap_or("")), p["img"].as_str().unwrap_or("")) };
        let kick = if p["category"].as_str().unwrap_or("").is_empty() { String::new() } else { format!("<a class=\"kicker\" href=\"{}\">{}</a>", esc(p["cat_url"].as_str().unwrap_or("")), esc(p["category"].as_str().unwrap_or(""))) };
        let desc = if on(v, "show_desc") && !p["description"].as_str().unwrap_or("").is_empty() { format!("<p class=\"dek\">{}</p>", esc(p["description"].as_str().unwrap_or(""))) } else { String::new() };
        format!("<article class=\"teaser card\">{img}<div>{kick}<h3><a href=\"{}\">{}</a></h3>{desc}<p class=\"meta\"><time datetime=\"{}\">{}</time></p></div></article>",
            esc(p["url"].as_str().unwrap_or("")), esc(p["title"].as_str().unwrap_or("")), esc(p["date_iso"].as_str().unwrap_or("")), esc(p["date"].as_str().unwrap_or("")))
    }

    fn title(&self, v: &Value) -> String { if s(v, "title").is_empty() { String::new() } else { format!("<h2 class=\"pb-title\">{}</h2>", esc(s(v, "title"))) } }
    /// Nell'editor: questa parte si scrive direttamente nell'anteprima (campo del widget; «para» paragrafi, «lines» righe).
    fn ed(&self, field: &str, multi: &str) -> String {
        if !self.cx.editing { return String::new() }
        if multi.is_empty() { format!(" data-pb-edit=\"{field}\"") } else { format!(" data-pb-edit=\"{field}\" data-pb-multi=\"{multi}\"") }
    }
    fn empty(&self, what: &str) -> String { if self.cx.editing { format!("<p class=\"pb-empty\">{what}</p>") } else { String::new() } }
    /// Schede grigie d'esempio con la forma del blocco: solo nell'editor, quando non ci sono articoli da mostrare.
    fn skel(&self, cards: usize, what: &str) -> String {
        if !self.cx.editing { return String::new() }
        let one = "<div class=\"pb-skel-card\"><i></i><b></b><b></b><b class=\"s\"></b></div>".repeat(cards.clamp(1, 6));
        format!("<div class=\"pb-skel\"><div class=\"pb-skel-grid\" style=\"--n:{}\">{one}</div><p class=\"pb-note\">{what}</p></div>", cards.clamp(1, 4))
    }
    /// Un modulo d'esempio (newsletter o modulo non attivo): solo nell'editor.
    fn mock(&self, title: &str, what: &str) -> String {
        if !self.cx.editing { return String::new() }
        format!("<div class=\"pb-mock\"><strong>{title}</strong><div class=\"pb-mock-row\"><span></span><b></b></div><p class=\"pb-note\">{what}</p></div>")
    }

    fn widget(&mut self, w: &Value) -> String {
        let sel = format!("#pb-{}", id(w));
        self.css.box_(&sel, w);
        let body = match w["type"].as_str().unwrap_or("") {
            "heading" => {
                let tag = match s(w, "tag") { "h1" => "h1", "h3" => "h3", "h4" => "h4", _ => "h2" };
                self.css.resp(&format!("{sel} .pb-h"), w, "size", "font-size", "px");
                let text = esc(if s(w, "text").is_empty() { "Titolo" } else { s(w, "text") });
                let inner = if s(w, "link").is_empty() { text } else { format!("<a href=\"{}\">{text}</a>", esc(s(w, "link"))) };
                format!("<{tag} class=\"pb-h\"{}>{inner}</{tag}>", self.ed("text", ""))
            }
            "text" => {
                self.css.resp(&format!("{sel} .pb-t"), w, "size", "font-size", "px");
                format!("<div class=\"pb-t\"{}>{}</div>", self.ed("text", "para"), s(w, "text").split("\n\n").filter(|p| !p.trim().is_empty()).map(|p| format!("<p>{}</p>", esc(p.trim()).replace('\n', "<br>"))).collect::<String>())
            }
            "image" => {
                self.css.resp(&format!("{sel} img"), w, "width", "width", "%");
                let src = s(w, "src");
                if src.is_empty() { if self.cx.editing { "<div class=\"pb-ph-img\" data-pb-pick><svg width=\"44\" height=\"44\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.6\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\"><rect x=\"3\" y=\"4\" width=\"18\" height=\"16\" rx=\"2\"/><circle cx=\"9\" cy=\"10\" r=\"2\"/><path d=\"m21 17-5-5-9 8\"/></svg><strong>Scegli un'immagine</strong><span>Clicca per aprire la libreria</span></div>".to_string() } else { String::new() } } else {
                    let attrs = if src.starts_with("/media/") { crate::media::img_attrs(src, src) } else { format!("src=\"{}\"", esc(src)) };
                    let img = format!("<img {attrs} alt=\"{}\" loading=\"lazy\" sizes=\"(min-width: 1200px) 1200px, 100vw\">", esc(s(w, "alt")));
                    let img = if s(w, "link").is_empty() { img } else { format!("<a href=\"{}\">{img}</a>", esc(s(w, "link"))) };
                    let cap = if s(w, "caption").is_empty() { String::new() } else { format!("<figcaption>{}</figcaption>", esc(s(w, "caption"))) };
                    format!("<figure class=\"pb-img\" style=\"margin:0\">{img}{cap}</figure>")
                }
            }
            "button" => {
                if let Some(c) = color(s(w, "btn_bg")) { self.css.d += &format!("{sel} .pb-btn{{background:{c}}}") }
                if let Some(c) = color(s(w, "btn_color")) { self.css.d += &format!("{sel} .pb-btn{{color:{c}}}") }
                let blank = if on(w, "newtab") { " target=\"_blank\" rel=\"noopener\"" } else { "" };
                format!("<a class=\"pb-btn\" href=\"{}\"{blank}{}>{}</a>", esc(if s(w, "url").is_empty() { "#" } else { s(w, "url") }), self.ed("label", ""), esc(if s(w, "label").is_empty() { "Scopri di più" } else { s(w, "label") }))
            }
            "divider" => {
                let style = match s(w, "style") { "dashed" => "dashed", "dotted" => "dotted", _ => "solid" };
                format!("<hr class=\"pb-hr\" style=\"border-top:{}px {style} {};width:{}%\">", n(w, "thick", 1.0).clamp(1.0, 20.0), color(s(w, "line")).unwrap_or("var(--line,#ddd)"), n(w, "width", 100.0).clamp(5.0, 100.0))
            }
            "spacer" => { self.css.resp(&format!("{sel} .pb-sp"), w, "h", "height", "px"); format!("<div class=\"pb-sp\" style=\"height:{}px\"></div>", n(w, "h", 40.0).clamp(0.0, 400.0)) }
            "video" => {
                let url = s(w, "url");
                let yt = url.split(['=', '/']).filter(|x| x.len() == 11).last().filter(|_| url.contains("youtu"));
                let vm = url.contains("vimeo.com").then(|| url.rsplit('/').next().unwrap_or("").chars().filter(char::is_ascii_digit).collect::<String>()).filter(|x| !x.is_empty());
                let src = yt.map(|i| format!("https://www.youtube-nocookie.com/embed/{i}")).or(vm.map(|i| format!("https://player.vimeo.com/video/{i}")));
                match src { Some(src) => format!("<div class=\"pb-video\"><iframe src=\"{src}\" title=\"{}\" loading=\"lazy\" allowfullscreen referrerpolicy=\"strict-origin-when-cross-origin\"></iframe></div>", esc(if s(w, "title").is_empty() { "Video" } else { s(w, "title") })), None => self.empty("Incolla l'indirizzo di un video di YouTube o Vimeo") }
            }
            "list" => {
                let cls = match s(w, "marker") { "check" => "pb-ul pb-check", "number" => "pb-ul", _ => "pb-ul" };
                let tag = if s(w, "marker") == "number" { "ol" } else { "ul" };
                format!("<{tag} class=\"{cls}\"{}>{}</{tag}>", self.ed("items", "lines"), s(w, "items").lines().filter(|l| !l.trim().is_empty()).map(|l| format!("<li>{}</li>", esc(l.trim()))).collect::<String>())
            }
            "posts_grid" => {
                self.css.resp(&format!("{sel} .pb-grid"), w, "cols", "--cols", "");
                let posts = self.pick(w, 6, false);
                if posts.is_empty() { self.skel(n(w, "cols", 3.0) as usize, "Qui compariranno gli articoli: in questa scelta non ce ne sono ancora.") } else {
                    let cols = n(w, "cols", 3.0).clamp(1.0, 6.0);
                    let sizes = format!("(min-width: 1200px) {}px, (min-width: 768px) 50vw, 100vw", (1200.0 / cols) as i64);
                    format!("{}<div class=\"pb-grid\" style=\"--cols:{cols}\">{}</div>", self.title(w), posts.iter().map(|p| self.card(p, w, &sizes)).collect::<String>())
                }
            }
            "posts_list" | "latest" => {
                let posts = self.pick(w, if w["type"] == "latest" { 8 } else { 5 }, false);
                let time = w["type"] == "latest" || on(w, "show_time");
                format!("{}<ul class=\"pb-list\">{}</ul>", self.title(w), posts.iter().map(|p| format!("<li>{}<a href=\"{}\">{}</a></li>",
                    if time { format!("<time datetime=\"{}\">{}</time>", esc(p["date_iso"].as_str().unwrap_or("")), esc(p["time"].as_str().unwrap_or(""))) } else { String::new() },
                    esc(p["url"].as_str().unwrap_or("")), esc(p["title"].as_str().unwrap_or("")))).collect::<String>())
            }
            "hero" => {
                let posts = self.pick(w, 1, s(w, "mode") != "latest");
                match posts.first() {
                    None => self.skel(1, "Qui comparirà l'articolo d'apertura: non ce ne sono ancora."),
                    Some(p) => {
                        let (url, title, img) = (esc(p["url"].as_str().unwrap_or("")), esc(p["title"].as_str().unwrap_or("")), p["img"].as_str().unwrap_or(""));
                        let kick = if p["category"].as_str().unwrap_or("").is_empty() { String::new() } else { format!("<a class=\"kicker\" href=\"{}\">{}</a>", esc(p["cat_url"].as_str().unwrap_or("")), esc(p["category"].as_str().unwrap_or(""))) };
                        let desc = esc(p["description"].as_str().unwrap_or(""));
                        let pic = if img.is_empty() { String::new() } else { format!("<img {img} sizes=\"(min-width: 1200px) 720px, 100vw\" alt=\"\" fetchpriority=\"high\">") };
                        if s(w, "layout") == "overlay" {
                            format!("<a class=\"pb-over teaser\" href=\"{url}\">{pic}<div class=\"pb-cap\"><h2>{title}</h2>{}</div></a>", if desc.is_empty() { String::new() } else { format!("<p>{desc}</p>") })
                        } else {
                            format!("<article class=\"pb-hero teaser\"><a href=\"{url}\" tabindex=\"-1\" aria-hidden=\"true\">{pic}</a><div>{kick}<h2><a href=\"{url}\">{title}</a></h2>{}<p class=\"meta\"><time datetime=\"{}\">{}</time></p></div></article>",
                                if desc.is_empty() { String::new() } else { format!("<p class=\"dek\">{desc}</p>") }, esc(p["date_iso"].as_str().unwrap_or("")), esc(p["date"].as_str().unwrap_or("")))
                        }
                    }
                }
            }
            "most_read" => {
                let ids: Vec<i64> = serde_json::from_str(opt(self.cx.st, "most_read", "[]")).unwrap_or_default();
                let want = n(w, "count", 5.0) as usize;
                let list: Vec<&Value> = ids.iter().filter_map(|id| self.cx.items.iter().find(|p| p["id"].as_i64() == Some(*id))).take(want).collect();
                if list.is_empty() { self.skel(1, "I più letti arrivano da Google Analytics: collegalo in Integrazioni › Più letti.") } else {
                    format!("{}<ol class=\"pb-list pb-num\">{}</ol>", self.title(w), list.iter().map(|p| format!("<li><a href=\"{}\">{}</a></li>", esc(p["url"].as_str().unwrap_or("")), esc(p["title"].as_str().unwrap_or("")))).collect::<String>())
                }
            }
            "form" => match s(w, "form").parse::<i64>().ok().and_then(|id| self.cx.forms.get(&id)) {
                Some(h) => h.clone(),
                None => self.mock("Modulo", "Scegli quale modulo nelle impostazioni del widget (i moduli si creano in pannello › Moduli)."),
            },
            "newsletter" => if self.cx.newsletter.is_empty() { self.mock("Iscriviti alla newsletter", "Il modulo vero compare quando la newsletter è attiva (pannello › Newsletter).") } else { self.cx.newsletter.clone() },
            "ad" => {
                let code = match s(w, "slot") { "ad_top" | "ad_bottom" | "ad_inarticle" => opt(self.cx.st, s(w, "slot"), "").to_string(), _ => String::new() };
                let code = crate::site::consent_gate(self.cx.st, &code, "ads"); // anche qui, solo dopo il consenso
                // Nell'anteprima dell'editor il codice pubblicitario non si carica mai (girerebbe sul dominio del pannello).
                let code = if self.cx.editing && !code.is_empty() { return_placeholder() } else { code };
                if code.is_empty() { self.empty("Spazio pubblicitario: il codice si imposta in Impostazioni > Pubblicità") } else { format!("<div class=\"ad\" data-pagefind-ignore>{code}</div>") }
            }
            "html" => s(w, "code").to_string(), // HTML libero: solo gli amministratori possono salvare questa pagina
            "logo" => {
                self.css.resp(&format!("{sel} .pb-logo"), w, "h", "--h", "px");
                let st = self.cx.st;
                let logo = opt(st, "logo", "");
                let inner = if s(w, "show") != "name" && !logo.is_empty() { format!("<img src=\"{}\" alt=\"{}\">", esc(logo), esc(opt(st, "site_name", ""))) } else { esc(opt(st, "site_name", "")) };
                let tag = if on(w, "tagline") && !opt(st, "description", "").is_empty() { format!("<p class=\"pb-tag\">{}</p>", esc(opt(st, "description", ""))) } else { String::new() };
                format!("<a class=\"pb-logo\" href=\"{}/\">{inner}</a>{tag}", esc(site::base(st)))
            }
            "menu" => {
                let links: Vec<(String, String)> = if s(w, "source") == "custom" {
                    s(w, "items").lines().filter_map(|l| { let (a, b) = l.split_once('|')?; Some((a.trim().to_string(), b.trim().to_string())) }).collect()
                } else if s(w, "source") == "cats" { self.cx.cats.clone() }
                else { site::menu(self.cx.st).iter().map(|m| (m["label"].as_str().unwrap_or("").to_string(), m["url"].as_str().unwrap_or("").to_string())).collect() };
                self.css.resp(&format!("{sel} .pb-menu a"), w, "size", "font-size", "px");
                let a: String = links.iter().map(|(l, u)| format!("<a href=\"{}\">{}</a>", esc(u), esc(l))).collect();
                if a.is_empty() { self.empty("Il menu è vuoto: scrivi le voci qui a destra, oppure in Impostazioni > Menu") } else {
                    let up = if on(w, "upper") { " pb-up" } else { "" };
                    // Sul telefono il menu diventa un pulsante «Menu» che si apre (solo HTML: <details>, niente JavaScript).
                    let burger = if on(w, "no_burger") { String::new() } else { format!("<details class=\"pb-burger\"><summary aria-label=\"Apri il menu\">☰ Menu</summary><nav aria-label=\"Sezioni\">{a}</nav></details>") };
                    format!("<nav class=\"pb-menu{up}{}\" aria-label=\"Sezioni\">{a}</nav>{burger}", if on(w, "no_burger") { "" } else { " pb-collapse" })
                }
            }
            "search" => format!("<a class=\"pb-search\" href=\"{}/cerca/\"><svg width=\"16\" height=\"16\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2.6\" aria-hidden=\"true\"><circle cx=\"11\" cy=\"11\" r=\"7\"/><path d=\"m20 20-3.5-3.5\"/></svg>{}</a>",
                esc(site::base(self.cx.st)), esc(if s(w, "label").is_empty() { "Cerca" } else { s(w, "label") })),
            "social" => {
                let list = site::social(self.cx.st);
                if list.is_empty() { self.empty("Nessun profilo social: aggiungili in Impostazioni > Piè di pagina") } else {
                    let brand = s(w, "style") != "plain";
                    // colori dei social con testo leggibile (contrasto ≥ 4,5:1)
                    let col = |n: &str| match n { "Facebook" => Some(("#0866FF", "#fff")), "Instagram" => Some(("#C13584", "#fff")), "X" | "TikTok" | "Threads" => Some(("#000", "#fff")), "YouTube" => Some(("#CC0000", "#fff")), "LinkedIn" => Some(("#0A66C2", "#fff")), "Telegram" => Some(("#26A5E4", "#0b1638")), "WhatsApp" => Some(("#25D366", "#0b1638")), _ => None };
                    format!("{}<nav class=\"pb-soc\" aria-label=\"Seguici\">{}</nav>", self.title(w), list.iter().map(|x| {
                        let n = x["name"].as_str().unwrap_or("");
                        let st = if brand { col(n).map(|(c, t)| format!(" style=\"--c:{c};--t:{t}\"")).unwrap_or_default() } else { String::new() };
                        format!("<a href=\"{}\" rel=\"me noopener\" target=\"_blank\"{st}>{}</a>", esc(x["url"].as_str().unwrap_or("")), esc(n))
                    }).collect::<String>())
                }
            }
            "categories" => {
                if self.cx.cats.is_empty() { self.empty("Non ci sono ancora sezioni") } else {
                    format!("{}<ul class=\"pb-list\">{}</ul>", self.title(w), self.cx.cats.iter().map(|(n, u)| format!("<li><a href=\"{}\">{}</a></li>", esc(u), esc(n))).collect::<String>())
                }
            }
            "tabs" | "accordion" => {
                // Una voce per riga: «Titolo | testo» (nel testo, «//» va a capo).
                let items: Vec<(String, String)> = s(w, "items").lines().filter_map(|l| { let (a, b) = l.split_once('|')?; Some((a.trim().to_string(), b.trim().to_string())) }).take(12).collect();
                let para = |t: &str| t.split("//").map(|x| format!("<p>{}</p>", esc(x.trim()))).collect::<String>();
                if items.is_empty() { self.empty("Scrivi le voci qui a destra, una per riga: Titolo | testo") }
                else if w["type"] == "accordion" {
                    format!("{}<div class=\"pb-acc\">{}</div>", self.title(w), items.iter().enumerate().map(|(i, (a, b))| format!("<details{}><summary>{}</summary><div>{}</div></details>", if i == 0 && on(w, "first_open") { " open" } else { "" }, esc(a), para(b))).collect::<String>())
                } else {
                    // Schede solo con CSS: pulsanti di scelta nascosti e le etichette come linguette (si usano anche con le frecce della tastiera).
                    let g = id(w);
                    for i in 0..items.len() {
                        self.css.d += &format!("#t{g}-{i}:checked~.pb-panels>.pb-panel:nth-child({}){{display:block}}#t{g}-{i}:checked+label{{border-bottom-color:var(--accent,#1e6bff)}}", i + 1);
                    }
                    let heads: String = items.iter().enumerate().map(|(i, (a, _))| format!("<input type=\"radio\" name=\"t{g}\" id=\"t{g}-{i}\"{}><label for=\"t{g}-{i}\">{}</label>", if i == 0 { " checked" } else { "" }, esc(a))).collect();
                    format!("{}<div class=\"pb-tabs\">{heads}<div class=\"pb-panels\">{}</div></div>", self.title(w), items.iter().map(|(_, b)| format!("<div class=\"pb-panel\">{}</div>", para(b))).collect::<String>())
                }
            }
            "carousel" => {
                let posts = self.pick(w, 8, false);
                if posts.is_empty() { self.skel(n(w, "cols", 3.0) as usize, "Qui compariranno gli articoli: in questa scelta non ce ne sono ancora.") } else {
                    format!("{}<div class=\"pb-car\" tabindex=\"0\" role=\"region\" aria-label=\"{}\">{}</div>", self.title(w), esc(if s(w, "title").is_empty() { "Articoli" } else { s(w, "title") }), posts.iter().map(|p| self.card(p, w, "300px")).collect::<String>())
                }
            }
            "gallery" => {
                self.css.resp(&format!("{sel} .pb-gal"), w, "cols", "--cols", "");
                let imgs: Vec<&str> = s(w, "images").lines().map(str::trim).filter(|l| l.starts_with("/media/")).take(40).collect();
                if imgs.is_empty() { self.empty("Aggiungi le foto dalla libreria qui a destra") } else {
                    format!("{}<div class=\"pb-gal\" style=\"--cols:{}\">{}</div>", self.title(w), n(w, "cols", 3.0).clamp(1.0, 6.0), imgs.iter().map(|u| format!("<a href=\"{}\"><img {} sizes=\"(min-width: 768px) 33vw, 50vw\" alt=\"\" loading=\"lazy\"></a>", esc(u), crate::media::img_attrs(u, u))).collect::<String>())
                }
            }
            "counter" => {
                self.css.resp(&format!("{sel} .pb-num"), w, "size", "font-size", "px");
                if let Some(c) = color(s(w, "num_color")) { self.css.d += &format!("{sel} .pb-num{{color:{c}}}") }
                format!("<div class=\"pb-num\">{}{}{}</div>{}", esc(s(w, "prefix")), esc(if s(w, "number").is_empty() { "0" } else { s(w, "number") }), esc(s(w, "suffix")),
                    if s(w, "label").is_empty() { String::new() } else { format!("<p class=\"pb-num-l\">{}</p>", esc(s(w, "label"))) })
            }
            "quote" => {
                let photo = if s(w, "photo").starts_with("/media/") { format!("<img {} sizes=\"44px\" alt=\"\" loading=\"lazy\">", crate::media::img_attrs(s(w, "photo"), s(w, "photo"))) } else { String::new() };
                let who = if s(w, "author").is_empty() { String::new() } else { format!("<figcaption>{photo}<span>{}{}</span></figcaption>", esc(s(w, "author")), if s(w, "role").is_empty() { String::new() } else { format!("<small>{}</small>", esc(s(w, "role"))) }) };
                format!("<figure class=\"pb-quote\"><blockquote{}>{}</blockquote>{who}</figure>", self.ed("text", "para"), esc(if s(w, "text").is_empty() { "La citazione" } else { s(w, "text") }))
            }
            "card" => {
                let img = if s(w, "src").starts_with("/media/") { format!("<img {} sizes=\"(min-width: 768px) 400px, 100vw\" alt=\"{}\" loading=\"lazy\">", crate::media::img_attrs(s(w, "src"), s(w, "src")), esc(s(w, "alt"))) } else { String::new() };
                let btn = if s(w, "label").is_empty() { String::new() } else { format!("<a class=\"pb-btn\" href=\"{}\"{}>{}</a>", esc(if s(w, "url").is_empty() { "#" } else { s(w, "url") }), self.ed("label", ""), esc(s(w, "label"))) };
                format!("<div class=\"pb-card\">{img}<div><h3{}>{}</h3>{}{btn}</div></div>", self.ed("heading", ""), esc(if s(w, "heading").is_empty() { "Titolo della scheda" } else { s(w, "heading") }), if s(w, "text").is_empty() { String::new() } else { format!("<p{}>{}</p>", self.ed("text", "para"), esc(s(w, "text"))) })
            }
            "copyright" => {
                let year = jiff::Timestamp::now().strftime("%Y").to_string();
                let legal: String = opt(self.cx.st, "footer", "").lines().filter(|l| !l.trim().is_empty()).map(|l| format!("<p>{}</p>", esc(l.trim()))).collect();
                let extra = if s(w, "text").is_empty() { String::new() } else { format!(" {}", esc(s(w, "text"))) };
                format!("<div class=\"pb-copy\"><p>© {year} {}.{extra}</p>{}</div>", esc(opt(self.cx.st, "site_name", "")), if on(w, "no_legal") { String::new() } else { legal })
            }
            _ => String::new(),
        };
        let body = match self.ghost.take() { Some(note) if self.cx.editing => format!("<div class=\"pb-ghost\"><p class=\"pb-note\">{note}</p>{body}</div>"), _ => body };
        let edit = if self.cx.editing { format!(" data-pb=\"widget\" data-pb-id=\"{}\" data-pb-type=\"{}\" draggable=\"true\"", id(w), esc(w["type"].as_str().unwrap_or(""))) } else { String::new() };
        format!("<div class=\"pb-w\" id=\"pb-{}\"{edit}>{body}</div>", id(w))
    }
}

fn return_placeholder() -> String { "<p class=\"pb-empty\">Spazio pubblicitario: sul sito qui compare l'annuncio (nell'anteprima non si carica)</p>".into() }

/// Trasforma la struttura della pagina in HTML (con il suo CSS). `editing`: segni per l'editor, messaggi nei blocchi vuoti.
pub fn render(doc: &Value, cx: &Ctx, root: &str) -> String {
    let mut r = Render { cx, css: Css::default(), shown: HashSet::new(), dedupe: doc["no_dupes"].as_bool().unwrap_or(true), ghost: None };
    let mut html = String::new();
    for sec in doc["sections"].as_array().into_iter().flatten().take(60) {
        let sid = id(sec);
        let sel = format!("#pb-{sid}");
        r.css.box_(&sel, sec);
        r.css.resp(&sel, sec, "pad_y", "padding-top", "px");
        r.css.resp(&sel, sec, "pad_y", "padding-bottom", "px");
        if let Some(img) = Some(s(sec, "bg_img")).filter(|x| x.starts_with("/media/")) { r.css.d += &format!("{sel}{{background-image:url(\"{}\");background-size:cover;background-position:center}}", img.replace('"', "")) }
        let max = n(sec, "max", 1200.0).clamp(480.0, 2400.0);
        let gap = n(sec, "gap", 24.0).clamp(0.0, 120.0);
        if on(sec, "stack_t") { r.css.t += &format!("{sel} .pb-in{{flex-direction:column}}{sel} .pb-c{{width:100%}}") }
        let mut cols = String::new();
        for col in sec["columns"].as_array().into_iter().flatten().take(6) {
            let cid = id(col);
            let csel = format!("#pb-{cid}");
            r.css.box_(&csel, col);
            let grow = n(col, "width", 0.0);
            let flex = if grow > 0.0 { format!(" style=\"flex:0 0 calc({grow}% - {gap}px * {:.3})\"", 1.0 - grow / 100.0) } else { String::new() };
            let widgets: String = col["widgets"].as_array().into_iter().flatten().take(40).map(|w| r.widget(w)).collect();
            let widgets = if widgets.is_empty() && cx.editing { "<div class=\"pb-drop\">Trascina qui un widget</div>".to_string() } else { widgets };
            let edit = if cx.editing { format!(" data-pb=\"col\" data-pb-id=\"{cid}\"") } else { String::new() };
            cols += &format!("<div class=\"pb-c\" id=\"pb-{cid}\"{flex}{edit}>{widgets}</div>");
        }
        let edit = if cx.editing { format!(" data-pb=\"section\" data-pb-id=\"{sid}\"") } else { String::new() };
        html += &format!("<section class=\"pb-s{}\" id=\"pb-{sid}\"{edit}><div class=\"pb-in\" style=\"--pb-max:{max}px;--pb-gap:{gap}px\">{cols}</div></section>", if on(sec, "full") { " pb-full" } else { "" });
    }
    // Nella home, un titolo principale per Google e per i lettori di schermo.
    let h1 = if cx.name == "home" { format!("<h1 class=\"sr\">{}</h1>", esc(opt(cx.st, "site_name", ""))) } else { String::new() };
    let tag = match cx.name { "header" => "header", "footer" => "footer", _ => "div" };
    format!("<{tag} class=\"pb pb-{}\" id=\"{root}\"><style>{BASE}{}</style>{h1}{html}</{tag}>", cx.name, r.css.out())
}

/// Modelli di partenza per la home.
pub fn template(name: &str, page: &str) -> Value {
    let w = |t: &str, id: &str, settings: Value| json!({"id": id, "type": t, "settings": settings});
    let sec = |id: &str, cols: Vec<(f64, Vec<Value>)>, settings: Value| json!({"id": id, "settings": settings, "columns": cols.into_iter().enumerate().map(|(i, (width, widgets))| json!({"id": format!("{id}c{i}"), "settings": {"width": width}, "widgets": widgets})).collect::<Vec<_>>()});
    let page = if page.starts_with("page:") { "page" } else { page };
    match (page, name) {
        ("page", "vuoto") => json!({"no_dupes": true, "sections": [sec("ps1", vec![(0.0, vec![w("heading", "pw1", json!({"tag": "h1", "text": "Titolo della pagina"}))])], json!({}))]}),
        ("page", _) => json!({"no_dupes": true, "sections": [
            sec("ps1", vec![(0.0, vec![w("heading", "pw1", json!({"tag": "h1", "text": "Titolo della pagina", "size": 44, "size_m": 32}))])], json!({"pad_y": 40})),
            sec("ps2", vec![(58.0, vec![w("text", "pw2", json!({"text": "Racconta qui chi siete: la storia del giornale, la redazione, come contattarvi.\n\nUna riga vuota separa i paragrafi."}))]), (42.0, vec![w("image", "pw3", json!({"radius": 12}))])], json!({})),
            sec("ps3", vec![(33.0, vec![w("counter", "pw4", json!({"number": "2015", "label": "anno di fondazione"}))]), (33.0, vec![w("counter", "pw5", json!({"number": "12", "label": "giornalisti in redazione"}))]), (34.0, vec![w("counter", "pw6", json!({"number": "40.000", "label": "lettori ogni giorno"}))])], json!({"pad_y": 30})),
            sec("ps4", vec![(0.0, vec![w("accordion", "pw7", json!({"title": "Domande frequenti", "items": "Come posso segnalare una notizia? | Scrivi a redazione@… oppure usa i contatti qui sotto.\nCome posso fare pubblicità? | Contatta l'ufficio commerciale.", "first_open": true}))])], json!({})),
        ]}),
        ("header", "centrata") => json!({"sections": [
            sec("hs1", vec![(0.0, vec![w("logo", "hw1", json!({"tagline": true, "align": "center", "h": 56, "h_m": 40}))])], json!({"pad_y": 18})),
            sec("hs2", vec![(0.0, vec![w("menu", "hw2", json!({"upper": true, "align": "center"}))])], json!({"pad_y": 6, "bg": "#0b1638", "color": "#ffffff"})),
        ]}),
        ("header", _) => json!({"sections": [
            sec("hs1", vec![(66.0, vec![w("logo", "hw1", json!({"tagline": true, "h": 48, "h_m": 36}))]), (34.0, vec![w("search", "hw2", json!({"align": "right"}))])], json!({"pad_y": 16})),
            sec("hs2", vec![(0.0, vec![w("menu", "hw3", json!({"upper": true}))])], json!({"pad_y": 4})),
        ]}),
        ("footer", _) => json!({"sections": [
            sec("fs1", vec![(40.0, vec![w("logo", "fw1", json!({"show": "name", "tagline": true})), w("social", "fw2", json!({"mt": 14}))]), (30.0, vec![w("categories", "fw3", json!({"title": "Sezioni"}))]), (30.0, vec![w("newsletter", "fw4", json!({}))])], json!({"pad_y": 36, "bg": "#0b1638", "color": "#ffffff"})),
            sec("fs2", vec![(0.0, vec![w("copyright", "fw5", json!({}))])], json!({"pad_y": 14, "bg": "#070d26", "color": "#cfd6ea"})),
        ]}),
        (_, "rivista") => json!({"no_dupes": true, "sections": [
            sec("s1", vec![(0.0, vec![w("hero", "w1", json!({"layout": "overlay"}))])], json!({"pad_y": 0, "full": false})),
            sec("s2", vec![(0.0, vec![w("posts_grid", "w2", json!({"title": "In primo piano", "count": 3, "cols": 3, "show_desc": true}))])], json!({})),
            sec("s3", vec![(0.0, vec![w("posts_grid", "w3", json!({"title": "Le altre notizie", "count": 8, "cols": 4, "cols_t": 2, "cols_m": 1}))])], json!({})),
            sec("s4", vec![(50.0, vec![w("most_read", "w4", json!({"title": "I più letti", "count": 5}))]), (50.0, vec![w("newsletter", "w5", json!({}))])], json!({})),
        ]}),
        (_, "vuoto") => json!({"no_dupes": true, "sections": [sec("s1", vec![(0.0, vec![])], json!({}))]}),
        _ => json!({"no_dupes": true, "sections": [
            sec("s1", vec![(66.0, vec![w("hero", "w1", json!({"mode": "featured"}))]), (34.0, vec![w("latest", "w2", json!({"title": "Ultime notizie", "count": 8}))])], json!({})),
            sec("s2", vec![(0.0, vec![w("posts_grid", "w3", json!({"title": "In evidenza", "count": 6, "cols": 3, "show_desc": true}))])], json!({})),
            sec("s3", vec![(66.0, vec![w("posts_grid", "w4", json!({"title": "Altre notizie", "count": 6, "cols": 2}))]), (34.0, vec![w("most_read", "w5", json!({"title": "I più letti", "count": 5})), w("newsletter", "w6", json!({}))])], json!({})),
        ]}),
    }
}
