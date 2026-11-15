//! The "Bink audio DCT" audio of a track.
//!
//! Each packet carries one or more blocks. A block is, per channel: two coefficients written
//! out in full, one quantizer per frequency band, then the remaining coefficients in groups with
//! their bit width; an inverse DCT turns them back into samples. Blocks overlap by one
//! sixteenth: the tail of one fades into the head of the next.
//!
//! The arithmetic here is floating point: the result is close to the reference's, not
//! bit-identical.

use crate::bits::Bits;
use crate::container::AudioTrack;
use crate::{Error, Result};

/// Critical frequencies (Hz) that delimit the quantizer bands.
const CRITICAL_FREQS: [u32; 25] = [100, 200, 300, 400, 510, 630, 770, 920, 1080, 1270, 1480, 1720, 2000, 2320, 2700, 3150, 3700, 4400, 5300, 6400, 7700, 9500, 12000, 15500, 24500];
/// Lengths of the coefficient groups, in units of eight.
const RLE_LENGTHS: [usize; 16] = [2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 32, 64];

pub struct AudioDecoder {
    channels: usize,
    /// Samples per block and per channel, before the overlap.
    frame_len: usize,
    overlap: usize,
    root: f32,
    bands: Vec<usize>,
    quant_table: [f32; 96],
    /// The tail of the last block of each channel.
    previous: Vec<Vec<f32>>,
    first: bool,
    dct: Dct,
}

impl AudioDecoder {
    pub fn new(track: &AudioTrack) -> Result<AudioDecoder> {
        if !track.dct {
            return Err(Error::Unsupported("Bink RDFT audio not supported".into()));
        }
        let frame_len = if track.sample_rate < 22050 { 512 } else if track.sample_rate < 44100 { 1024 } else { 2048 };
        let half = (track.sample_rate as u64 + 1) / 2;
        let root = frame_len as f32 / ((frame_len as f32).sqrt() * 32768.0);
        let mut quant_table = [0f32; 96];
        for (i, q) in quant_table.iter_mut().enumerate() {
            *q = (i as f32 * 0.152_891_65).exp() * root;
        }
        let mut num_bands = 1;
        while num_bands < 25 && half > CRITICAL_FREQS[num_bands - 1] as u64 {
            num_bands += 1;
        }
        let mut bands = vec![2usize];
        for i in 1..num_bands {
            bands.push((CRITICAL_FREQS[i - 1] as u64 * frame_len as u64 / half) as usize & !1);
        }
        bands.push(frame_len);
        let channels = track.channels as usize;
        Ok(AudioDecoder { channels, frame_len, overlap: frame_len / 16, root, bands, quant_table, previous: vec![vec![0.0; frame_len / 16]; channels], first: true, dct: Dct::new(frame_len) })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Decodes a packet and appends the samples to `out`, interleaved by channel, in [-1, 1].
    pub fn decode(&mut self, packet: &[u8], out: &mut Vec<f32>) -> Result<()> {
        if packet.len() < 4 {
            return Ok(());
        }
        let mut gb = Bits::new(packet);
        gb.skip(32); // size of the decoded samples
        let mut block = vec![vec![0f32; self.frame_len]; self.channels];
        while gb.left() > 0 {
            self.decode_block(&mut gb, &mut block)?;
            gb.align32();
            for i in 0..self.frame_len - self.overlap {
                for ch in block.iter() {
                    out.push(ch[i]);
                }
            }
        }
        Ok(())
    }

    fn decode_block(&mut self, gb: &mut Bits, out: &mut [Vec<f32>]) -> Result<()> {
        let num_bands = self.bands.len() - 1;
        let mut coeffs = vec![0f32; self.frame_len];
        gb.skip(2);
        for samples in out.iter_mut() {
            if gb.left() < 58 {
                return Err(Error::Invalid("truncated audio block"));
            }
            coeffs[0] = get_float(gb) * self.root;
            coeffs[1] = get_float(gb) * self.root;
            if gb.left() < num_bands as isize * 8 {
                return Err(Error::Invalid("truncated audio quantizers"));
            }
            let mut quant = [0f32; 25];
            for q in quant.iter_mut().take(num_bands) {
                *q = self.quant_table[(gb.get(8) as usize).min(95)];
            }
            let mut k = 0;
            let mut q = quant[0];
            let mut i = 2;
            while i < self.frame_len {
                let j = if gb.bit() { i + RLE_LENGTHS[gb.get(4) as usize] * 8 } else { i + 8 };
                let j = j.min(self.frame_len);
                let width = gb.get(4);
                if width == 0 {
                    coeffs[i..j].fill(0.0);
                    i = j;
                    while self.bands[k] < i {
                        q = quant[k];
                        k += 1;
                    }
                } else {
                    while i < j {
                        if self.bands[k] == i {
                            q = quant[k];
                            k += 1;
                        }
                        let coeff = gb.get(width);
                        coeffs[i] = if coeff == 0 {
                            0.0
                        } else if gb.bit() {
                            -q * coeff as f32
                        } else {
                            q * coeff as f32
                        };
                        i += 1;
                    }
                }
            }
            self.dct.inverse(&coeffs, samples);
        }
        // The head of the block fades in from the tail of the previous one.
        let channels = out.len();
        let count = (self.overlap * channels) as f32;
        for (ch, samples) in out.iter_mut().enumerate() {
            if !self.first {
                let mut j = ch;
                for i in 0..self.overlap {
                    samples[i] = (self.previous[ch][i] * (count - j as f32) + samples[i] * j as f32) / count;
                    j += channels;
                }
            }
            self.previous[ch].copy_from_slice(&samples[self.frame_len - self.overlap..]);
        }
        self.first = false;
        Ok(())
    }
}

/// A floating-point number in the stream's format: 5 bits of exponent, 23 of mantissa, sign.
fn get_float(gb: &mut Bits) -> f32 {
    let power = gb.get(5) as i32;
    let f = gb.get(23) as f32 * 2f32.powi(power - 23);
    if gb.bit() { -f } else { f }
}

/// Inverse DCT (DCT-III) of N points, N a power of two:
/// `x[n] = (2/N)·(c[0] + Σ c[k]·cos(π·k·(2n+1) / 2N))`, computed with an N-point complex
/// FFT. The 2/N factor is what brings the samples into [-1, 1] (verified against the
/// reference: without it, the signal is N/2 times larger).
struct Dct {
    n: usize,
    /// `e^(iπk / 2N)`: the rotation applied to each coefficient before the FFT.
    twist: Vec<(f64, f64)>,
    /// `e^(2πik / N)`: the roots used by the FFT.
    roots: Vec<(f64, f64)>,
    work: Vec<(f64, f64)>,
}

impl Dct {
    fn new(n: usize) -> Dct {
        let angle = |k: usize, d: f64| {
            let a = std::f64::consts::PI * k as f64 / d;
            (a.cos(), a.sin())
        };
        Dct { n, twist: (0..n).map(|k| angle(k, 2.0 * n as f64)).collect(), roots: (0..n).map(|k| angle(2 * k, n as f64)).collect(), work: vec![(0.0, 0.0); n] }
    }

    fn inverse(&mut self, coeffs: &[f32], out: &mut [f32]) {
        let n = self.n;
        let bits = n.trailing_zeros();
        // Rotated coefficients, already in the bit-reversed order the FFT wants as input.
        for k in 0..n {
            let (c, s) = self.twist[k];
            let v = coeffs[k] as f64;
            self.work[k.reverse_bits() >> (usize::BITS - bits)] = (v * c, v * s);
        }
        // Positive-exponent FFT: w[m] = Σ t[k]·e^(2πi·k·m / N).
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for start in (0..n).step_by(len) {
                for i in 0..len / 2 {
                    let (wr, wi) = self.roots[i * step];
                    let (ar, ai) = self.work[start + i];
                    let (br, bi) = self.work[start + i + len / 2];
                    let (tr, ti) = (br * wr - bi * wi, br * wi + bi * wr);
                    self.work[start + i] = (ar + tr, ai + ti);
                    self.work[start + i + len / 2] = (ar - tr, ai - ti);
                }
            }
            len <<= 1;
        }
        // Real part: the even samples are in the first half, the odd ones in the second, backwards.
        let scale = 2.0 / n as f64;
        for m in 0..n / 2 {
            out[2 * m] = (self.work[m].0 * scale) as f32;
            out[2 * m + 1] = (self.work[n - 1 - m].0 * scale) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_dct_matches_the_definition() {
        let n = 64;
        let coeffs: Vec<f32> = (0..n).map(|k| ((k * 37 + 11) % 23) as f32 - 11.0).collect();
        let mut fast = vec![0f32; n];
        Dct::new(n).inverse(&coeffs, &mut fast);
        for (i, f) in fast.iter().enumerate() {
            let slow: f64 = (0..n).map(|k| coeffs[k] as f64 * (std::f64::consts::PI * k as f64 * (2 * i + 1) as f64 / (2 * n) as f64).cos()).sum::<f64>() * 2.0 / n as f64;
            assert!((slow - *f as f64).abs() < 1e-3, "sample {i}: {slow} versus {f}");
        }
    }

    #[test]
    fn float_format() {
        // exponent 23, mantissa 5, minus sign: -5
        let v: u32 = 23 | 5 << 5 | 1 << 28;
        let bytes = v.to_le_bytes();
        assert_eq!(get_float(&mut Bits::new(&bytes)), -5.0);
    }
}
