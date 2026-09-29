// Scrittura assistita: dalle fonti di una notizia a una bozza già formattata e ottimizzata per la SEO,
// con OpenAI o Anthropic, più l'immagine in evidenza generata con OpenAI.
use crate::{opt, s, site, App, R};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};
use std::{collections::HashMap, net::{IpAddr, SocketAddr, ToSocketAddrs}, time::Duration};

/// "fornitore:modello". L'elenco si può allargare dalle Integrazioni con un modello qualsiasi.
pub const TEXT_MODELS: &[(&str, &str)] = &[
    ("anthropic:claude-opus-5-5", "Claude Opus 5.5 (Anthropic): la qualità più alta"),
    ("anthropic:claude-fable-5-1", "Claude Fable 5.1 (Anthropic)"),
    ("anthropic:claude-sonnet-5", "Claude Sonnet 5 (Anthropic): equilibrato"),
    ("anthropic:claude-haiku-4-5-20251001", "Claude Haiku 4.5 (Anthropic): economico e veloce"),
    ("openai:gpt-6-astra", "GPT-6 Astra (OpenAI): la qualità più alta"),
    ("openai:gpt-6-sol", "GPT-6 Sol (OpenAI): equilibrato"),
    ("openai:gpt-6-luna", "GPT-6 Luna (OpenAI): economico e veloce"),
    ("openai:gpt-5.6-terra", "GPT-5.6 Terra (OpenAI)"),
];
pub const IMAGE_MODELS: &[(&str, &str)] = &[("gpt-image-2", "GPT Image 2 (OpenAI)")];

fn base(env: &str, default: &str) -> String { std::env::var(env).unwrap_or_else(|_| default.into()) }

/// Modelli utilizzabili con le chiavi inserite, più quelli aggiunti a mano.
pub fn models(st: &crate::Settings) -> Vec<(String, String)> {
    let has = |p: &str| !opt(st, if p == "openai" { "openai_key" } else { "anthropic_key" }, "").is_empty();
    let mut v: Vec<(String, String)> = TEXT_MODELS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    for l in opt(st, "ai_text_custom", "").lines().map(str::trim).filter(|l| l.contains(':')) { v.push((l.to_string(), format!("{l} (aggiunto a mano)"))) }
    v.into_iter().filter(|(id, _)| id.split(':').next().is_some_and(has)).collect()
}

fn http_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(c, r) => {
            let v: Value = r.into_json().unwrap_or_default();
            let m = v["error"]["message"].as_str().unwrap_or("");
            match c { 401 => "chiave API non valida".into(), 404 => format!("modello non disponibile per questa chiave ({m})"), 429 => "limite di richieste o credito esaurito".into(), _ => format!("errore {c}: {m}") }
        }
        e => e.to_string(),
    }
}

/// Chiede un testo al modello scelto.
fn ask(st: &crate::Settings, model: &str, system: &str, user: &str) -> R<String> {
    let (provider, name) = model.split_once(':').ok_or("modello non valido")?;
    let timeout = Duration::from_secs(300);
    if provider == "anthropic" {
        let v: Value = crate::api_agent(300).post(&format!("{}/v1/messages", base("PRESSTATIC_ANTHROPIC_API", "https://api.anthropic.com")))
            .set("x-api-key", opt(st, "anthropic_key", "")).set("anthropic-version", "2023-06-01").timeout(timeout)
            .send_json(json!({"model": name, "max_tokens": 8000, "system": system, "messages": [{"role": "user", "content": user}]}))
            .map_err(http_error)?.into_json().map_err(s)?;
        Ok(v["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).collect())
    } else {
        let v: Value = crate::api_agent(300).post(&format!("{}/v1/chat/completions", base("PRESSTATIC_OPENAI_API", "https://api.openai.com")))
            .set("Authorization", &format!("Bearer {}", opt(st, "openai_key", ""))).timeout(timeout)
            .send_json(json!({"model": name, "response_format": {"type": "json_object"},
                "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}]}))
            .map_err(http_error)?.into_json().map_err(s)?;
        Ok(v["choices"][0]["message"]["content"].as_str().unwrap_or_default().to_string())
    }
}

/// Immagine in evidenza, in JPEG orizzontale.
fn image(st: &crate::Settings, prompt: &str) -> R<Vec<u8>> {
    let v: Value = crate::api_agent(300).post(&format!("{}/v1/images/generations", base("PRESSTATIC_OPENAI_API", "https://api.openai.com")))
        .set("Authorization", &format!("Bearer {}", opt(st, "openai_key", ""))).timeout(Duration::from_secs(300))
        .send_json(json!({"model": opt(st, "ai_image_model", "gpt-image-2"), "prompt": prompt, "size": "1536x1024", "output_format": "jpeg", "n": 1}))
        .map_err(http_error)?.into_json().map_err(s)?;
    B64.decode(v["data"][0]["b64_json"].as_str().ok_or("nessuna immagine ricevuta")?).map_err(s)
}

/// Testo leggibile di una pagina web: solo l'articolo se c'è, senza script, menu e piè di pagina.
fn text_of(html: &str) -> String {
    let low = html.to_ascii_lowercase();
    let (a, b) = match (low.find("<article"), low.rfind("</article>")) { (Some(a), Some(b)) if b > a => (a, b), _ => (0, html.len()) };
    let (html, low) = (&html[a..b], &low[a..b]);
    let (mut out, mut i) = (String::new(), 0);
    'outer: while i < html.len() {
        if html.as_bytes()[i] == b'<' {
            for tag in ["script", "style", "noscript", "svg", "nav", "footer", "form", "aside"] {
                if low[i + 1..].starts_with(tag) {
                    // Un tag mai chiuso chiude la lettura: così la scansione resta lineare anche su pagine costruite apposta.
                    match low[i..].find(&format!("</{tag}>")) { Some(e) => { i += e + tag.len() + 3; continue 'outer } None => break 'outer }
                }
            }
            i += low[i..].find('>').map_or(html.len() - i, |e| e + 1);
            out.push(' ');
        } else {
            let c = html[i..].chars().next().unwrap_or(' ');
            out.push(c);
            i += c.len_utf8();
        }
    }
    let out = out.replace("&nbsp;", " ").replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&rsquo;", "’").replace("&lsquo;", "‘").replace("&ldquo;", "“").replace("&rdquo;", "”").replace("&lt;", "<").replace("&gt;", ">");
    out.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(12000).collect()
}

/// Indirizzo raggiungibile da Internet: niente server stesso, rete interna, dati del provider (169.254.x.x) o indirizzi riservati.
pub(crate) fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_loopback() || v.is_private() || v.is_link_local() || v.is_unspecified() || v.is_broadcast() || v.is_multicast()
                || v.is_documentation() || o[0] == 0 || o[0] >= 240 || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) || (o[0] == 198 && (18..20).contains(&o[1])))
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() { return public(IpAddr::V4(v4)) }
            let g = v.segments();
            !(v.is_loopback() || v.is_unspecified() || v.is_multicast() || (g[0] & 0xfe00) == 0xfc00 || (g[0] & 0xffc0) == 0xfe80
                || (g[0] == 0x64 && g[1] == 0xff9b) || (g[0] == 0x2001 && g[1] == 0xdb8))
        }
    }
}

/// Scarica una fonte. Il controllo sta nel risolutore DNS: vale per ogni connessione, compresi i reindirizzamenti,
/// e usa proprio gli indirizzi controllati (niente trucchi con un DNS che cambia risposta tra controllo e connessione).
fn fetch(url: &str) -> R<String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).redirects(5)
        .resolver(|netloc: &str| -> std::io::Result<Vec<SocketAddr>> {
            let ok: Vec<SocketAddr> = netloc.to_socket_addrs()?.filter(|a| public(a.ip()) && matches!(a.port(), 80 | 443)).collect();
            if ok.is_empty() { Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "indirizzo non consentito")) } else { Ok(ok) }
        }).build();
    let r = agent.get(url).set("User-Agent", "Mozilla/5.0 (compatible; Presstatic/1.0)").call().map_err(|e| {
        if e.to_string().contains("non consentito") { format!("{url}: si possono usare solo indirizzi pubblici di siti web") } else { e.to_string() }
    })?;
    let mut body = String::new();
    std::io::Read::read_to_string(&mut std::io::Read::take(r.into_reader(), 3_000_000), &mut body).map_err(s)?;
    Ok(text_of(&body))
}

/// Scrive la bozza e la salva: restituisce l'id dell'articolo e il messaggio per il redattore.
pub fn write(app: &App, me: &site::User, f: &HashMap<String, String>) -> R<(i64, String)> {
    let st = app.settings();
    let v = |k: &str| f.get(k).map(|x| x.trim().to_string()).unwrap_or_default();
    let model = Some(v("model")).filter(|m| models(&st).iter().any(|x| &x.0 == m)).ok_or("scegli un modello tra quelli disponibili")?;
    let topic = v("topic");
    let mut sources = String::new();
    for (i, url) in v("sources").lines().map(str::trim).filter(|l| l.starts_with("http")).take(6).enumerate() {
        sources += &format!("\n\n[{}] {url}\n{}", i + 1, fetch(url)?);
    }
    if !v("notes").is_empty() { sources += &format!("\n\n[Testo fornito dalla redazione]\n{}", v("notes")) }
    if topic.is_empty() && sources.is_empty() { return Err("scrivi di cosa parla la notizia o indica almeno una fonte".into()) }
    let words = match v("length").as_str() { "short" => 400, "long" => 1200, _ => 700 };
    let cats: Vec<String> = {
        let db = app.db.lock().unwrap_or_else(|e| e.into_inner());
        let mut q = db.prepare("SELECT DISTINCT category FROM posts WHERE category <> ''").map_err(s)?;
        let c = q.query_map([], |r| r.get(0)).map_err(s)?.filter_map(Result::ok).collect();
        c
    };
    let system = format!("Sei un giornalista esperto e il responsabile SEO di «{}», una testata online italiana. Scrivi articoli originali, accurati e chiari, in italiano.", opt(&st, "site_name", "una testata"));
    let user = format!(r#"Scrivi un articolo di circa {words} parole sulla notizia descritta qui sotto, usando solo i fatti presenti nelle fonti.

Regole:
- Non inventare fatti, numeri, date, nomi o dichiarazioni. Usa le virgolette solo per frasi che compaiono identiche nelle fonti, sempre attribuite a chi le ha dette. Se le fonti non bastano, scrivi un articolo più breve.
- Riscrivi con parole tue, senza copiare frasi dalle fonti.
- Nelle prime due frasi rispondi a chi, cosa, quando, dove e perché.
- Paragrafi brevi, due o tre titoletti <h2> descrittivi, un elenco puntato solo se aiuta davvero.
- SEO: la parola chiave principale nel titolo, nel primo paragrafo e in un titoletto, in modo naturale. Titolo di massimo 65 caratteri, sommario tra 130 e 155 caratteri.
- Chiudi con un paragrafo «Fonti:» che contiene i link alle fonti usate.
{}
Rispondi solo con un oggetto JSON con queste chiavi:
"title", "description", "slug" (minuscole e trattini), "category" (scegli tra: {}; se nessuna va bene, proponine una breve), "tags" (da 3 a 6), "link_keywords" (da 1 a 3 espressioni per cui altri articoli dovrebbero rimandare a questo), "body" (HTML con i soli tag p, h2, h3, strong, em, ul, ol, li, blockquote, a), "image_prompt" (in inglese: illustrazione editoriale concettuale adatta alla notizia, senza testo, senza loghi, senza persone reali riconoscibili).

NOTIZIA E INDICAZIONI DELLA REDAZIONE:
{topic}

FONTI:{sources}"#,
        Some(opt(&st, "ai_style", "")).filter(|x| !x.is_empty()).map(|x| format!("- Stile della testata: {x}")).unwrap_or_default(),
        if cats.is_empty() { "nessuna categoria esistente".to_string() } else { cats.join(", ") });
    let out = ask(&st, &model, &system, &user)?;
    let json_part = out.find('{').zip(out.rfind('}')).map(|(a, b)| &out[a..=b]).ok_or("il modello non ha risposto nel formato richiesto")?;
    let a: Value = serde_json::from_str(json_part).map_err(|_| "il modello non ha risposto nel formato richiesto, riprova")?;
    let body = a["body"].as_str().unwrap_or_default();
    if body.len() < 100 { return Err("il testo ricevuto è troppo breve, riprova o aggiungi fonti".into()) }
    let list = |k: &str| a[k].as_array().map(|x| x.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    // I campi generati vengono accorciati ai limiti che save_post accetta: una risposta un po' lunga
    // diventa una bozza da rifinire, non un errore che fa perdere la chiamata (già pagata).
    let cut = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    let mut fields: HashMap<String, String> = [
        ("title", cut(a["title"].as_str().unwrap_or("Bozza"), 300)), ("description", cut(a["description"].as_str().unwrap_or(""), 500)), ("body", site::sanitize(body)),
        ("slug", cut(a["slug"].as_str().unwrap_or(""), 200)), ("category", cut(a["category"].as_str().unwrap_or(""), 100)), ("status", "draft".into()), ("schema_type", "NewsArticle".into()),
    ].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    fields.insert("tags".into(), list("tags"));
    fields.insert("link_keywords".into(), list("link_keywords"));
    let mut note = String::new();
    if v("image") == "on" {
        let prompt = a["image_prompt"].as_str().unwrap_or(fields["title"].as_str()).to_string();
        match image(&st, &format!("{prompt}. Editorial illustration, no text, no logos.")).and_then(|bytes| site::save_upload(app, "illustrazione.jpg", &bytes)) {
            Ok(url) => { fields.insert("image".into(), url); }
            Err(e) => note = format!(" Immagine non generata: {e}."),
        }
    }
    let label = models(&st).into_iter().find(|m| m.0 == model).map(|m| m.1).unwrap_or(model);
    let (id, _) = site::save_post(app, me, 0, fields, None)?;
    Ok((id, format!("Bozza scritta con {label}. Prima di pubblicare controlla fatti, nomi, citazioni e fonti.{note}")))
}
