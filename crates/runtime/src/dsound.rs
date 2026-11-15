//! DirectSound 8 su un mixer nostro. I buffer esistono in memoria guest; un thread li
//! miscela ogni 10 ms a 48 kHz stereo e manda il risultato a PulseAudio (`libpulse-simple`,
//! caricata a tempo di esecuzione: se manca, o con `ALTAIR_AUDIO=0`, il mixer gira muto).
//! È il mixer a far avanzare i cursori di riproduzione, quindi il gioco (DARE) tiene il suo
//! ritmo anche senza uscita audio.

use crate::com::{self, Method};
use crate::win32::{Call, Runtime, Shim::L};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// I buffer hanno un lock loro, separato da quello del runtime: il mixer non deve aspettare
/// il gioco, che il lock del runtime lo tiene quasi sempre.
static BUFFERS: std::sync::LazyLock<Mutex<HashMap<u32, Buffer>>> = std::sync::LazyLock::new(Default::default);
fn buffers() -> MutexGuard<'static, HashMap<u32, Buffer>> {
    BUFFERS.lock().unwrap_or_else(|e| e.into_inner())
}

const IID_BUFFER8: &str = "6825A449-7524-4D82-920F-50E36AB3AB1E";
const IID_BUFFER: &str = "279AFA85-4981-11CE-A521-0020AF0BE560";
const IID_3D_BUFFER: &str = "279AFA86-4981-11CE-A521-0020AF0BE560";
const IID_3D_LISTENER: &str = "279AFA84-4981-11CE-A521-0020AF0BE560";
const IID_PROPERTY_SET: &str = "31EFAC30-515C-11D0-A9AA-00AA0061BE93";

pub struct Buffer {
    data: u32,
    size: u32,
    format: [u8; 18],
    bytes_per_sec: u32,
    /// Cursore di riproduzione, in fotogrammi audio (con parte frazionaria).
    cursor: f64,
    playing: bool,
    looping: bool,
    volume: u32,
    pan: u32,
    frequency: u32,
    primary: bool,
}

impl Buffer {
    fn block(&self) -> u32 {
        (u16::from_le_bytes([self.format[12], self.format[13]]) as u32).max(1)
    }
    fn position(&self) -> u32 {
        if self.size == 0 {
            return 0;
        }
        (self.cursor as u32 * self.block()).min(if self.looping { self.size - 1 } else { self.size })
    }
}

// --- mixer ----------------------------------------------------------------------------------

const RATE: u32 = 48_000;
/// Fotogrammi per giro del mixer: 10 ms.
const TICK: usize = 480;

/// `libpulse-simple` caricata con `dlopen`: bastano tre funzioni.
struct Pulse {
    handle: *mut libc::c_void,
    write: unsafe extern "C" fn(*mut libc::c_void, *const u8, usize, *mut i32) -> i32,
}

impl Pulse {
    fn open() -> Option<Pulse> {
        if std::env::var("ALTAIR_AUDIO").is_ok_and(|v| v == "0") {
            return None;
        }
        unsafe {
            let lib = libc::dlopen(c"libpulse-simple.so.0".as_ptr(), libc::RTLD_NOW);
            if lib.is_null() {
                eprintln!("[audio] libpulse-simple non trovata: mixer muto");
                return None;
            }
            let new = libc::dlsym(lib, c"pa_simple_new".as_ptr());
            let write = libc::dlsym(lib, c"pa_simple_write".as_ptr());
            if new.is_null() || write.is_null() {
                return None;
            }
            type New = unsafe extern "C" fn(*const i8, *const i8, i32, *const i8, *const i8, *const u32, *const u8, *const u32, *mut i32) -> *mut libc::c_void;
            let new: New = std::mem::transmute(new);
            // pa_sample_spec { format, rate, channels }: S16LE (3), 48 kHz, stereo.
            let spec = [3u32, RATE, 2];
            // pa_buffer_attr { maxlength, tlength, prebuf, minreq, fragsize }. La coda è lunga
            // (`ALTAIR_AUDIO_MS`, 200 ms): con 60 ms il sink di WSLg rallenta fino a un quinto
            // del tempo reale, per tutti i programmi.
            let ms = std::env::var("ALTAIR_AUDIO_MS").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(200).clamp(20, 2000);
            let attr = [u32::MAX, RATE * 4 / 1000 * ms, u32::MAX, u32::MAX, u32::MAX];
            let mut err = 0;
            let handle = new(std::ptr::null(), c"altair".as_ptr(), 1, std::ptr::null(), c"Assassin's Creed".as_ptr(), spec.as_ptr(), std::ptr::null(), attr.as_ptr(), &mut err);
            if handle.is_null() {
                eprintln!("[audio] PulseAudio non raggiungibile (errore {err}): mixer muto");
                return None;
            }
            eprintln!("[audio] uscita su PulseAudio, 48 kHz stereo");
            Some(Pulse { handle, write: std::mem::transmute::<*mut libc::c_void, unsafe extern "C" fn(*mut libc::c_void, *const u8, usize, *mut i32) -> i32>(write) })
        }
    }
}

/// Guadagno lineare da un'attenuazione DirectSound in centesimi di decibel (0 … -10000).
fn gain(hundredths_db: i32) -> f32 {
    if hundredths_db <= -10_000 { 0.0 } else { 10f32.powf(hundredths_db.min(0) as f32 / 2000.0) }
}

/// Aggiunge a `out` (stereo intercalato) `TICK` fotogrammi di ogni buffer che suona e fa
/// avanzare i cursori.
fn mix(mem: crate::machine::Mem, out: &mut [f32]) {
    for b in buffers().values_mut() {
        if !b.playing || b.size == 0 || b.primary {
            continue;
        }
        let block = b.block();
        let channels = u16::from_le_bytes([b.format[2], b.format[3]]).max(1) as u32;
        let bits = u16::from_le_bytes([b.format[14], b.format[15]]);
        let frames = b.size / block;
        if frames == 0 {
            continue;
        }
        let sample = |frame: u32, ch: u32| -> f32 {
            let a = b.data + frame * block + ch.min(channels - 1) * (bits as u32 / 8);
            match bits {
                8 => (mem.r8(a) as f32 - 128.0) / 128.0,
                16 => mem.r16(a) as i16 as f32 / 32768.0,
                _ => 0.0,
            }
        };
        let volume = gain(b.volume as i32);
        let pan = b.pan as i32;
        let (gl, gr) = (volume * gain(-pan.max(0)), volume * gain(pan.min(0)));
        let step = b.frequency as f64 / RATE as f64;
        let mut cursor = b.cursor;
        let mut ended = false;
        for i in 0..TICK {
            if cursor >= frames as f64 {
                if !b.looping {
                    ended = true;
                    break;
                }
                cursor -= frames as f64;
            }
            if volume > 0.0 {
                let f0 = cursor as u32;
                let f1 = if f0 + 1 < frames { f0 + 1 } else if b.looping { 0 } else { f0 };
                let t = (cursor - f0 as f64) as f32;
                let l = sample(f0, 0) * (1.0 - t) + sample(f1, 0) * t;
                let r = if channels > 1 { sample(f0, 1) * (1.0 - t) + sample(f1, 1) * t } else { l };
                out[2 * i] += l * gl;
                out[2 * i + 1] += r * gr;
            }
            cursor += step;
        }
        b.cursor = if ended { frames as f64 } else { cursor };
        if ended {
            b.playing = false;
        }
    }
}

// --- audio dei filmati -------------------------------------------------------------------------

/// L'audio di un filmato: ffmpeg lo decodifica in un thread (48 kHz, stereo, 16 bit) e il mixer
/// lo suona man mano che arriva.
struct FileSound {
    samples: std::sync::Arc<Mutex<Vec<i16>>>,
    /// Prossimo campione da suonare.
    pos: usize,
    paused: bool,
    volume: f32,
}

static NEXT_FILE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
static FILES: std::sync::LazyLock<Mutex<HashMap<u32, FileSound>>> = std::sync::LazyLock::new(Default::default);
fn files() -> MutexGuard<'static, HashMap<u32, FileSound>> {
    FILES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Avvia l'audio di un file multimediale. Le tracce di un Bink in 5.1 sono separate (fronte,
/// centro, bassi, retro): si sommano fronte, centro e retro.
pub fn play_file(path: &std::path::Path, tracks: u32) -> Option<u32> {
    use std::io::Read;
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-i"]).arg(path);
    if tracks >= 4 {
        cmd.args(["-filter_complex", "[0:a:0][0:a:1][0:a:3]amix=inputs=3:normalize=0[a]", "-map", "[a]"]);
    } else {
        cmd.args(["-map", "0:a:0"]);
    }
    cmd.args(["-vn", "-f", "s16le", "-ar", "48000", "-ac", "2", "-"]);
    let mut child = cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
    let mut out = child.stdout.take()?;
    let samples = std::sync::Arc::new(Mutex::new(Vec::new()));
    let sink = samples.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1 << 16];
        let mut odd: Option<u8> = None;
        while let Ok(n) = out.read(&mut buf) {
            if n == 0 || std::sync::Arc::strong_count(&sink) == 1 {
                break; // fine del flusso, o nessuno ascolta più
            }
            let mut v = sink.lock().unwrap_or_else(|e| e.into_inner());
            for &b in &buf[..n] {
                match odd.take() {
                    Some(lo) => v.push(i16::from_le_bytes([lo, b])),
                    None => odd = Some(b),
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    });
    let id = NEXT_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    files().insert(id, FileSound { samples, pos: 0, paused: false, volume: 1.0 });
    Some(id)
}
/// Un suono alimentato da chi chiama: ritorna l'identificativo e la coda in cui mettere i
/// campioni (stereo intercalato, 48 kHz, 16 bit). Il mixer li suona man mano che arrivano.
pub fn play_stream() -> (u32, std::sync::Arc<Mutex<Vec<i16>>>) {
    let samples = std::sync::Arc::new(Mutex::new(Vec::new()));
    let id = NEXT_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    files().insert(id, FileSound { samples: samples.clone(), pos: 0, paused: false, volume: 1.0 });
    (id, samples)
}
pub fn stop_file(id: u32) {
    files().remove(&id);
}
pub fn pause_file(id: u32, paused: bool) {
    if let Some(f) = files().get_mut(&id) {
        f.paused = paused;
    }
}
pub fn volume_file(id: u32, volume: f32) {
    if let Some(f) = files().get_mut(&id) {
        f.volume = volume.clamp(0.0, 1.0);
    }
}

fn mix_files(out: &mut [f32]) {
    for f in files().values_mut().filter(|f| !f.paused) {
        let samples = f.samples.lock().unwrap_or_else(|e| e.into_inner());
        let n = out.len().min(samples.len().saturating_sub(f.pos));
        for (o, s) in out.iter_mut().zip(&samples[f.pos..f.pos + n]) {
            *o += *s as f32 / 32768.0 * f.volume;
        }
        f.pos += n;
    }
}

/// Il thread del mixer: parte con il primo dispositivo DirectSound.
fn mixer_thread(mem: crate::machine::Mem) {
    // L'uscita sta in un thread a parte, dietro una coda corta: se PulseAudio si ferma (con
    // WSLg capita) si perde del suono, ma i cursori, e con loro il gioco, non rallentano.
    let (tx, rx) = std::sync::mpsc::sync_channel::<[u8; 4 * TICK]>(6);
    let _ = std::thread::Builder::new().name("audio-out".into()).spawn(move || {
        let Some(p) = Pulse::open() else { return };
        for chunk in rx {
            let mut err = 0;
            unsafe { (p.write)(p.handle, chunk.as_ptr(), chunk.len(), &mut err) };
        }
    });
    let mut dropped = 0u64;
    // `ALTAIR_AUDIO_DUMP=file.raw`: il miscelato anche su file (S16LE, 48 kHz, stereo), per
    // controllarlo senza ascoltare.
    let mut dump = std::env::var_os("ALTAIR_AUDIO_DUMP").and_then(|p| std::fs::File::create(p).ok());
    let mut next = Instant::now();
    let mut out = [0f32; 2 * TICK];
    let mut pcm = [0u8; 4 * TICK];
    loop {
        next += Duration::from_millis(10);
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_millis(200) {
            next = now; // rimasto indietro (gioco fermo in un caricamento): non si rincorre
        }
        out.fill(0.0);
        mix(mem, &mut out);
        mix_files(&mut out);
        for (i, v) in out.iter().enumerate() {
            let s = (v.clamp(-1.0, 1.0) * 32767.0) as i16;
            pcm[2 * i..2 * i + 2].copy_from_slice(&s.to_le_bytes());
        }
        if let Some(f) = &mut dump {
            let _ = std::io::Write::write_all(f, &pcm);
        }
        if let Err(std::sync::mpsc::TrySendError::Full(_)) = tx.try_send(pcm) {
            dropped += 1;
            if dropped.is_power_of_two() {
                eprintln!("[audio] uscita in ritardo: {dropped} blocchi da 10 ms scartati finora");
            }
        }
    }
}

// --- IDirectSound8 --------------------------------------------------------------------------

pub fn direct_sound_create8(rt: &mut Runtime, c: &mut Call) -> u32 {
    let obj = com::create(rt, "IDirectSound8", DSOUND, 0);
    rt.mem.w32(c.arg(1), obj);
    let mem = rt.mem;
    static MIXER: std::sync::Once = std::sync::Once::new();
    MIXER.call_once(|| {
        std::thread::Builder::new().name("mixer".into()).spawn(move || mixer_thread(mem)).expect("thread del mixer");
    });
    eprintln!("[audio] DirectSound creato");
    0
}

fn ds_get_caps(rt: &mut Runtime, c: &mut Call) -> u32 {
    let p = c.arg(1);
    let size = rt.mem.r32(p).min(96);
    rt.mem.fill(p + 4, size.saturating_sub(4), 0);
    rt.mem.w32(p + 4, 0x0f5f); // primario e secondari, mono e stereo, 8 e 16 bit
    rt.mem.w32(p + 8, 100);
    rt.mem.w32(p + 12, 200_000);
    rt.mem.w32(p + 16, 1);
    0
}

fn ds_create_sound_buffer(rt: &mut Runtime, c: &mut Call) -> u32 {
    let desc = c.arg(1);
    let (flags, size, wfx) = (rt.mem.r32(desc + 4), rt.mem.r32(desc + 8), rt.mem.r32(desc + 16));
    let mut format = [0u8; 18];
    if wfx != 0 {
        format.copy_from_slice(rt.mem.bytes(wfx, 18));
    } else {
        // Formato predefinito del buffer primario: 44,1 kHz, stereo, 16 bit.
        format[..16].copy_from_slice(&[1, 0, 2, 0, 0x44, 0xac, 0, 0, 0x10, 0xb1, 2, 0, 4, 0, 16, 0]);
    }
    let bytes_per_sec = u32::from_le_bytes(format[8..12].try_into().unwrap());
    let rate = u32::from_le_bytes(format[4..8].try_into().unwrap());
    let primary = flags & 1 != 0;
    let data = if size > 0 { rt.halloc(size, true) } else { 0 };
    let obj = com::create(rt, "IDirectSoundBuffer8", BUFFER, 0);
    if rt.trace {
        eprintln!("    buffer audio {obj:#x}: {size} byte, {rate} Hz, flag {flags:#x}{}", if primary { " (primario)" } else { "" });
    }
    buffers().insert(obj, Buffer { data, size, format, bytes_per_sec, cursor: 0.0, playing: false, looping: false, volume: 0, pan: 0, frequency: rate, primary });
    rt.mem.w32(c.arg(2), obj);
    0
}

fn ds_duplicate(rt: &mut Runtime, c: &mut Call) -> u32 {
    let copy = {
        let all = buffers();
        let Some(src) = all.get(&c.arg(1)) else { return 0x8007_0057 };
        Buffer { playing: false, cursor: 0.0, ..Buffer { format: src.format, ..*src } }
    };
    let obj = com::create(rt, "IDirectSoundBuffer8", BUFFER, 0);
    buffers().insert(obj, copy);
    rt.mem.w32(c.arg(2), obj);
    0
}

fn ds_get_speaker_config(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.w32(c.arg(1), 4); // DSSPEAKER_STEREO
    0
}

static DSOUND: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("CreateSoundBuffer", 4, Some(L(ds_create_sound_buffer))),
    ("GetCaps", 2, Some(L(ds_get_caps))),
    ("DuplicateSoundBuffer", 3, Some(L(ds_duplicate))),
    ("SetCooperativeLevel", 3, Some(L(com::ok))),
    ("Compact", 1, Some(L(com::ok))),
    ("GetSpeakerConfig", 2, Some(L(ds_get_speaker_config))),
    ("SetSpeakerConfig", 2, Some(L(com::ok))),
    ("Initialize", 2, Some(L(com::ok))),
    ("VerifyCertification", 2, None),
];

// --- IDirectSoundBuffer8 --------------------------------------------------------------------

/// Un buffer con il lock della tabella ancora in mano.
struct BufRef(MutexGuard<'static, HashMap<u32, Buffer>>, u32);
impl std::ops::Deref for BufRef {
    type Target = Buffer;
    fn deref(&self) -> &Buffer {
        &self.0[&self.1]
    }
}
impl std::ops::DerefMut for BufRef {
    fn deref_mut(&mut self) -> &mut Buffer {
        self.0.get_mut(&self.1).unwrap()
    }
}

fn buf(_: &mut Runtime, c: &Call) -> Option<BufRef> {
    let all = buffers();
    all.contains_key(&c.arg(0)).then(|| BufRef(all, c.arg(0)))
}

fn buf_query_interface(rt: &mut Runtime, c: &mut Call) -> u32 {
    let iid = com::guid(rt, c.arg(1));
    let obj = match iid.as_str() {
        IID_BUFFER8 | IID_BUFFER => {
            com::add_ref(rt, c);
            c.arg(0)
        }
        IID_3D_BUFFER => com::create(rt, "IDirectSound3DBuffer", BUFFER_3D, 0),
        IID_3D_LISTENER => com::create(rt, "IDirectSound3DListener", LISTENER_3D, 0),
        IID_PROPERTY_SET => com::create(rt, "IKsPropertySet", PROPERTY_SET, 0),
        _ => return com::no_interface(rt, c),
    };
    rt.mem.w32(c.arg(2), obj);
    0
}

fn buf_get_caps(rt: &mut Runtime, c: &mut Call) -> u32 {
    let p = c.arg(1);
    let (size, primary) = buf(rt, c).map_or((0, false), |b| (b.size, b.primary));
    rt.mem.w32(p + 4, if primary { 0x1 } else { 0x8 | 0xe0 }); // software, volume/pan/frequenza
    rt.mem.w32(p + 8, size);
    rt.mem.w32(p + 12, 0);
    rt.mem.w32(p + 16, 0);
    0
}

fn buf_get_current_position(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(b) = buf(rt, c) else { return 0x8007_0057 };
    let (play, size, lead) = (b.position(), b.size, b.bytes_per_sec / 50 & !3);
    let write = if size > 0 { (play + lead) % size } else { 0 };
    if c.arg(1) != 0 {
        rt.mem.w32(c.arg(1), play);
    }
    if c.arg(2) != 0 {
        rt.mem.w32(c.arg(2), write);
    }
    0
}

fn buf_get_format(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(format) = buf(rt, c).map(|b| b.format) else { return 0x8007_0057 };
    let n = c.arg(2).min(18);
    if c.arg(1) != 0 {
        rt.mem.write(c.arg(1), &format[..n as usize]);
    }
    if c.arg(3) != 0 {
        rt.mem.w32(c.arg(3), 18);
    }
    0
}

fn buf_get_volume(rt: &mut Runtime, c: &mut Call) -> u32 {
    let v = buf(rt, c).map_or(0, |b| b.volume);
    rt.mem.w32(c.arg(1), v);
    0
}
fn buf_get_pan(rt: &mut Runtime, c: &mut Call) -> u32 {
    let v = buf(rt, c).map_or(0, |b| b.pan);
    rt.mem.w32(c.arg(1), v);
    0
}
fn buf_get_frequency(rt: &mut Runtime, c: &mut Call) -> u32 {
    let v = buf(rt, c).map_or(0, |b| b.frequency);
    rt.mem.w32(c.arg(1), v);
    0
}
fn buf_get_status(rt: &mut Runtime, c: &mut Call) -> u32 {
    let st = buf(rt, c).map_or(0, |b| {
        let ended = !b.looping && b.position() >= b.size;
        if b.playing && !ended { 1 | if b.looping { 4 } else { 0 } } else { 0 }
    });
    rt.mem.w32(c.arg(1), st);
    0
}

fn buf_lock(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (mut offset, mut bytes, flags) = (c.arg(1), c.arg(2), c.arg(7));
    let Some(b) = buf(rt, c) else { return 0x8007_0057 };
    let (data, size) = (b.data, b.size);
    if size == 0 {
        return 0x8878_0064; // DSERR_BUFFERLOST: il primario non si blocca
    }
    if flags & 1 != 0 {
        offset = (b.position() + (b.bytes_per_sec / 50 & !3)) % size; // DSBLOCK_FROMWRITECURSOR
    }
    if flags & 2 != 0 {
        bytes = size; // DSBLOCK_ENTIREBUFFER
    }
    offset %= size;
    let bytes = bytes.min(size);
    let first = bytes.min(size - offset);
    rt.mem.w32(c.arg(3), data + offset);
    rt.mem.w32(c.arg(4), first);
    if c.arg(5) != 0 {
        rt.mem.w32(c.arg(5), if bytes > first { data } else { 0 });
    }
    if c.arg(6) != 0 {
        rt.mem.w32(c.arg(6), bytes - first);
    }
    0
}

fn buf_play(rt: &mut Runtime, c: &mut Call) -> u32 {
    let looping = c.arg(3) & 1 != 0;
    if let Some(mut b) = buf(rt, c) {
        if !b.playing && !looping && b.position() >= b.size {
            b.cursor = 0.0; // un suono finito riparte dall'inizio
        }
        b.playing = true;
        b.looping = looping;
    }
    0
}
fn buf_stop(rt: &mut Runtime, c: &mut Call) -> u32 {
    if let Some(mut b) = buf(rt, c) {
        b.playing = false;
    }
    0
}
fn buf_set_current_position(rt: &mut Runtime, c: &mut Call) -> u32 {
    let pos = c.arg(1);
    if let Some(mut b) = buf(rt, c) {
        b.cursor = (pos / b.block()) as f64;
    }
    0
}
fn buf_set_format(rt: &mut Runtime, c: &mut Call) -> u32 {
    let f: [u8; 18] = rt.mem.bytes(c.arg(1), 18).try_into().unwrap();
    if let Some(mut b) = buf(rt, c) {
        b.format = f;
        b.bytes_per_sec = u32::from_le_bytes(f[8..12].try_into().unwrap());
        b.frequency = u32::from_le_bytes(f[4..8].try_into().unwrap());
    }
    0
}
fn buf_set_volume(rt: &mut Runtime, c: &mut Call) -> u32 {
    let v = c.arg(1);
    if let Some(mut b) = buf(rt, c) {
        b.volume = v;
    }
    0
}
fn buf_set_pan(rt: &mut Runtime, c: &mut Call) -> u32 {
    let v = c.arg(1);
    if let Some(mut b) = buf(rt, c) {
        b.pan = v;
    }
    0
}
fn buf_set_frequency(rt: &mut Runtime, c: &mut Call) -> u32 {
    let v = c.arg(1);
    if let Some(mut b) = buf(rt, c) {
        if v != 0 {
            b.frequency = v;
            b.bytes_per_sec = v * b.block();
        }
    }
    0
}

static BUFFER: &[Method] = &[
    ("QueryInterface", 3, Some(L(buf_query_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("GetCaps", 2, Some(L(buf_get_caps))),
    ("GetCurrentPosition", 3, Some(L(buf_get_current_position))),
    ("GetFormat", 4, Some(L(buf_get_format))),
    ("GetVolume", 2, Some(L(buf_get_volume))),
    ("GetPan", 2, Some(L(buf_get_pan))),
    ("GetFrequency", 2, Some(L(buf_get_frequency))),
    ("GetStatus", 2, Some(L(buf_get_status))),
    ("Initialize", 3, Some(L(com::ok))),
    ("Lock", 8, Some(L(buf_lock))),
    ("Play", 4, Some(L(buf_play))),
    ("SetCurrentPosition", 2, Some(L(buf_set_current_position))),
    ("SetFormat", 2, Some(L(buf_set_format))),
    ("SetVolume", 2, Some(L(buf_set_volume))),
    ("SetPan", 2, Some(L(buf_set_pan))),
    ("SetFrequency", 2, Some(L(buf_set_frequency))),
    ("Stop", 1, Some(L(buf_stop))),
    ("Unlock", 5, Some(L(com::ok))),
    ("Restore", 1, Some(L(com::ok))),
    ("SetFX", 4, Some(L(com::ok))),
    ("AcquireResources", 4, Some(L(com::ok))),
    ("GetObjectInPath", 5, None),
];

// --- 3D ed EAX: si accetta ogni impostazione, non si risponde alle domande ------------------

static LISTENER_3D: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("GetAllParameters", 2, None),
    ("GetDistanceFactor", 2, None),
    ("GetDopplerFactor", 2, None),
    ("GetOrientation", 3, None),
    ("GetPosition", 2, None),
    ("GetRolloffFactor", 2, None),
    ("GetVelocity", 2, None),
    ("SetAllParameters", 3, Some(L(com::ok))),
    ("SetDistanceFactor", 3, Some(L(com::ok))),
    ("SetDopplerFactor", 3, Some(L(com::ok))),
    ("SetOrientation", 8, Some(L(com::ok))),
    ("SetPosition", 5, Some(L(com::ok))),
    ("SetRolloffFactor", 3, Some(L(com::ok))),
    ("SetVelocity", 5, Some(L(com::ok))),
    ("CommitDeferredSettings", 1, Some(L(com::ok))),
];

static BUFFER_3D: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("GetAllParameters", 2, None),
    ("GetConeAngles", 3, None),
    ("GetConeOrientation", 2, None),
    ("GetConeOutsideVolume", 2, None),
    ("GetMaxDistance", 2, None),
    ("GetMinDistance", 2, None),
    ("GetMode", 2, None),
    ("GetPosition", 2, None),
    ("GetVelocity", 2, None),
    ("SetAllParameters", 3, Some(L(com::ok))),
    ("SetConeAngles", 4, Some(L(com::ok))),
    ("SetConeOrientation", 5, Some(L(com::ok))),
    ("SetConeOutsideVolume", 3, Some(L(com::ok))),
    ("SetMaxDistance", 3, Some(L(com::ok))),
    ("SetMinDistance", 3, Some(L(com::ok))),
    ("SetMode", 3, Some(L(com::ok))),
    ("SetPosition", 5, Some(L(com::ok))),
    ("SetVelocity", 5, Some(L(com::ok))),
];

/// `IKsPropertySet`: la porta d'accesso a EAX. Nessuna proprietà è supportata.
fn prop_unsupported(_: &mut Runtime, _: &mut Call) -> u32 {
    0x8007_0490 // E_PROP_ID_UNSUPPORTED
}
fn prop_query_support(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.w32(c.arg(3), 0);
    0x8007_0490
}

static PROPERTY_SET: &[Method] = &[
    ("QueryInterface", 3, Some(L(com::no_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("Get", 8, Some(L(prop_unsupported))),
    ("Set", 7, Some(L(prop_unsupported))),
    ("QuerySupport", 4, Some(L(prop_query_support))),
];
