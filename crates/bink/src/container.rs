//! Il contenitore Bink: intestazione, tracce audio, indice dei fotogrammi.
//!
//! ```text
//! "BIK" + revisione, u32 dimensione - 8, u32 fotogrammi, u32 fotogramma più grande, u32,
//! u32 larghezza, u32 altezza, u32 fps numeratore, u32 fps denominatore, u32 flag video,
//! u32 tracce audio; per traccia u32 (dimensione massima), poi u16 frequenza + u16 flag,
//! poi u32 id; infine fotogrammi + 1 offset u32 (bit 0 = fotogramma chiave).
//! Ogni fotogramma: per traccia u32 lunghezza + dati audio; poi il video.
//! ```

use crate::{Error, Result};
use std::io::{Read, Seek, SeekFrom};

const FLAG_ALPHA: u32 = 0x0010_0000;
const AUDIO_DCT: u16 = 0x1000;
const AUDIO_STEREO: u16 = 0x2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioTrack {
    pub sample_rate: u32,
    pub channels: u32,
    /// Trasformata usata: DCT, altrimenti RDFT (non gestita).
    pub dct: bool,
    pub id: u32,
}

/// Un fotogramma del contenitore, diviso nelle sue parti.
pub struct Packet {
    pub keyframe: bool,
    /// Un pacchetto audio per traccia (vuoto se la traccia non ha dati in questo fotogramma).
    pub audio: Vec<Vec<u8>>,
    pub video: Vec<u8>,
}

pub struct Bink<R> {
    reader: R,
    pub revision: u8,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub video_flags: u32,
    pub tracks: Vec<AudioTrack>,
    /// (offset, dimensione, fotogramma chiave) di ogni fotogramma.
    index: Vec<(u64, u32, bool)>,
}

impl<R: Read + Seek> Bink<R> {
    pub fn open(mut reader: R) -> Result<Bink<R>> {
        let mut head = [0u8; 44];
        reader.read_exact(&mut head)?;
        let field = |i: usize| u32::from_le_bytes(head[i..i + 4].try_into().unwrap());
        if &head[..3] != b"BIK" {
            return Err(Error::Unsupported("non è un file Bink 1".into()));
        }
        let revision = head[3];
        if !matches!(revision, b'f' | b'g' | b'h' | b'i') {
            return Err(Error::Unsupported(format!("revisione Bink '{}' non gestita", revision as char)));
        }
        let (file_size, frames) = (field(4) as u64 + 8, field(8));
        let (width, height, fps_num, fps_den, video_flags, track_count) = (field(20), field(24), field(28), field(32), field(36), field(40));
        if frames == 0 || frames > 1_000_000 || width == 0 || height == 0 || width > 7680 || height > 4800 || fps_num == 0 || fps_den == 0 || track_count > 256 {
            return Err(Error::Unsupported("intestazione Bink non valida".into()));
        }
        if video_flags & FLAG_ALPHA != 0 {
            return Err(Error::Unsupported("Bink con canale alfa non gestito".into()));
        }
        let mut u32s = |n: usize| -> Result<Vec<u32>> {
            let mut raw = vec![0u8; 4 * n];
            reader.read_exact(&mut raw)?;
            Ok(raw.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect())
        };
        let n = track_count as usize;
        let _max_sizes = u32s(n)?;
        let formats = u32s(n)?;
        let ids = u32s(n)?;
        let tracks = formats
            .iter()
            .zip(&ids)
            .map(|(f, id)| {
                let flags = (f >> 16) as u16;
                AudioTrack { sample_rate: f & 0xffff, channels: if flags & AUDIO_STEREO != 0 { 2 } else { 1 }, dct: flags & AUDIO_DCT != 0, id: *id }
            })
            .collect();
        let offsets = u32s(frames as usize + 1)?;
        let mut index = Vec::with_capacity(frames as usize);
        for i in 0..frames as usize {
            let start = (offsets[i] & !1) as u64;
            let end = if i + 1 == frames as usize { file_size } else { (offsets[i + 1] & !1) as u64 };
            if end <= start || end - start > u32::MAX as u64 {
                return Err(Error::Unsupported("indice dei fotogrammi non valido".into()));
            }
            index.push((start, (end - start) as u32, offsets[i] & 1 != 0));
        }
        Ok(Bink { reader, revision, width, height, fps_num, fps_den, video_flags, tracks, index })
    }

    pub fn frames(&self) -> u32 {
        self.index.len() as u32
    }

    /// Dimensione su disco di un fotogramma e se è un fotogramma chiave.
    pub fn frame_info(&self, frame: u32) -> Option<(u32, bool)> {
        self.index.get(frame as usize).map(|e| (e.1, e.2))
    }

    /// Legge un fotogramma (da 0) e lo divide in audio e video.
    pub fn packet(&mut self, frame: u32) -> Result<Packet> {
        let &(offset, size, keyframe) = self.index.get(frame as usize).ok_or(Error::Invalid("fotogramma oltre la fine"))?;
        self.reader.seek(SeekFrom::Start(offset))?;
        let mut data = vec![0u8; size as usize];
        self.reader.read_exact(&mut data)?;
        let mut pos = 0usize;
        let mut audio = Vec::with_capacity(self.tracks.len());
        for _ in 0..self.tracks.len() {
            let len = data.get(pos..pos + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize).ok_or(Error::Invalid("pacchetto audio troncato"))?;
            pos += 4;
            let body = data.get(pos..pos + len).ok_or(Error::Invalid("pacchetto audio oltre il fotogramma"))?;
            audio.push(body.to_vec());
            pos += len;
        }
        Ok(Packet { keyframe, audio, video: data[pos..].to_vec() })
    }
}
