//! DirectSound 8 on a mixer of our own. The buffers live in guest memory; a thread mixes
//! them every 10 ms at 48 kHz stereo and sends the result to PulseAudio (`libpulse-simple`,
//! loaded at run time: if it is missing, or with `ALTAIR_AUDIO=0`, the mixer runs silent).
//! It is the mixer that advances the play cursors, so the game (DARE) keeps its
//! pace even without audio output.

use crate::com::{self, Method};
use crate::win32::{Call, Runtime, Shim::{F, L}};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The buffers have a lock of their own, separate from the runtime's: the mixer must not wait
/// for the game, which holds the runtime lock almost all the time.
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
    /// Play cursor, in audio frames (with a fractional part).
    cursor: f64,
    playing: bool,
    looping: bool,
    volume: u32,
    pan: u32,
    frequency: u32,
    primary: bool,
    /// For each thirty-second of the buffer, whether it has been written since the last time the
    /// mixer played it; the thirty-second the cursor is in and whether it is "stale"; whether the
    /// game fills the buffer while it plays (a stream). Used to count the game's delays.
    /// Last region handed to the game with `Lock` (offset, bytes), for `ALTAIR_AUDIO_LOG`.
    locked: (u32, u32),
    fresh: [bool; 32],
    at: (usize, bool),
    streamed: bool,
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

/// `ALTAIR_AUDIO_LOG=1`: one line for every Play, Stop, position change and Lock of the buffers,
/// with the play cursor at that moment. Used to understand how the game fills them.
fn audio_log() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("ALTAIR_AUDIO_LOG").is_ok_and(|v| v == "1"))
}

/// Mixer passes since startup; passes in which a stream played data not rewritten in time, and
/// stream passes in total (to give a proportion).
static TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static STALE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static STREAMED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// --- mixer ----------------------------------------------------------------------------------

const RATE: u32 = crate::host::AUDIO_RATE;
/// Frames per mixer pass: 10 ms.
const TICK: usize = 480;

/// Linear gain from a DirectSound attenuation in hundredths of a decibel (0 … -10000).
fn gain(hundredths_db: i32) -> f32 {
    if hundredths_db <= -10_000 { 0.0 } else { 10f32.powf(hundredths_db.min(0) as f32 / 2000.0) }
}

/// Adds to `out` (interleaved stereo) `TICK` frames of every buffer that is playing and
/// advances the cursors.
fn mix(mem: crate::machine::Mem, out: &mut [f32]) {
    TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
        // A stream that enters a region already played and not rewritten since is repeating data
        // one lap old: the game did not fill it in time.
        if b.streamed && b.looping {
            let chunk = ((cursor as u64 % frames as u64) * 32 / frames as u64) as usize;
            if chunk != b.at.0 {
                b.at = (chunk, !b.fresh[chunk]);
                b.fresh[chunk] = false;
            }
            if volume > 0.0 {
                STREAMED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if b.at.1 {
                    STALE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
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

// --- video audio -------------------------------------------------------------------------------

/// The audio of a video: ffmpeg decodes it in a thread (48 kHz, stereo, 16 bit) and the mixer
/// plays it as it arrives.
struct FileSound {
    samples: std::sync::Arc<Mutex<Vec<i16>>>,
    /// Next sample to play.
    pos: usize,
    paused: bool,
    volume: f32,
}

static NEXT_FILE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
static FILES: std::sync::LazyLock<Mutex<HashMap<u32, FileSound>>> = std::sync::LazyLock::new(Default::default);
fn files() -> MutexGuard<'static, HashMap<u32, FileSound>> {
    FILES.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts the audio of a media file. The tracks of a 5.1 Bink are separate (front,
/// centre, bass, rear): front, centre and rear are summed.
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
                break; // end of the stream, or nobody is listening any more
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
/// A sound fed by the caller: returns the identifier and the queue in which to put the
/// samples (interleaved stereo, 48 kHz, 16 bit). The mixer plays them as they arrive.
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

/// The mixer thread: it starts with the first DirectSound device.
fn mixer_thread(mem: crate::machine::Mem) {
    // The output lives in a separate thread, behind a short queue: if PulseAudio stalls (it happens
    // with WSLg) some sound is lost, but the cursors, and the game with them, do not slow down.
    let (tx, rx) = std::sync::mpsc::sync_channel::<[u8; 4 * TICK]>(6);
    let _ = std::thread::Builder::new().name("audio-out".into()).spawn(move || {
        if std::env::var("ALTAIR_AUDIO").is_ok_and(|v| v == "0") {
            return;
        }
        let Some(mut out) = crate::host::AudioOut::open() else { return };
        for chunk in rx {
            out.write(&chunk);
        }
    });
    let mut dropped = 0u64;
    // `ALTAIR_AUDIO_DUMP=file.raw`: the mix also goes to a file (S16LE, 48 kHz, stereo), to
    // check it without listening.
    let mut dump = std::env::var_os("ALTAIR_AUDIO_DUMP").and_then(|p| std::fs::File::create(p).ok());
    let mut next = Instant::now();
    let mut out = [0f32; 2 * TICK];
    let mut pcm = [0u8; 4 * TICK];
    loop {
        next += Duration::from_millis(10);
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_millis(20) {
            // Late: we do not catch up. Recovering the lost time would make the cursors advance
            // in jumps, and the game keeps only 60 ms of sound ready beyond the cursor.
            next = now;
        }
        if TICKS.load(std::sync::atomic::Ordering::Relaxed) % 500 == 499 {
            let (stale, all) = (STALE.swap(0, std::sync::atomic::Ordering::Relaxed), STREAMED.swap(0, std::sync::atomic::Ordering::Relaxed));
            if stale > 0 {
                eprintln!("[audio] in the last 5 s, {} ms of streams out of {} played from regions not rewritten in time by the game", stale * 10, all * 10);
            }
        }
        out.fill(0.0);
        mix(mem, &mut out);
        // Debug fast-forward (clock.rs): the cursors must advance with the game's clock, or
        // the game's sound engine, which times its streams with that clock, loses track of
        // them (a scene waiting for a voice line then never ends). Nothing of it is heard:
        // one block in every `speed` would only be noise.
        let speed = crate::clock::speed();
        for _ in 1..speed {
            let mut unheard = [0f32; 2 * TICK];
            mix(mem, &mut unheard);
        }
        if speed > 1 {
            out.fill(0.0);
        }
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
                eprintln!("[audio] output running late: {dropped} 10 ms blocks dropped so far");
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
        std::thread::Builder::new().name("mixer".into()).spawn(move || mixer_thread(mem)).expect("mixer thread");
    });
    eprintln!("[audio] DirectSound created");
    0
}

fn ds_get_caps(rt: &mut Runtime, c: &mut Call) -> u32 {
    let p = c.arg(1);
    let size = rt.mem.r32(p).min(96);
    rt.mem.fill(p + 4, size.saturating_sub(4), 0);
    rt.mem.w32(p + 4, 0x0f5f); // primary and secondary, mono and stereo, 8 and 16 bit
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
        // Default format of the primary buffer: 44.1 kHz, stereo, 16 bit.
        format[..16].copy_from_slice(&[1, 0, 2, 0, 0x44, 0xac, 0, 0, 0x10, 0xb1, 2, 0, 4, 0, 16, 0]);
    }
    let bytes_per_sec = u32::from_le_bytes(format[8..12].try_into().unwrap());
    let rate = u32::from_le_bytes(format[4..8].try_into().unwrap());
    let primary = flags & 1 != 0;
    if audio_log() {
        let raw = if wfx != 0 { rt.mem.bytes(wfx, 40).iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ") } else { "(no format)".into() };
        eprintln!("[audio] CreateSoundBuffer flags {flags:#x}, {size} bytes, format: {raw}");
    }
    let data = if size > 0 { rt.halloc(size, true) } else { 0 };
    let obj = com::create(rt, "IDirectSoundBuffer8", BUFFER, 0);
    if rt.trace {
        eprintln!("    audio buffer {obj:#x}: {size} bytes, {rate} Hz, flags {flags:#x}{}", if primary { " (primary)" } else { "" });
    }
    buffers().insert(obj, Buffer { data, size, format, bytes_per_sec, cursor: 0.0, playing: false, looping: false, volume: 0, pan: 0, frequency: rate, primary, locked: (0, 0), fresh: [true; 32], at: (usize::MAX, false), streamed: false });
    rt.mem.w32(c.arg(2), obj);
    0
}

fn ds_duplicate(rt: &mut Runtime, c: &mut Call) -> u32 {
    let copy = {
        let all = buffers();
        let Some(src) = all.get(&c.arg(1)) else { return 0x8007_0057 };
        Buffer { playing: false, cursor: 0.0, locked: (0, 0), fresh: [true; 32], at: (usize::MAX, false), streamed: false, ..Buffer { format: src.format, ..*src } }
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

/// A buffer with the table lock still held.
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

fn buf(c: &Call) -> Option<BufRef> {
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

fn buf_get_caps(c: &mut Call) -> u32 {
    let p = c.arg(1);
    let (size, primary) = buf(c).map_or((0, false), |b| (b.size, b.primary));
    c.mem.w32(p + 4, if primary { 0x1 } else { 0x8 | 0xe0 }); // software, volume/pan/frequency
    c.mem.w32(p + 8, size);
    c.mem.w32(p + 12, 0);
    c.mem.w32(p + 16, 0);
    0
}

fn buf_get_current_position(c: &mut Call) -> u32 {
    let Some(b) = buf(c) else { return 0x8007_0057 };
    let (play, size, lead) = (b.position(), b.size, b.bytes_per_sec / 50 & !3);
    let write = if size > 0 { (play + lead) % size } else { 0 };
    if c.arg(1) != 0 {
        c.mem.w32(c.arg(1), play);
    }
    if c.arg(2) != 0 {
        c.mem.w32(c.arg(2), write);
    }
    0
}

fn buf_get_format(c: &mut Call) -> u32 {
    let Some(format) = buf(c).map(|b| b.format) else { return 0x8007_0057 };
    let n = c.arg(2).min(18);
    if c.arg(1) != 0 {
        c.mem.write(c.arg(1), &format[..n as usize]);
    }
    if c.arg(3) != 0 {
        c.mem.w32(c.arg(3), 18);
    }
    0
}

fn buf_get_volume(c: &mut Call) -> u32 {
    let v = buf(c).map_or(0, |b| b.volume);
    c.mem.w32(c.arg(1), v);
    0
}
fn buf_get_pan(c: &mut Call) -> u32 {
    let v = buf(c).map_or(0, |b| b.pan);
    c.mem.w32(c.arg(1), v);
    0
}
fn buf_get_frequency(c: &mut Call) -> u32 {
    let v = buf(c).map_or(0, |b| b.frequency);
    c.mem.w32(c.arg(1), v);
    0
}
fn buf_get_status(c: &mut Call) -> u32 {
    let st = buf(c).map_or(0, |b| {
        let ended = !b.looping && b.position() >= b.size;
        if b.playing && !ended { 1 | if b.looping { 4 } else { 0 } } else { 0 }
    });
    c.mem.w32(c.arg(1), st);
    0
}

fn buf_lock(c: &mut Call) -> u32 {
    let (mut offset, mut bytes, flags) = (c.arg(1), c.arg(2), c.arg(7));
    let Some(mut b) = buf(c) else { return 0x8007_0057 };
    let (data, size) = (b.data, b.size);
    if size == 0 {
        return 0x8878_0064; // DSERR_BUFFERLOST: the primary cannot be locked
    }
    if flags & 1 != 0 {
        offset = (b.position() + (b.bytes_per_sec / 50 & !3)) % size; // DSBLOCK_FROMWRITECURSOR
    }
    if flags & 2 != 0 {
        bytes = size; // DSBLOCK_ENTIREBUFFER
    }
    offset %= size;
    let bytes = bytes.min(size);
    // Records when each region was written: the mixer uses it to notice when it is playing
    // data one lap old.
    b.streamed |= b.playing;
    b.locked = (offset, bytes);
    // Investigation (`ALTAIR_AUDIO_TRAP=N`, Linux only): on the N-th fill of the game's mixing
    // buffer the region becomes read-only, and whoever writes to it gets named.
    #[cfg(unix)]
    if b.playing && b.size == 37888 {
        static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if std::env::var("ALTAIR_AUDIO_TRAP").ok().and_then(|v| v.parse::<u64>().ok()) == Some(n) {
            eprintln!("[audio] write trap on {:#x}, {bytes} bytes", data + offset);
            crate::host::write_trap(c.mem.ptr(data + offset), bytes as usize);
        }
    }
    let (first_chunk, chunks) = (offset as u64 * 32 / size as u64, (bytes as u64 * 32).div_ceil(size as u64).min(32));
    for k in 0..chunks {
        b.fresh[((first_chunk + k) % 32) as usize] = true;
    }
    if audio_log() {
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        eprintln!("[audio] {:#x} Lock offset {offset} bytes {bytes} flags {flags:#x}; cursor {} of {size}, {}; t={} ms thread {}", c.arg(0), b.position(), if b.playing { "playing" } else { "stopped" }, START.get_or_init(Instant::now).elapsed().as_millis(), std::thread::current().name().unwrap_or("?"));
    }
    let first = bytes.min(size - offset);
    c.mem.w32(c.arg(3), data + offset);
    c.mem.w32(c.arg(4), first);
    if c.arg(5) != 0 {
        c.mem.w32(c.arg(5), if bytes > first { data } else { 0 });
    }
    if c.arg(6) != 0 {
        c.mem.w32(c.arg(6), bytes - first);
    }
    0
}

fn buf_play(c: &mut Call) -> u32 {
    let looping = c.arg(3) & 1 != 0;
    if let Some(mut b) = buf(c) {
        if !b.playing && !looping && b.position() >= b.size {
            b.cursor = 0.0; // a finished sound restarts from the beginning
        }
        b.playing = true;
        b.looping = looping;
        if audio_log() {
            let f = b.format;
            eprintln!("[audio] {:#x} Play loop={looping} cursor {} of {}; format tag {} channels {} {} Hz {} bit, frequency {}, volume {}", c.arg(0), b.position(), b.size, u16::from_le_bytes([f[0], f[1]]), u16::from_le_bytes([f[2], f[3]]), u32::from_le_bytes([f[4], f[5], f[6], f[7]]), u16::from_le_bytes([f[14], f[15]]), b.frequency, b.volume as i32);
        }
    }
    0
}
fn buf_stop(c: &mut Call) -> u32 {
    if let Some(mut b) = buf(c) {
        b.playing = false;
        if audio_log() {
            eprintln!("[audio] {:#x} Stop at cursor {}", c.arg(0), b.position());
        }
    }
    0
}
fn buf_set_current_position(c: &mut Call) -> u32 {
    let pos = c.arg(1);
    if let Some(mut b) = buf(c) {
        if audio_log() {
            eprintln!("[audio] {:#x} SetCurrentPosition {pos} (was {})", c.arg(0), b.position());
        }
        b.cursor = (pos / b.block()) as f64;
    }
    0
}
fn buf_set_format(c: &mut Call) -> u32 {
    let f: [u8; 18] = c.mem.bytes(c.arg(1), 18).try_into().unwrap();
    if audio_log() {
        eprintln!("[audio] {:#x} SetFormat: {}", c.arg(0), c.mem.bytes(c.arg(1), 40).iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
    }
    if let Some(mut b) = buf(c) {
        b.format = f;
        b.bytes_per_sec = u32::from_le_bytes(f[8..12].try_into().unwrap());
        b.frequency = u32::from_le_bytes(f[4..8].try_into().unwrap());
    }
    0
}
fn buf_set_volume(c: &mut Call) -> u32 {
    let v = c.arg(1);
    if let Some(mut b) = buf(c) {
        b.volume = v;
    }
    0
}
fn buf_set_pan(c: &mut Call) -> u32 {
    let v = c.arg(1);
    if let Some(mut b) = buf(c) {
        b.pan = v;
    }
    0
}
fn buf_set_frequency(c: &mut Call) -> u32 {
    let v = c.arg(1);
    if let Some(mut b) = buf(c) {
        if v != 0 {
            b.frequency = v;
            b.bytes_per_sec = v * b.block();
        }
    }
    0
}

/// These methods do not touch the runtime and do not take its lock: the game's audio thread
/// calls them continuously and must fill every buffer within 60 ms, while that lock is contended
/// by the drawing thread (hundreds of thousands of times per frame). Waiting for it, the thread
/// arrived late and the mixer played stale data: the sound "crackled".
fn free_ok(_: &mut Call) -> u32 {
    0
}

/// `Unlock`. With `ALTAIR_AUDIO_LOG=1` it appends what the game has just written to a stream to
/// `run/ctl/stream-<buffer>.raw` (samples as they are): used to look at the waveform
/// coming out of the game's decoder, before our mixer.
fn buf_unlock(c: &mut Call) -> u32 {
    if audio_log() {
        if let Some(b) = buf(c) {
            if b.streamed {
                let (offset, bytes) = b.locked;
                let first = bytes.min(b.size - offset);
                let mut data = c.mem.bytes(b.data + offset, first).to_vec();
                data.extend_from_slice(c.mem.bytes(b.data, bytes - first));
                let path = format!("run/ctl/stream-{:x}.raw", c.arg(0));
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = std::io::Write::write_all(&mut f, &data);
                }
            }
        }
    }
    0
}

static BUFFER: &[Method] = &[
    ("QueryInterface", 3, Some(L(buf_query_interface))),
    ("AddRef", 1, Some(L(com::add_ref))),
    ("Release", 1, Some(L(com::release))),
    ("GetCaps", 2, Some(F(buf_get_caps))),
    ("GetCurrentPosition", 3, Some(F(buf_get_current_position))),
    ("GetFormat", 4, Some(F(buf_get_format))),
    ("GetVolume", 2, Some(F(buf_get_volume))),
    ("GetPan", 2, Some(F(buf_get_pan))),
    ("GetFrequency", 2, Some(F(buf_get_frequency))),
    ("GetStatus", 2, Some(F(buf_get_status))),
    ("Initialize", 3, Some(F(free_ok))),
    ("Lock", 8, Some(F(buf_lock))),
    ("Play", 4, Some(F(buf_play))),
    ("SetCurrentPosition", 2, Some(F(buf_set_current_position))),
    ("SetFormat", 2, Some(F(buf_set_format))),
    ("SetVolume", 2, Some(F(buf_set_volume))),
    ("SetPan", 2, Some(F(buf_set_pan))),
    ("SetFrequency", 2, Some(F(buf_set_frequency))),
    ("Stop", 1, Some(F(buf_stop))),
    ("Unlock", 5, Some(F(buf_unlock))),
    ("Restore", 1, Some(F(free_ok))),
    ("SetFX", 4, Some(F(free_ok))),
    ("AcquireResources", 4, Some(F(free_ok))),
    ("GetObjectInPath", 5, None),
];

// --- 3D and EAX: every setting is accepted, questions are not answered ----------------------

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

/// `IKsPropertySet`: the gateway to EAX. No property is supported.
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
