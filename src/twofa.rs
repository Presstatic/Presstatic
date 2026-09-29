//! Verifica in due passaggi con le app di autenticazione (Google Authenticator, Microsoft Authenticator, 1Password,
//! Aegis…): codici TOTP di 6 cifre che cambiano ogni 30 secondi (RFC 6238), più 10 codici di recupero usa e getta.
use ring::{digest, hmac, rand::{SecureRandom, SystemRandom}};

const B32: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32(data: &[u8]) -> String {
    let (mut out, mut buf, mut bits) = (String::new(), 0u32, 0);
    for &b in data {
        buf = (buf << 8) | b as u32; bits += 8;
        while bits >= 5 { bits -= 5; out.push(B32[((buf >> bits) & 31) as usize] as char); }
    }
    if bits > 0 { out.push(B32[((buf << (5 - bits)) & 31) as usize] as char); }
    out
}

fn unbase32(s: &str) -> Option<Vec<u8>> {
    let (mut out, mut buf, mut bits) = (vec![], 0u32, 0);
    for c in s.chars().filter(|c| !c.is_whitespace() && *c != '=') {
        let v = B32.iter().position(|&x| x as char == c.to_ascii_uppercase())? as u32;
        buf = (buf << 5) | v; bits += 5;
        if bits >= 8 { bits -= 8; out.push((buf >> bits) as u8); }
    }
    Some(out)
}

fn random(n: usize) -> Vec<u8> { let mut b = vec![0u8; n]; SystemRandom::new().fill(&mut b).expect("generatore casuale"); b }

/// Chiave segreta nuova (160 bit), in base32 come la vogliono le app.
pub fn new_secret() -> String { base32(&random(20)) }

fn code_at(secret: &[u8], step: i64) -> u32 {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let h = hmac::sign(&key, &step.to_be_bytes());
    let h = h.as_ref();
    let o = (h[19] & 0x0f) as usize;
    (u32::from_be_bytes([h[o] & 0x7f, h[o + 1], h[o + 2], h[o + 3]])) % 1_000_000
}

/// Controlla un codice di 6 cifre (accetta il passo di 30 secondi prima e dopo, per gli orologi un po' sfasati).
/// Restituisce il passo usato, che va salvato: lo stesso codice non vale una seconda volta.
pub fn verify(secret_b32: &str, code: &str, last_step: i64, now: i64) -> Option<i64> {
    let code: String = code.chars().filter(char::is_ascii_digit).collect();
    if code.len() != 6 { return None }
    let code: u32 = code.parse().ok()?;
    let secret = unbase32(secret_b32)?;
    let step = now / 30;
    [step - 1, step, step + 1].into_iter().filter(|&s| s > last_step).find(|&s| code_at(&secret, s) == code)
}

pub fn otpauth(issuer: &str, account: &str, secret: &str) -> String {
    let e = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    format!("otpauth://totp/{}:{}?secret={secret}&issuer={}&algorithm=SHA1&digits=6&period=30", e(issuer), e(account), e(issuer))
}

/// Codice QR in SVG da mostrare nel pannello (nessun servizio esterno: la chiave non esce dal server).
pub fn qr_svg(data: &str) -> String {
    qrcode::QrCode::new(data.as_bytes()).map(|c| c.render::<qrcode::render::svg::Color>().min_dimensions(220, 220).quiet_zone(true)
        .dark_color(qrcode::render::svg::Color("#0b1638")).light_color(qrcode::render::svg::Color("#ffffff")).build()).unwrap_or_default()
}

/// 10 codici di recupero nella forma «abcd-efgh»: servono se il telefono si perde. Si mostrano una volta sola.
pub fn recovery_codes() -> Vec<String> {
    const A: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789"; // senza lettere e cifre che si confondono (l, 1, o, 0)
    (0..10).map(|_| { let r = random(8); let c: String = r.iter().map(|b| A[*b as usize % A.len()] as char).collect(); format!("{}-{}", &c[..4], &c[4..]) }).collect()
}

/// Impronta di un codice di recupero o di un link di recupero: nel database non si salva mai il codice vero.
pub fn fingerprint(code: &str) -> String {
    let norm: String = code.trim().to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    digest::digest(&digest::SHA256, norm.as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Token casuale per i link di recupero della password.
pub fn token() -> String { random(24).iter().map(|b| format!("{b:02x}")).collect() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc6238() {
        // Vettore di prova della RFC 6238 (SHA1): segreto "12345678901234567890", istante 59 → 94287082 (8 cifre) → 287082
        let s = base32(b"12345678901234567890");
        assert_eq!(code_at(&unbase32(&s).unwrap(), 59 / 30), 287082);
        assert_eq!(verify(&s, "287082", 0, 59), Some(1));
        assert_eq!(verify(&s, "287082", 1, 59), None); // già usato
    }
}
