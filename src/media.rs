// Immagini come Ghost: ogni caricamento diventa una copia di riserva JPEG/PNG (senza dati EXIF,
// quindi senza la posizione GPS del telefono) più versioni WebP a varie larghezze per srcset.
use crate::{s, R};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::io::Cursor;

const WIDTHS: [u32; 4] = [480, 800, 1200, 1600];
const MAX: u32 = 2400;

/// Larghezze WebP generate per un'immagine larga `w` pixel (mai ingrandita).
pub fn widths(w: u32) -> Vec<u32> {
    let mut v: Vec<u32> = WIDTHS.into_iter().filter(|&x| x < w).collect();
    if w <= 1600 { v.push(w) }
    v
}

pub struct Processed { pub fallback: Vec<u8>, pub ext: &'static str, pub w: u32, pub h: u32, pub webp: Vec<(u32, Vec<u8>)> }

// Una sola immagine elaborata alla volta: più caricamenti insieme non possono esaurire la memoria del server.
static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
static WAITING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
struct Turn;
impl Drop for Turn { fn drop(&mut self) { WAITING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed); } }

/// Al massimo 4 immagini in coda: oltre, il caricamento viene rifiutato invece di occupare thread in attesa.
fn one_at_a_time() -> R<(Turn, std::sync::MutexGuard<'static, ()>)> {
    if WAITING.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 4 {
        WAITING.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        return Err("troppe immagini in elaborazione in questo momento, riprova tra qualche secondo".into());
    }
    let turn = Turn;
    Ok((turn, GATE.lock().unwrap_or_else(|e| e.into_inner())))
}

fn decode(bytes: &[u8]) -> R<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(s)?;
    // Un file piccolo può dichiarare dimensioni enormi: si rifiuta prima di allocare la memoria.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(12_000);
    limits.max_image_height = Some(12_000);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let mut dec = reader.into_decoder().map_err(s)?;
    let orientation = dec.orientation().map_err(s)?;
    let mut img = DynamicImage::from_decoder(dec).map_err(s)?;
    img.apply_orientation(orientation); // le foto del telefono restano dritte
    Ok(img)
}

fn webp(img: &DynamicImage) -> Vec<u8> {
    let rgba = img.to_rgba8();
    webp::Encoder::from_rgba(&rgba, img.width(), img.height()).encode(78.0).to_vec()
}

/// `png`: il file caricato era un PNG (grafica, loghi, schermate) e la copia di riserva resta PNG.
pub fn process(bytes: &[u8], png: bool) -> R<Processed> {
    let _turn = one_at_a_time()?;
    let mut img = decode(bytes)?;
    if img.width() > MAX { img = img.resize(MAX, u32::MAX, FilterType::CatmullRom) }
    let (w, h, alpha) = (img.width(), img.height(), png || img.color().has_alpha());
    let mut fallback = Vec::new();
    if alpha {
        img.write_to(&mut Cursor::new(&mut fallback), ImageFormat::Png).map_err(s)?;
    } else {
        JpegEncoder::new_with_quality(&mut fallback, 85).encode_image(&img.to_rgb8()).map_err(s)?;
    }
    let variants = widths(w).into_iter()
        .map(|x| (x, webp(&if x == w { img.clone() } else { img.resize(x, u32::MAX, FilterType::CatmullRom) })))
        .collect();
    Ok(Processed { fallback, ext: if alpha { "png" } else { "jpg" }, w, h, webp: variants })
}

/// Foto dell'autore: quadrata, 256 pixel (nitida fino a 128 pixel sugli schermi ad alta densità), WebP.
pub fn avatar(bytes: &[u8]) -> R<Vec<u8>> {
    let _turn = one_at_a_time()?;
    Ok(webp(&decode(bytes)?.resize_to_fill(256, 256, FilterType::CatmullRom)))
}

/// Larghezza e altezza di un'immagine caricata, lette dal nome del file (…-1200x800.jpg).
pub fn dims(local: &str) -> Option<(u32, u32)> {
    let (stem, _) = local.rsplit_once('.')?;
    let (w, h) = stem.rsplit_once('-')?.1.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// Attributi di <img> per un file caricato: src WebP, srcset, larghezza e altezza (niente salti di layout).
/// Per immagini esterne o vecchie restituisce solo src.
pub fn img_attrs(url: &str, local: &str) -> String {
    let esc = crate::site::esc;
    let parsed = local.rsplit_once('.').and_then(|(stem, ext)| {
        if !matches!(ext, "jpg" | "png") || !local.starts_with("/media/") { return None }
        let (base, dims) = stem.rsplit_once('-')?;
        let (w, h) = dims.split_once('x')?;
        Some((base, w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))
    });
    let Some((base, w, h)) = parsed else { return format!("src=\"{}\"", esc(url)) };
    let prefix = &url[..url.len() - local.len()]; // dominio, se l'indirizzo è assoluto
    let list = widths(w);
    let src = list.iter().find(|&&x| x >= 1200).or(list.last()).copied().unwrap_or(w);
    let srcset: Vec<String> = list.iter().map(|x| format!("{prefix}{base}-{x}.webp {x}w")).collect();
    format!("src=\"{}\" srcset=\"{}\" width=\"{w}\" height=\"{h}\"", esc(&format!("{prefix}{base}-{src}.webp")), esc(&srcset.join(", ")))
}

/// Ricostruisce una GIF tenendo solo i blocchi necessari a mostrarla (immagini, controllo grafico, ciclo
/// dell'animazione) e togliendo commenti ed estensioni applicative con metadati (XMP e simili).
/// Restituisce None se il file non è una GIF valida.
pub fn clean_gif(b: &[u8]) -> Option<Vec<u8>> {
    if b.len() < 13 || !(b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")) { return None }
    let mut out = b[..13].to_vec();
    let flags = b[10];
    let mut i = 13;
    if flags & 0x80 != 0 { // tavolozza globale
        let n = 3 * (1usize << ((flags & 7) + 1));
        out.extend_from_slice(b.get(i..i + n)?);
        i += n;
    }
    // Copia i sotto-blocchi (lunghezza + dati, fino al blocco di lunghezza 0) a partire da i.
    fn sub_blocks(b: &[u8], mut i: usize) -> Option<usize> {
        loop {
            let n = *b.get(i)? as usize;
            i += 1 + n;
            if n == 0 { return Some(i) }
            if i > b.len() { return None }
        }
    }
    loop {
        match *b.get(i)? {
            0x3B => { out.push(0x3B); return Some(out) } // fine del file
            0x2C => { // immagine: descrittore, eventuale tavolozza locale, dati compressi
                let d = b.get(i..i + 10)?;
                let mut j = i + 10;
                if d[9] & 0x80 != 0 { j += 3 * (1usize << ((d[9] & 7) + 1)); }
                j += 1; // dimensione minima del codice LZW
                let end = sub_blocks(b, j)?;
                out.extend_from_slice(b.get(i..end)?);
                i = end;
            }
            0x21 => { // estensione
                let label = *b.get(i + 1)?;
                let end = sub_blocks(b, i + 2)?;
                let keep = match label {
                    0xF9 | 0x01 => true, // controllo grafico (tempi e trasparenza), testo semplice
                    0xFF => b.get(i + 3..i + 14).is_some_and(|id| id == b"NETSCAPE2.0" || id == b"ANIMEXTS1.0"), // solo il ciclo dell'animazione
                    _ => false, // commenti (0xFE) e ogni altra estensione applicativa (XMP, metadati)
                };
                if keep { out.extend_from_slice(b.get(i..end)?); }
                i = end;
            }
            _ => return None,
        }
    }
}
