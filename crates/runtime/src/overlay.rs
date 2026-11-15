//! Il pannello di diagnostica (F8): fotogrammi al secondo, tempo per fotogramma, draw.
//!
//! È disegnato dal renderer sopra l'immagine finita, non nel titolo della finestra, e vuole
//! sembrare un pezzo dell'Animus: tutto il gioco è il ricordo riprodotto da una macchina, e una
//! macchina può avere un suo pannello di servizio. Niente asset del gioco: forme, caratteri e
//! l'elica animata sono descritti qui sotto in codice, e i colori sono quelli chiari e freddi
//! dell'interfaccia dell'Animus, con il rosso della desincronizzazione quando il ritmo crolla.
//!
//! Tutto è fatto di triangoli colorati (nessuna texture), in pixel della finestra.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

static ON: AtomicBool = AtomicBool::new(false);

/// Quanti fotogrammi tiene il grafico.
const HISTORY: usize = 56;

struct Stats {
    last: Option<Instant>,
    /// Millisecondi degli ultimi fotogrammi, dal più vecchio al più recente.
    times: [f32; HISTORY],
    /// Valori mostrati, aggiornati due volte al secondo perché restino leggibili.
    fps: f32,
    ms: f32,
    draws: u64,
    acc: (Instant, u32, u64),
    started: Instant,
}

static STATS: Mutex<Option<Stats>> = Mutex::new(None);

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// F8, il comando `fps` del canale di comando, o `ALTAIR_FPS=1` all'avvio.
pub fn toggle() -> bool {
    !ON.fetch_xor(true, Ordering::Relaxed)
}

pub fn init() {
    if std::env::var("ALTAIR_FPS").map(|v| v != "0").unwrap_or(false) {
        ON.store(true, Ordering::Relaxed);
    }
}

/// Un fotogramma è finito: lo chiama `Present`, con i draw che conteneva.
pub fn frame(draws: u64) {
    let now = Instant::now();
    let mut guard = STATS.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.get_or_insert_with(|| Stats { last: None, times: [0.0; HISTORY], fps: 0.0, ms: 0.0, draws: 0, acc: (now, 0, 0), started: now });
    if let Some(last) = s.last {
        s.times.rotate_left(1);
        s.times[HISTORY - 1] = (now - last).as_secs_f32() * 1000.0;
    }
    s.last = Some(now);
    s.acc.1 += 1;
    s.acc.2 += draws;
    let elapsed = (now - s.acc.0).as_secs_f32();
    if elapsed >= 0.5 {
        s.fps = s.acc.1 as f32 / elapsed;
        s.ms = elapsed * 1000.0 / s.acc.1 as f32;
        s.draws = s.acc.2 / s.acc.1 as u64;
        s.acc = (now, 0, 0);
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub color: [f32; 4],
}

/// Caratteri 5×7, una riga per stringa, dall'alto. Solo quelli che servono.
fn glyph(ch: char) -> [u8; 7] {
    match ch {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        '.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        '/' => [0b00001, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b10000],
        '-' => [0, 0, 0, 0b01110, 0, 0, 0],
        _ => [0; 7],
    }
}

/// Raccoglie i triangoli, in pixel con l'origine in alto a sinistra.
struct Draw {
    v: Vec<Vertex>,
    srgb: bool,
}

type Color = [f32; 4];

impl Draw {
    fn color(&self, c: Color) -> Color {
        // Su una superficie sRGB i colori dei vertici sono letti come lineari.
        if self.srgb {
            [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2), c[3]]
        } else {
            c
        }
    }

    fn tri(&mut self, p: [[f32; 2]; 3], c: Color) {
        let color = self.color(c);
        self.v.extend(p.iter().map(|&pos| Vertex { pos, color }));
    }

    fn quad(&mut self, p: [[f32; 2]; 4], c: Color) {
        self.tri([p[0], p[1], p[2]], c);
        self.tri([p[0], p[2], p[3]], c);
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        self.quad([[x, y], [x + w, y], [x + w, y + h], [x, y + h]], c);
    }

    /// Rombo centrato: il "pixel" dell'elica.
    fn diamond(&mut self, cx: f32, cy: f32, r: f32, c: Color) {
        self.quad([[cx, cy - r], [cx + r, cy], [cx, cy + r], [cx - r, cy]], c);
    }

    /// Testo con caratteri 5×7 ingranditi di `k`; ritorna la x dopo l'ultimo carattere.
    fn text(&mut self, mut x: f32, y: f32, k: f32, s: &str, c: Color) -> f32 {
        for ch in s.chars() {
            for (row, bits) in glyph(ch).iter().enumerate() {
                // Le file di pixel accesi diventano un solo rettangolo: meno triangoli.
                let mut col = 0;
                while col < 5 {
                    if bits >> (4 - col) & 1 == 1 {
                        let start = col;
                        while col < 5 && bits >> (4 - col) & 1 == 1 {
                            col += 1;
                        }
                        self.rect(x + start as f32 * k, y + row as f32 * k, (col - start) as f32 * k, k, c);
                    } else {
                        col += 1;
                    }
                }
            }
            x += 6.0 * k;
        }
        x - k
    }
}

fn text_width(s: &str, k: f32) -> f32 {
    s.chars().count() as f32 * 6.0 * k - k
}

/// I triangoli del pannello per una finestra di `w`×`h` pixel, già in coordinate di clip.
pub fn build(w: u32, h: u32, srgb: bool) -> Vec<Vertex> {
    let guard = STATS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = guard.as_ref() else { return Vec::new() };
    let mut d = Draw { v: Vec::with_capacity(4096), srgb };
    // Unità: un pixel a 720 righe, mai meno di uno, arrotondata al mezzo pixel perché i
    // caratteri restino netti.
    let u = ((h as f32 / 720.0) * 2.0).round().max(2.0) / 2.0;
    let (pw, ph) = (254.0 * u, 78.0 * u);
    let (px, py) = (w as f32 - pw - 22.0 * u, 20.0 * u);

    // Colori dell'Animus: bianco freddo, azzurro, e il rosso della desincronizzazione.
    let slow = s.fps > 0.0 && s.fps < 30.0;
    let ink: Color = if slow { [1.0, 0.56, 0.50, 0.96] } else { [0.93, 0.97, 1.0, 0.96] };
    let dim: Color = [0.62, 0.78, 0.92, 0.80];
    let accent: Color = if slow { [0.95, 0.30, 0.26, 0.95] } else { [0.35, 0.80, 1.0, 0.95] };
    let glass: Color = [0.03, 0.06, 0.10, 0.62];

    // Il vetro: un rettangolo con l'angolo in basso a sinistra tagliato.
    let cut = 14.0 * u;
    d.quad([[px, py], [px + pw, py], [px + pw, py + ph], [px + cut, py + ph]], glass);
    d.tri([[px, py], [px + cut, py + ph], [px, py + ph - cut]], glass);
    // Bordi sottili: sopra pieno, a destra corto, il taglio in azzurro.
    let t = u.max(1.0);
    d.rect(px, py, pw, t, dim);
    d.rect(px, py, 46.0 * u, 2.0 * t, accent);
    d.rect(px + pw - t, py, t, 18.0 * u, dim);
    d.rect(px + cut, py + ph - t, pw - cut - 40.0 * u, t, [dim[0], dim[1], dim[2], 0.35]);
    d.quad([[px, py + ph - cut], [px + t * 1.5, py + ph - cut], [px + cut + t * 1.5, py + ph], [px + cut, py + ph]], accent);

    // L'elica: due filamenti di rombi che ruotano piano, con i pioli tra l'uno e l'altro.
    let phase = s.started.elapsed().as_secs_f32() * 1.6;
    let (hx, hy, hh, hr) = (px + 24.0 * u, py + 12.0 * u, 54.0 * u, 9.0 * u);
    const RUNGS: usize = 9;
    for i in 0..RUNGS {
        let f = i as f32 / (RUNGS - 1) as f32;
        let a = phase + f * 4.4;
        let y = hy + f * hh;
        let (xa, xb) = (hx + a.sin() * hr, hx - a.sin() * hr);
        let depth = a.cos(); // > 0: il primo filamento è davanti
        if i % 2 == 0 {
            d.rect(xa.min(xb), y - t * 0.5, (xa - xb).abs(), t, [dim[0], dim[1], dim[2], 0.30]);
        }
        let (near, far) = if depth >= 0.0 { (xa, xb) } else { (xb, xa) };
        let k = depth.abs();
        d.diamond(far, y, (1.6 + 0.6 * (1.0 - k)) * u, [dim[0], dim[1], dim[2], 0.45 + 0.2 * (1.0 - k)]);
        d.diamond(near, y, (2.2 + 1.0 * k) * u, if i % 3 == 0 { accent } else { ink });
    }

    // Intestazione.
    let tx = px + 46.0 * u;
    let end = d.text(tx, py + 9.0 * u, u, "ANIMUS", dim);
    d.text(end + 6.0 * u, py + 9.0 * u, u, "/ SINCRONIA", [dim[0], dim[1], dim[2], 0.5]);

    // Il numero grande, allineato a destra su quattro cifre, e la sua unità.
    let fps = format!("{:.0}", s.fps.min(9999.0));
    let big = 4.0 * u;
    let slot = text_width("0000", big);
    let nx = tx + slot - text_width(&fps, big);
    let end = d.text(nx, py + 22.0 * u, big, &fps, ink);
    d.text(end + 5.0 * u, py + 22.0 * u + 3.0 * big, u, "FPS", accent);

    // Colonna di destra: millisecondi e draw.
    let cx = px + 168.0 * u;
    let small = 1.5 * u;
    d.text(cx, py + 23.0 * u, u, "MS", dim);
    d.text(cx + 34.0 * u, py + 21.5 * u, small, &format!("{:.1}", s.ms.min(999.9)), ink);
    d.text(cx, py + 39.0 * u, u, "DRAW", dim);
    d.text(cx + 34.0 * u, py + 37.5 * u, small, &format!("{}", s.draws.min(99999)), ink);

    // Il grafico: una barretta per fotogramma, alta quanto è durato. La scala è il doppio
    // della media mostrata, così a ritmo costante le barre stanno a metà e gli scatti sporgono.
    let (gx, gy, gh) = (tx, py + 58.0 * u, 12.0 * u);
    let scale = (s.ms * 2.0).max(1.0);
    let step = (px + pw - 12.0 * u - gx) / HISTORY as f32;
    d.rect(gx, gy + gh, step * HISTORY as f32 - u, t * 0.75, [dim[0], dim[1], dim[2], 0.35]);
    for (i, &ms) in s.times.iter().enumerate() {
        if ms <= 0.0 {
            continue;
        }
        let f = (ms / scale).min(1.0);
        let spike = ms > s.ms * 1.6 && s.ms > 0.0;
        let age = 0.35 + 0.65 * (i as f32 / HISTORY as f32);
        let c = if spike { [0.95, 0.30, 0.26, 0.95] } else { [ink[0], ink[1], ink[2], 0.85 * age] };
        let bh = (f * gh).max(t);
        d.rect(gx + i as f32 * step, gy + gh - bh, (step - u).max(t), bh, c);
    }

    // Da pixel a coordinate di clip.
    let (sx, sy) = (2.0 / w.max(1) as f32, 2.0 / h.max(1) as f32);
    for v in &mut d.v {
        v.pos = [v.pos[0] * sx - 1.0, 1.0 - v.pos[1] * sy];
    }
    d.v
}

pub fn bytes(v: &[Vertex]) -> &[u8] {
    // SAFETY: `Vertex` è `repr(C)` di soli f32, senza riempimento.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), std::mem::size_of_val(v)) }
}

pub const SHADER: &str = "
struct V { @builtin(position) p: vec4f, @location(0) c: vec4f }
@vertex fn vs(@location(0) pos: vec2f, @location(1) color: vec4f) -> V { return V(vec4f(pos, 0.0, 1.0), color); }
@fragment fn fs(v: V) -> @location(0) vec4f { return v.c; }
";
