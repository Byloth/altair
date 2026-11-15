//! Bink video (`binkw32.dll`) without RAD's library: the videos are decoded by our own
//! `altair-bink` (video identical to the reference, see `tools/binktest.py`), and their audio
//! (separate 5.1 tracks, summed to stereo) goes to the mixer in `dsound.rs`.
//!
//! `ALTAIR_BINK=ffmpeg` uses instead an `ffmpeg` process read from a pipe, as it was before:
//! it stays as a term of comparison. `ALTAIR_VIDEO=0` skips the videos.

use crate::win32::{Call, Runtime, Shim, Shim::L};
use std::collections::HashMap;
use std::io::Read;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

struct Video {
    path: std::path::PathBuf,
    name: String,
    w: u32,
    h: u32,
    frames: u32,
    /// Duration of one frame.
    period: Duration,
    child: Option<Child>,
    out: Option<ChildStdout>,
    /// Current frame, from 1; how many have been read from the pipe.
    frame: u32,
    decoded: u32,
    /// Y, Cr and Cb planes of the current frame, in guest memory (the game reads them too).
    planes: [u32; 3],
    /// When the first frame was shown, corrected for pauses.
    start: Option<Instant>,
    paused: Option<Instant>,
    audio: Option<u32>,
    calls: HashMap<&'static str, u64>,
    /// Our own decoder; `None` when ffmpeg is used.
    ours: Option<Ours>,
}

/// How many frames of audio are decoded ahead of the video.
const AUDIO_AHEAD: u32 = 20;

/// The state of our own decoder for an open video.
struct Ours {
    bink: altair_bink::Bink<std::io::BufReader<std::fs::File>>,
    video: altair_bink::VideoDecoder,
    /// One decoder per track (the ones we cannot decode stay `None`).
    tracks: Vec<Option<altair_bink::AudioDecoder>>,
    /// Frames whose audio has already been handed to the mixer.
    audio_done: u32,
    /// Where the mixer reads the audio from (stereo, 48 kHz, 16 bit).
    sink: Option<std::sync::Arc<Mutex<Vec<i16>>>>,
}

impl Ours {
    fn open(path: &std::path::Path) -> Option<Ours> {
        let bink = altair_bink::Bink::open(std::io::BufReader::new(std::fs::File::open(path).ok()?)).ok()?;
        let video = altair_bink::VideoDecoder::new(bink.width, bink.height, bink.revision);
        // The mixer works at 48 kHz: tracks at another rate (the bass, at 2 kHz) are left out.
        let tracks = bink.tracks.iter().map(|t| if t.sample_rate == 48_000 { altair_bink::AudioDecoder::new(t).ok() } else { None }).collect();
        Some(Ours { bink, video, tracks, audio_done: 0, sink: None })
    }

    /// Decodes the audio up to frame `upto` (excluded) and queues it for the mixer.
    /// With four tracks (separate 5.1) it sums front, centre and rear; otherwise the first one.
    fn pump_audio(&mut self, upto: u32) {
        let Some(sink) = self.sink.clone() else { return };
        let surround = self.tracks.len() >= 4;
        while self.audio_done < upto.min(self.bink.frames()) {
            let Ok(packet) = self.bink.packet(self.audio_done) else { break };
            self.audio_done += 1;
            let mut mix: Vec<f32> = Vec::new();
            for (i, dec) in self.tracks.iter_mut().enumerate() {
                let Some(dec) = dec else { continue };
                if !(i == 0 || (surround && (i == 1 || i == 3))) {
                    continue;
                }
                let mut samples = Vec::new();
                if dec.decode(&packet.audio[i], &mut samples).is_err() {
                    continue;
                }
                let channels = dec.channels();
                let frames = samples.len() / channels;
                if mix.len() < 2 * frames {
                    mix.resize(2 * frames, 0.0);
                }
                for f in 0..frames {
                    // A mono track (the centre) goes to both sides, attenuated.
                    let (l, r) = if channels >= 2 { (samples[f * channels], samples[f * channels + 1]) } else { (samples[f] * 0.707, samples[f] * 0.707) };
                    mix[2 * f] += l;
                    mix[2 * f + 1] += r;
                }
            }
            let mut out = sink.lock().unwrap_or_else(|e| e.into_inner());
            out.extend(mix.iter().map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16));
        }
    }
}

static VIDEOS: LazyLock<Mutex<HashMap<u32, Video>>> = LazyLock::new(Default::default);
fn videos() -> MutexGuard<'static, HashMap<u32, Video>> {
    VIDEOS.lock().unwrap_or_else(|e| e.into_inner())
}

impl Video {
    fn spawn(&mut self) -> bool {
        self.stop();
        let child = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(&self.path)
            .args(["-an", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        match child {
            Ok(mut c) => {
                self.out = c.stdout.take();
                self.child = Some(c);
                self.decoded = 0;
                true
            }
            Err(_) => false,
        }
    }
    fn stop(&mut self) {
        self.out = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
    /// Puts the current frame into the guest planes using our own decoder.
    fn decode_ours(&mut self, mem: crate::machine::Mem) {
        let Some(o) = self.ours.as_mut() else { return };
        if self.frame < self.decoded {
            // The video restarts: the first frame is always a key frame.
            o.video = altair_bink::VideoDecoder::new(self.w, self.h, o.bink.revision);
            self.decoded = 0;
        }
        o.pump_audio(self.frame + AUDIO_AHEAD);
        let mut fresh = false;
        while self.decoded < self.frame {
            let Ok(packet) = o.bink.packet(self.decoded) else { return };
            self.decoded += 1;
            if let Err(e) = o.video.decode(&packet.video) {
                eprintln!("[video] {}: frame {} cannot be decoded ({e})", self.name, self.decoded);
                return;
            }
            fresh = true;
        }
        if !fresh {
            return; // already in the planes
        }
        // The game's planes are Y, Cr, Cb; the decoder's are Y, Cb, Cr.
        let frame = o.video.current();
        for (guest, plane) in [(self.planes[0], 0), (self.planes[1], 2), (self.planes[2], 1)] {
            let (w, h) = frame.plane_size(plane);
            for y in 0..h {
                mem.write(guest + (y * w) as u32, frame.row(plane, y));
            }
        }
    }

    /// Puts the current frame into the guest planes, reading the missing ones from the pipe.
    fn decode(&mut self, mem: crate::machine::Mem) {
        if self.ours.is_some() {
            return self.decode_ours(mem);
        }
        if self.frame <= self.decoded && self.frame != 1 {
            return; // already decoded
        }
        if self.frame < self.decoded || self.out.is_none() {
            self.spawn(); // the video restarts
        }
        let (y, c) = ((self.w * self.h) as usize, ((self.w / 2) * (self.h / 2)) as usize);
        let mut buf = vec![0u8; y + 2 * c];
        while self.decoded < self.frame {
            let Some(out) = self.out.as_mut() else { return };
            if out.read_exact(&mut buf).is_err() {
                self.out = None; // end of the stream: the last frame stays
                return;
            }
            self.decoded += 1;
        }
        // yuv420p carries Y, U (Cb), V (Cr); Bink's planes are Y, Cr, Cb.
        mem.write(self.planes[0], &buf[..y]);
        mem.write(self.planes[2], &buf[y..y + c]);
        mem.write(self.planes[1], &buf[y + c..]);
    }
}

/// The videos open right now, for whoever drives the game from the control channel.
pub fn playing() -> String {
    videos().values().map(|v| v.name.rsplit(['\\', '/']).next().unwrap_or("").to_string()).collect::<Vec<_>>().join(",")
}

fn count(c: &Call, what: &'static str) {
    if let Some(v) = videos().get_mut(&c.arg(0)) {
        *v.calls.entry(what).or_default() += 1;
    }
}

/// `BinkOpen(name, flags)`: the header gives size, frame count and frame rate.
fn bink_open(rt: &mut Runtime, c: &mut Call) -> u32 {
    let name = rt.mem.cstr(c.arg(0));
    let skip = |why: &str| {
        eprintln!("[video] {name} skipped: {why}");
        0
    };
    if std::env::var("ALTAIR_VIDEO").is_ok_and(|v| v == "0") {
        return skip("ALTAIR_VIDEO=0");
    }
    let Some((path, _)) = rt.fs.resolve(&name) else { return skip("invalid path") };
    let mut head = [0u8; 44];
    if std::fs::File::open(&path).and_then(|mut f| f.read_exact(&mut head)).is_err() || &head[..3] != b"BIK" {
        return skip("file not readable");
    }
    let field = |i: usize| u32::from_le_bytes(head[i..i + 4].try_into().unwrap());
    let (frames, w, h, fps_num, fps_den) = (field(8), field(20), field(24), field(28).max(1), field(32).max(1));
    let handle = rt.halloc(1024, true);
    let planes = [rt.halloc(w * h, true), rt.halloc((w / 2) * (h / 2), true), rt.halloc((w / 2) * (h / 2), true)];
    // Without data the chroma is zero, i.e. green: we start from black.
    rt.mem.fill(planes[1], (w / 2) * (h / 2), 128);
    rt.mem.fill(planes[2], (w / 2) * (h / 2), 128);
    let mut v = Video {
        path: path.clone(),
        name: name.clone(),
        w,
        h,
        frames,
        period: Duration::from_nanos(1_000_000_000 * fps_den as u64 / fps_num as u64),
        child: None,
        out: None,
        frame: 1,
        decoded: 0,
        planes,
        start: None,
        paused: None,
        audio: None,
        calls: HashMap::new(),
        ours: None,
    };
    let use_ffmpeg = std::env::var("ALTAIR_BINK").is_ok_and(|v| v == "ffmpeg");
    if !use_ffmpeg {
        v.ours = Ours::open(&path);
        if v.ours.is_none() {
            eprintln!("[video] {name}: our own decoder cannot open it, trying ffmpeg");
        }
    }
    if v.ours.is_none() && !v.spawn() {
        for p in planes.into_iter().chain([handle]) {
            rt.hfree(p);
        }
        return skip("ffmpeg not available");
    }
    // BINK structure: the initial fields, the ones the game reads.
    for (i, val) in [w, h, frames, 1, 0, fps_num, fps_den, 0, c.arg(1)].iter().enumerate() {
        rt.mem.w32(handle + 4 * i as u32, *val);
    }
    if let Some(o) = v.ours.as_mut() {
        if head[40] > 0 {
            let (id, sink) = crate::dsound::play_stream();
            v.audio = Some(id);
            o.sink = Some(sink);
            o.pump_audio(AUDIO_AHEAD);
        }
    } else if head[40] > 0 {
        v.audio = crate::dsound::play_file(&path, head[40] as u32);
    }
    eprintln!("[video] {name}: {w}x{h}, {frames} frames at {:.2} per second, {} audio tracks ({})", fps_num as f64 / fps_den as f64, head[40], if v.ours.is_some() { "our own decoder" } else { "ffmpeg" });
    videos().insert(handle, v);
    handle
}

fn bink_close(rt: &mut Runtime, c: &mut Call) -> u32 {
    if let Some(mut v) = videos().remove(&c.arg(0)) {
        v.stop();
        if let Some(a) = v.audio {
            crate::dsound::stop_file(a);
        }
        let mut calls: Vec<_> = v.calls.iter().collect();
        calls.sort();
        eprintln!("[video] {} closed at frame {} of {}; calls: {calls:?}", v.name, v.frame, v.frames);
        for p in v.planes.into_iter().chain([c.arg(0)]) {
            rt.hfree(p);
        }
    }
    0
}

fn bink_do_frame(rt: &mut Runtime, c: &mut Call) -> u32 {
    count(c, "DoFrame");
    if let Some(v) = videos().get_mut(&c.arg(0)) {
        v.decode(rt.mem);
        v.start.get_or_insert_with(Instant::now);
    }
    0
}

fn bink_next_frame(rt: &mut Runtime, c: &mut Call) -> u32 {
    count(c, "NextFrame");
    if let Some(v) = videos().get_mut(&c.arg(0)) {
        rt.mem.w32(c.arg(0) + 16, v.frame); // LastFrameNum
        if v.frame >= v.frames {
            // Bink starts over: this is how looping videos run (loading screen).
            v.frame = 1;
            v.start = None;
        } else {
            v.frame += 1;
        }
        rt.mem.w32(c.arg(0) + 12, v.frame);
    }
    0
}

/// `BinkWait`: 1 until it is time for the current frame.
fn bink_wait(_: &mut Runtime, c: &mut Call) -> u32 {
    count(c, "Wait");
    let all = videos();
    let Some(v) = all.get(&c.arg(0)) else { return 0 };
    if v.paused.is_some() {
        return 1;
    }
    let Some(start) = v.start else { return 0 };
    (start.elapsed() < v.period * (v.frame - 1)) as u32
}

fn bink_pause(_: &mut Runtime, c: &mut Call) -> u32 {
    count(c, "Pause");
    if let Some(v) = videos().get_mut(&c.arg(0)) {
        match (c.arg(1) != 0, v.paused) {
            (true, None) => v.paused = Some(Instant::now()),
            (false, Some(since)) => {
                v.start = v.start.map(|s| s + since.elapsed());
                v.paused = None;
            }
            _ => {}
        }
        if let Some(a) = v.audio {
            crate::dsound::pause_file(a, c.arg(1) != 0);
        }
    }
    0
}

/// `BinkCopyToBuffer(bink, dest, pitch, height, x, y, flags)`: converts the frame to BGRA.
/// The 16 and 24 bit formats are not handled (the game asks for 32 bit).
fn bink_copy_to_buffer(rt: &mut Runtime, c: &mut Call) -> u32 {
    count(c, "CopyToBuffer");
    let all = videos();
    let Some(v) = all.get(&c.arg(0)) else { return 0 };
    let (dest, pitch, dest_h, x0, y0, flags) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4), c.arg(5), c.arg(6));
    if !matches!(flags & 0xf, 3..=6) {
        eprintln!("[video] BinkCopyToBuffer with format {} not handled", flags & 0xf);
        return 0;
    }
    let rgb_order = matches!(flags & 0xf, 4 | 6); // 32R, 32RA: red first
    let m = rt.mem;
    let (yp, crp, cbp) = (m.bytes(v.planes[0], v.w * v.h), m.bytes(v.planes[1], (v.w / 2) * (v.h / 2)), m.bytes(v.planes[2], (v.w / 2) * (v.h / 2)));
    let mut row = vec![0u8; (v.w * 4) as usize];
    for y in 0..v.h.min(dest_h.saturating_sub(y0)) {
        for x in 0..v.w {
            let ci = ((y / 2) * (v.w / 2) + x / 2) as usize;
            // Limited-range BT.601, like Bink videos.
            let l = 1.164 * (yp[(y * v.w + x) as usize] as f32 - 16.0);
            let (cr, cb) = (crp[ci] as f32 - 128.0, cbp[ci] as f32 - 128.0);
            let (r, g, b) = ((l + 1.596 * cr) as i32, (l - 0.813 * cr - 0.391 * cb) as i32, (l + 2.018 * cb) as i32);
            let px = [b.clamp(0, 255) as u8, g.clamp(0, 255) as u8, r.clamp(0, 255) as u8, 255];
            let o = (x * 4) as usize;
            row[o..o + 4].copy_from_slice(&if rgb_order { [px[2], px[1], px[0], 255] } else { px });
        }
        m.write(dest + (y0 + y) * pitch + x0 * 4, &row);
    }
    0
}

/// `BinkGetFrameBuffersInfo(bink, info)`: the YUV planes, for those who convert them themselves with a shader.
fn bink_get_frame_buffers_info(rt: &mut Runtime, c: &mut Call) -> u32 {
    count(c, "GetFrameBuffersInfo");
    let all = videos();
    let Some(v) = all.get(&c.arg(0)) else { return 0 };
    let p = c.arg(1);
    rt.mem.fill(p, 24 + 2 * 48, 0);
    for (i, val) in [1, v.w, v.h, v.w / 2, v.h / 2, 0].iter().enumerate() {
        rt.mem.w32(p + 4 * i as u32, *val);
    }
    // Frames[0]: YPlane, cRPlane, cBPlane (Allocate, Buffer, BufferPitch); APlane empty.
    for (i, (plane, pitch)) in [(v.planes[0], v.w), (v.planes[1], v.w / 2), (v.planes[2], v.w / 2)].iter().enumerate() {
        let q = p + 24 + 12 * i as u32;
        rt.mem.w32(q, 1);
        rt.mem.w32(q + 4, *plane);
        rt.mem.w32(q + 8, *pitch);
    }
    1
}

fn bink_set_volume(_: &mut Runtime, c: &mut Call) -> u32 {
    count(c, "SetVolume");
    if let Some(a) = videos().get(&c.arg(0)).and_then(|v| v.audio) {
        crate::dsound::volume_file(a, c.arg(2) as f32 / 32768.0);
    }
    0
}

fn zero(_: &mut Runtime, _: &mut Call) -> u32 {
    0
}
fn one(_: &mut Runtime, _: &mut Call) -> u32 {
    1
}
fn should_skip(_: &mut Runtime, c: &mut Call) -> u32 {
    count(c, "ShouldSkip");
    0
}

pub static SHIMS: &[(&str, u32, Shim)] = &[
    ("_BinkSetMemory@8", 2, L(zero)),
    ("_BinkSetSoundSystem@8", 2, L(one)),
    ("_BinkOpenDirectSound@4", 1, L(one)),
    ("_BinkSetSoundTrack@8", 2, L(zero)),
    ("_BinkOpen@8", 2, L(bink_open)),
    ("_BinkClose@4", 1, L(bink_close)),
    ("_BinkCopyToBuffer@28", 7, L(bink_copy_to_buffer)),
    ("_BinkDoFrame@4", 1, L(bink_do_frame)),
    ("_BinkNextFrame@4", 1, L(bink_next_frame)),
    ("_BinkPause@8", 2, L(bink_pause)),
    ("_BinkSetVolume@12", 3, L(bink_set_volume)),
    ("_BinkShouldSkip@4", 1, L(should_skip)),
    ("_BinkWait@4", 1, L(bink_wait)),
    ("_BinkGetFrameBuffersInfo@8", 2, L(bink_get_frame_buffers_info)),
];
