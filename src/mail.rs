//! Invio delle email con SMTP e TLS: recupero della password e avvisi di sicurezza (più avanti newsletter e commenti).
//! Funziona con qualsiasi servizio SMTP: la casella del dominio, Gmail con password per le app, Brevo, Mailgun, Amazon SES.
use crate::{opt, R, Settings};
use lettre::{
    message::{Mailbox, MultiPart},
    transport::smtp::{authentication::Credentials, client::{Tls, TlsParameters}},
    Message, SmtpTransport, Transport,
};
use std::time::Duration;

pub fn configured(st: &Settings) -> bool { !opt(st, "smtp_host", "").is_empty() && !opt(st, "smtp_from", "").is_empty() }

/// Collegamento al server SMTP, riusabile per molte email (la newsletter non riapre la connessione per ogni iscritto).
pub struct Sender { transport: SmtpTransport, from: Mailbox, host: String, port: u16 }

pub fn sender(st: &Settings) -> R<Sender> {
    if !configured(st) { return Err("l'invio delle email non è configurato: impostalo in Integrazioni, sezione Email".into()) }
    let from_addr = opt(st, "smtp_from", "").parse().map_err(|_| "l'indirizzo del mittente non è valido".to_string())?;
    let from = Mailbox::new(Some(opt(st, "smtp_from_name", opt(st, "site_name", "Presstatic")).to_string()), from_addr);
    let host = opt(st, "smtp_host", "").trim().to_string();
    let security = opt(st, "smtp_security", "starttls");
    let port: u16 = opt(st, "smtp_port", "").parse().unwrap_or(if security == "tls" { 465 } else { 587 });
    let mut b = SmtpTransport::builder_dangerous(&host).port(port).timeout(Some(Duration::from_secs(20)));
    b = match security {
        "none" => b.tls(Tls::None),
        mode => {
            let tls = TlsParameters::new(host.clone()).map_err(|e| format!("impostazioni TLS non valide: {e}"))?;
            b.tls(if mode == "tls" { Tls::Wrapper(tls) } else { Tls::Required(tls) })
        }
    };
    let user = opt(st, "smtp_user", "");
    if !user.is_empty() { b = b.credentials(Credentials::new(user.to_string(), opt(st, "smtp_pass", "").to_string())) }
    Ok(Sender { transport: b.build(), from, host, port })
}

/// Intestazioni per la disiscrizione con un clic (Gmail e gli altri mostrano «Annulla iscrizione»).
#[derive(Clone)]
struct ListUnsubscribe(String);
impl lettre::message::header::Header for ListUnsubscribe {
    fn name() -> lettre::message::header::HeaderName { lettre::message::header::HeaderName::new_from_ascii_str("List-Unsubscribe") }
    fn parse(s: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> { Ok(Self(s.to_string())) }
    fn display(&self) -> lettre::message::header::HeaderValue { lettre::message::header::HeaderValue::new(Self::name(), self.0.clone()) }
}
#[derive(Clone)]
struct ListUnsubscribePost;
impl lettre::message::header::Header for ListUnsubscribePost {
    fn name() -> lettre::message::header::HeaderName { lettre::message::header::HeaderName::new_from_ascii_str("List-Unsubscribe-Post") }
    fn parse(_: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> { Ok(Self) }
    fn display(&self) -> lettre::message::header::HeaderValue { lettre::message::header::HeaderValue::new(Self::name(), "List-Unsubscribe=One-Click".into()) }
}

impl Sender {
    /// `unsubscribe`: indirizzo per la disiscrizione con un clic (solo per la newsletter).
    pub fn send(&self, to: &str, subject: &str, text: &str, html: &str, unsubscribe: Option<&str>) -> R<()> {
        let to: Mailbox = to.parse().map_err(|_| format!("l'indirizzo {to} non è valido"))?;
        let mut b = Message::builder().from(self.from.clone()).to(to).subject(subject);
        if let Some(u) = unsubscribe { b = b.header(ListUnsubscribe(format!("<{u}>"))).header(ListUnsubscribePost); }
        let msg = b.multipart(MultiPart::alternative_plain_html(text.to_string(), html.to_string())).map_err(|e| format!("email non preparata: {e}"))?;
        self.transport.send(&msg).map_err(|e| format!("il server {}:{} non ha accettato l'email ({e})", self.host, self.port))?;
        Ok(())
    }
}

/// Come `send`, con «Rispondi a»: rispondendo si scrive a chi ha compilato il modulo, non al sito.
pub fn send_reply(st: &Settings, to: &str, reply_to: Option<&str>, subject: &str, text: &str, html: &str) -> R<()> {
    let snd = sender(st)?;
    let to_box: Mailbox = to.parse().map_err(|_| format!("l'indirizzo {to} non è valido"))?;
    let mut b = Message::builder().from(snd.from.clone()).to(to_box).subject(subject);
    if let Some(r) = reply_to.and_then(|r| r.parse::<Mailbox>().ok()) { b = b.reply_to(r); }
    let msg = b.multipart(MultiPart::alternative_plain_html(text.to_string(), html.to_string())).map_err(|e| format!("email non preparata: {e}"))?;
    snd.transport.send(&msg).map_err(|e| format!("il server {}:{} non ha accettato l'email ({e})", snd.host, snd.port))?;
    Ok(())
}

/// Invia un'email con versione in testo semplice e versione HTML.
pub fn send(st: &Settings, to: &str, subject: &str, text: &str, html: &str) -> R<()> {
    sender(st)?.send(to, subject, text, html, None)
}

/// Email con l'aspetto del pannello: titolo, testo, un pulsante e una nota finale.
pub fn layout(site: &str, title: &str, lines: &[&str], button: Option<(&str, &str)>, note: &str) -> String {
    let esc = crate::site::esc;
    let body: String = lines.iter().map(|l| format!("<p style=\"margin:0 0 14px;font-size:15px;line-height:1.6;color:#33415c\">{}</p>", esc(l))).collect();
    let btn = button.map(|(label, url)| format!(
        "<p style=\"margin:22px 0\"><a href=\"{}\" style=\"display:inline-block;padding:12px 22px;border-radius:12px;background:#1e6bff;background-image:linear-gradient(135deg,#1238c7,#1e6bff 55%,#06b6f0);color:#ffffff;font-weight:700;text-decoration:none\">{}</a></p>",
        esc(url), esc(label))).unwrap_or_default();
    format!("<!doctype html><html lang=\"it\"><body style=\"margin:0;background:#f4f6fb;font-family:Arial,Helvetica,sans-serif\">\
<div style=\"max-width:560px;margin:0 auto;padding:28px 16px\"><div style=\"background:#0a1233;border-radius:16px 16px 0 0;padding:18px 24px;color:#ffffff;font-weight:700\">{}</div>\
<div style=\"background:#ffffff;border-radius:0 0 16px 16px;padding:26px 24px;border:1px solid #e3e8f2;border-top:0\"><h1 style=\"margin:0 0 16px;font-size:21px;color:#0b1638\">{}</h1>{body}{btn}\
<p style=\"margin:18px 0 0;font-size:13px;line-height:1.5;color:#67728c\">{}</p></div></div></body></html>", esc(site), esc(title), esc(note))
}
