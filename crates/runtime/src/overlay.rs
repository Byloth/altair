//! The diagnostics panel (F8): frames per second, time per frame, draws.
//!
//! It is drawn by the renderer on top of the finished image, not in the window title, and it
//! wants to look like a piece of the Animus: the whole game is a memory replayed by a machine,
//! and a machine can have its own service panel. No game assets: shapes, glyphs and the
//! animated helix are described below in code, and the colours are the light, cold ones of
//! the Animus interface, with the red of desynchronization when the pace collapses.
//!
//! Everything is made of coloured triangles (no textures), in window pixels.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

static ON: AtomicBool = AtomicBool::new(false);

/// The graph is a history: one bar per time interval, not per frame, or it would
/// scroll away before one could look at it. 56 bars of a quarter of a second: 14 seconds.
const HISTORY: usize = 56;
const BUCKET: f32 = 0.25;
/// Fixed thresholds for red: a bar, once drawn, no longer changes colour.
const SLOW_FPS: f32 = 30.0;
const STUTTER_MS: f32 = 50.0;

/// One interval of the history.
#[derive(Clone, Copy, Default)]
struct Bucket {
    /// Average frames per second of the interval; 0 = still empty.
    fps: f32,
    /// The longest frame of the interval, in milliseconds.
    worst: f32,
}

struct Stats {
    last: Option<Instant>,
    /// The history, from oldest to most recent.
    history: [Bucket; HISTORY],
    /// The interval in progress: start, frames, longest frame.
    bucket: (Instant, u32, f32),
    /// Displayed values, updated twice a second so that they stay readable.
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

/// F8, the `fps` command of the control channel, or `ALTAIR_FPS=1` at startup.
pub fn toggle() -> bool {
    !ON.fetch_xor(true, Ordering::Relaxed)
}

pub fn init() {
    if std::env::var("ALTAIR_FPS").map(|v| v != "0").unwrap_or(false) {
        ON.store(true, Ordering::Relaxed);
    }
}

/// A frame has finished: `Present` calls it, with the draws it contained.
pub fn frame(draws: u64) {
    let now = Instant::now();
    let mut guard = STATS.lock().unwrap_or_else(|e| e.into_inner());
    let s = guard.get_or_insert_with(|| Stats { last: None, history: [Bucket::default(); HISTORY], bucket: (now, 0, 0.0), fps: 0.0, ms: 0.0, draws: 0, acc: (now, 0, 0), started: now });
    if let Some(last) = s.last {
        s.bucket.1 += 1;
        s.bucket.2 = s.bucket.2.max((now - last).as_secs_f32() * 1000.0);
    }
    let span = (now - s.bucket.0).as_secs_f32();
    if span >= BUCKET {
        s.history.rotate_left(1);
        // After a long pause (a loading screen) a single bar remains, low and red.
        s.history[HISTORY - 1] = Bucket { fps: (s.bucket.1 as f32 / span).max(0.5), worst: s.bucket.2 };
        s.bucket = (now, 0, 0.0);
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
    /// In the glyph atlas; negative for solid shapes.
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// 5×7 glyphs, one row per string, from the top. Only the ones that are needed.
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

/// Collects the triangles, in pixels with the origin at the top left.
struct Draw {
    v: Vec<Vertex>,
    srgb: bool,
    /// The Animus fonts read from the game's data; without them, the 5×7 ones above.
    fonts: Option<&'static crate::animus_font::Fonts>,
}

type Color = [f32; 4];

impl Draw {
    fn color(&self, c: Color) -> Color {
        // On an sRGB surface the vertex colours are read as linear.
        if self.srgb {
            [c[0].powf(2.2), c[1].powf(2.2), c[2].powf(2.2), c[3]]
        } else {
            c
        }
    }

    fn tri(&mut self, p: [[f32; 2]; 3], c: Color) {
        let color = self.color(c);
        self.v.extend(p.iter().map(|&pos| Vertex { pos, uv: [-1.0, -1.0], color }));
    }

    fn quad(&mut self, p: [[f32; 2]; 4], c: Color) {
        self.tri([p[0], p[1], p[2]], c);
        self.tri([p[0], p[2], p[3]], c);
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        self.quad([[x, y], [x + w, y], [x + w, y + h], [x, y + h]], c);
    }

    /// Centred diamond: the "pixel" of the helix.
    fn diamond(&mut self, cx: f32, cy: f32, r: f32, c: Color) {
        self.quad([[cx, cy - r], [cx + r, cy], [cx, cy + r], [cx - r, cy]], c);
    }

    /// The Animus font suited to a given capital height: the title one for the
    /// large numbers, the text one for the rest. Also returns the scale factor.
    fn face(&self, cap: f32) -> Option<(&'static crate::animus_font::Face, f32)> {
        let fonts = self.fonts?;
        let face = if cap >= fonts.text.cap * 1.3 { &fonts.title } else { &fonts.text };
        Some((face, cap / face.cap))
    }

    /// Text with capitals `cap` pixels tall and the top edge at `y`; returns the x after
    /// the last character. `track` is extra space between characters, in pixels.
    fn text(&mut self, mut x: f32, y: f32, cap: f32, track: f32, s: &str, c: Color) -> f32 {
        if let Some((face, k)) = self.face(cap) {
            let color = self.color(c);
            let base = y + cap;
            for ch in s.chars() {
                let Some(g) = face.glyphs.get(&ch).or_else(|| face.glyphs.get(&'?')) else { continue };
                if ch != ' ' {
                    let (x0, y0) = (x + g.dx * k, base - g.dy * k);
                    let (x1, y1) = (x0 + g.w * k, y0 + g.h * k);
                    let [u0, v0, u1, v1] = g.uv;
                    let corners = [([x0, y0], [u0, v0]), ([x1, y0], [u1, v0]), ([x1, y1], [u1, v1]), ([x0, y1], [u0, v1])];
                    self.v.extend([0, 1, 2, 0, 2, 3].map(|i| Vertex { pos: corners[i].0, uv: corners[i].1, color }));
                }
                x += g.advance * k + track;
            }
            return x - track;
        }
        // Fallback: 5×7 glyphs, one rectangle for each run of lit dots.
        let k = cap / 7.0;
        for ch in s.chars() {
            for (row, bits) in glyph(ch.to_ascii_uppercase()).iter().enumerate() {
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
            x += 6.0 * k + track;
        }
        x - k - track
    }

    fn text_width(&self, cap: f32, track: f32, s: &str) -> f32 {
        let n = s.chars().count() as f32;
        match self.face(cap) {
            Some((face, k)) => s.chars().filter_map(|ch| face.glyphs.get(&ch)).map(|g| g.advance * k).sum::<f32>() + track * (n - 1.0).max(0.0),
            None => n * 6.0 * cap / 7.0 - cap / 7.0 + track * (n - 1.0).max(0.0),
        }
    }
}

/// The panel's triangles for a window of `w`×`h` pixels, already in clip coordinates.
pub fn build(w: u32, h: u32, srgb: bool, fonts: Option<&'static crate::animus_font::Fonts>) -> Vec<Vertex> {
    let guard = STATS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = guard.as_ref() else { return Vec::new() };
    let mut d = Draw { v: Vec::with_capacity(4096), srgb, fonts };
    // Unit: one pixel at 720 lines, never less than one, rounded to half a pixel so that the
    // glyphs stay sharp.
    let u = ((h as f32 / 720.0) * 2.0).round().max(2.0) / 2.0;
    let (pw, ph) = (268.0 * u, 78.0 * u);
    let (px, py) = (w as f32 - pw - 22.0 * u, 20.0 * u);

    // Animus colours: cold white, light blue, and the red of desynchronization.
    let slow = s.fps > 0.0 && s.fps < 30.0;
    let ink: Color = if slow { [1.0, 0.56, 0.50, 0.96] } else { [0.93, 0.97, 1.0, 0.96] };
    let dim: Color = [0.62, 0.78, 0.92, 0.80];
    let accent: Color = if slow { [0.95, 0.30, 0.26, 0.95] } else { [0.35, 0.80, 1.0, 0.95] };
    let glass: Color = [0.03, 0.06, 0.10, 0.62];

    // The glass: a rectangle with the bottom-left corner cut off.
    let cut = 14.0 * u;
    d.quad([[px, py], [px + pw, py], [px + pw, py + ph], [px + cut, py + ph]], glass);
    d.tri([[px, py], [px + cut, py + ph], [px, py + ph - cut]], glass);
    // Thin borders: full on top, short on the right, the cut in light blue.
    let t = u.max(1.0);
    d.rect(px, py, pw, t, dim);
    d.rect(px, py, 46.0 * u, 2.0 * t, accent);
    d.rect(px + pw - t, py, t, 18.0 * u, dim);
    d.rect(px + cut, py + ph - t, pw - cut - 40.0 * u, t, [dim[0], dim[1], dim[2], 0.35]);
    d.quad([[px, py + ph - cut], [px + t * 1.5, py + ph - cut], [px + cut + t * 1.5, py + ph], [px + cut, py + ph]], accent);

    // The helix: two strands of diamonds rotating slowly, with the rungs between them.
    let phase = s.started.elapsed().as_secs_f32() * 1.6;
    let (hx, hy, hh, hr) = (px + 24.0 * u, py + 12.0 * u, 54.0 * u, 9.0 * u);
    const RUNGS: usize = 9;
    for i in 0..RUNGS {
        let f = i as f32 / (RUNGS - 1) as f32;
        let a = phase + f * 4.4;
        let y = hy + f * hh;
        let (xa, xb) = (hx + a.sin() * hr, hx - a.sin() * hr);
        let depth = a.cos(); // > 0: the first strand is in front
        if i % 2 == 0 {
            d.rect(xa.min(xb), y - t * 0.5, (xa - xb).abs(), t, [dim[0], dim[1], dim[2], 0.30]);
        }
        let (near, far) = if depth >= 0.0 { (xa, xb) } else { (xb, xa) };
        let k = depth.abs();
        d.diamond(far, y, (1.6 + 0.6 * (1.0 - k)) * u, [dim[0], dim[1], dim[2], 0.45 + 0.2 * (1.0 - k)]);
        d.diamond(near, y, (2.2 + 1.0 * k) * u, if i % 3 == 0 { accent } else { ink });
    }

    // Header. With the real fonts the labels are a little larger than the dot-matrix ones,
    // and spaced as in the Animus interface.
    let real = d.fonts.is_some();
    let (label, track) = if real { (8.0 * u, 1.2 * u) } else { (7.0 * u, 0.0) };
    let tx = px + 46.0 * u;
    let end = d.text(tx, py + 8.0 * u, label, track, "ANIMUS", dim);
    d.text(end + 6.0 * u, py + 8.0 * u, label, track, "/ SYNC", [dim[0], dim[1], dim[2], 0.5]);

    // The large number, right-aligned over four digits, and its unit.
    let fps = format!("{:.0}", s.fps.min(9999.0));
    let big = 28.0 * u;
    let slot = d.text_width(big, 0.0, "0000");
    let nx = tx + slot - d.text_width(big, 0.0, &fps);
    let end = d.text(nx, py + 22.0 * u, big, 0.0, &fps, ink);
    d.text(end + 6.0 * u, py + 22.0 * u + big - label, label, track, "FPS", accent);

    // Right column: milliseconds and draws.
    let cx = px + 180.0 * u;
    let small = if real { 11.0 * u } else { 10.5 * u };
    // The values are right-aligned, against the edge of the panel.
    let right = px + pw - 13.0 * u;
    for (row, name, value) in [(0.0, "MS", format!("{:.1}", s.ms.min(999.9))), (16.0, "DRAW", format!("{}", s.draws.min(99999)))] {
        d.text(cx, py + (24.0 + row) * u, label, track, name, dim);
        let x = right - d.text_width(small, 0.0, &value);
        d.text(x, py + (22.0 + row) * u, small, 0.0, &value, ink);
    }

    // The history: one bar every quarter of a second, as tall as the frames per second of
    // that interval; the oldest on the left, fainter. The scale is a fixed step
    // (30, 60, 120…) chosen from the visible maximum: it changes only when the pace really changes.
    // Red if the interval was slow (below 30) or had a stutter (a frame over
    // 50 ms): absolute thresholds, so a bar does not change colour afterwards.
    let (gx, gy, gh) = (tx, py + 58.0 * u, 12.0 * u);
    let top = s.history.iter().map(|b| b.fps).fold(0.0f32, f32::max);
    let scale = [30.0, 60.0, 120.0, 240.0, 480.0, 960.0].into_iter().find(|&v| top <= v).unwrap_or(1920.0);
    let step = (px + pw - 12.0 * u - gx) / HISTORY as f32;
    d.rect(gx, gy + gh, step * HISTORY as f32 - u, t * 0.75, [dim[0], dim[1], dim[2], 0.35]);
    for (i, b) in s.history.iter().enumerate() {
        if b.fps <= 0.0 {
            continue;
        }
        let bad = b.fps < SLOW_FPS || b.worst > STUTTER_MS;
        let age = 0.35 + 0.65 * (i as f32 / HISTORY as f32);
        let c = if bad { [0.95, 0.30, 0.26, 0.55 + 0.4 * age] } else { [0.93, 0.97, 1.0, 0.85 * age] };
        let bh = ((b.fps / scale).min(1.0) * gh).max(t);
        d.rect(gx + i as f32 * step, gy + gh - bh, (step - u).max(t), bh, c);
    }

    // From pixels to clip coordinates.
    let (sx, sy) = (2.0 / w.max(1) as f32, 2.0 / h.max(1) as f32);
    for v in &mut d.v {
        v.pos = [v.pos[0] * sx - 1.0, 1.0 - v.pos[1] * sy];
    }
    d.v
}

pub fn bytes(v: &[Vertex]) -> &[u8] {
    // SAFETY: `Vertex` is `repr(C)` made only of f32, with no padding.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), std::mem::size_of_val(v)) }
}

pub const SHADER: &str = "
struct V { @builtin(position) p: vec4f, @location(0) c: vec4f, @location(1) uv: vec2f }
@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@vertex fn vs(@location(0) pos: vec2f, @location(1) uv: vec2f, @location(2) color: vec4f) -> V { return V(vec4f(pos, 0.0, 1.0), color, uv); }
@fragment fn fs(v: V) -> @location(0) vec4f {
    // Solid shapes have negative uv; glyphs take their coverage from the atlas.
    let cover = textureSampleLevel(atlas, samp, max(v.uv, vec2f(0.0)), 0.0).r;
    return vec4f(v.c.rgb, v.c.a * select(cover, 1.0, v.uv.x < 0.0));
}
";
