//! DirectSound 8 muto. Il gioco non parte senza un dispositivo audio, quindi ne offriamo uno
//! che accetta tutto e non suona nulla: i buffer esistono in memoria guest e il cursore di
//! riproduzione avanza con l'orologio, così il mixer del gioco (DARE) tiene il suo ritmo.
//! L'uscita vera arriva in Fase 3.

use crate::com::{self, Method};
use crate::win32::{Call, Runtime, Shim::L};
use std::time::Instant;

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
    /// Posizione al momento dell'ultimo Play/Stop/SetCurrentPosition.
    base: u32,
    playing: Option<Instant>,
    looping: bool,
    volume: u32,
    pan: u32,
    frequency: u32,
    primary: bool,
}

impl Buffer {
    fn position(&self) -> u32 {
        let Some(t) = self.playing else { return self.base };
        if self.size == 0 {
            return 0;
        }
        let adv = (t.elapsed().as_micros() as u64 * self.bytes_per_sec as u64 / 1_000_000) & !3;
        let p = self.base as u64 + adv;
        if self.looping { (p % self.size as u64) as u32 } else { p.min(self.size as u64) as u32 }
    }
}

// --- IDirectSound8 --------------------------------------------------------------------------

pub fn direct_sound_create8(rt: &mut Runtime, c: &mut Call) -> u32 {
    let obj = com::create(rt, "IDirectSound8", DSOUND, 0);
    rt.mem.w32(c.arg(1), obj);
    eprintln!("[audio] DirectSound muto creato");
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
    rt.ds_buffers.insert(obj, Buffer { data, size, format, bytes_per_sec, base: 0, playing: None, looping: false, volume: 0, pan: 0, frequency: rate, primary });
    rt.mem.w32(c.arg(2), obj);
    0
}

fn ds_duplicate(rt: &mut Runtime, c: &mut Call) -> u32 {
    let Some(src) = rt.ds_buffers.get(&c.arg(1)) else { return 0x8007_0057 };
    let copy = Buffer { playing: None, base: 0, ..Buffer { format: src.format, ..*src } };
    let obj = com::create(rt, "IDirectSoundBuffer8", BUFFER, 0);
    rt.ds_buffers.insert(obj, copy);
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

fn buf<'a>(rt: &'a mut Runtime, c: &Call) -> Option<&'a mut Buffer> {
    rt.ds_buffers.get_mut(&c.arg(0))
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
        if b.playing.is_some() && !ended { 1 | if b.looping { 4 } else { 0 } } else { 0 }
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
    if let Some(b) = buf(rt, c) {
        if b.playing.is_none() {
            b.playing = Some(Instant::now());
        }
        b.looping = looping;
    }
    0
}
fn buf_stop(rt: &mut Runtime, c: &mut Call) -> u32 {
    if let Some(b) = buf(rt, c) {
        b.base = b.position();
        b.playing = None;
    }
    0
}
fn buf_set_current_position(rt: &mut Runtime, c: &mut Call) -> u32 {
    let pos = c.arg(1);
    if let Some(b) = buf(rt, c) {
        b.base = pos;
        if b.playing.is_some() {
            b.playing = Some(Instant::now());
        }
    }
    0
}
fn buf_set_format(rt: &mut Runtime, c: &mut Call) -> u32 {
    let f: [u8; 18] = rt.mem.bytes(c.arg(1), 18).try_into().unwrap();
    if let Some(b) = buf(rt, c) {
        b.format = f;
        b.bytes_per_sec = u32::from_le_bytes(f[8..12].try_into().unwrap());
        b.frequency = u32::from_le_bytes(f[4..8].try_into().unwrap());
    }
    0
}
fn buf_set_volume(rt: &mut Runtime, c: &mut Call) -> u32 {
    let v = c.arg(1);
    if let Some(b) = buf(rt, c) {
        b.volume = v;
    }
    0
}
fn buf_set_pan(rt: &mut Runtime, c: &mut Call) -> u32 {
    let v = c.arg(1);
    if let Some(b) = buf(rt, c) {
        b.pan = v;
    }
    0
}
fn buf_set_frequency(rt: &mut Runtime, c: &mut Call) -> u32 {
    let v = c.arg(1);
    if let Some(b) = buf(rt, c) {
        if v != 0 {
            // Cambiare frequenza cambia la velocità del cursore.
            let block = u16::from_le_bytes([b.format[12], b.format[13]]) as u32;
            b.base = b.position();
            if b.playing.is_some() {
                b.playing = Some(Instant::now());
            }
            b.frequency = v;
            b.bytes_per_sec = v * block.max(1);
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
    ("SetFX", 4, None),
    ("AcquireResources", 4, None),
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
