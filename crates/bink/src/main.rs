//! `altair-bink`: test tool for the decoder. Its output is game content: it does not
//! go into the repository (comparisons are done in memory or in `build/`).

use altair_bink::{AudioDecoder, Bink, VideoDecoder};
use std::io::Write;

const USAGE: &str = "usage: altair-bink info  <file.bik>              header, tracks, index
       altair-bink video <file.bik> [N]          the first N frames (all) as yuv420p on stdout
       altair-bink audio <file.bik> <track>      the track as interleaved little-endian f32 on stdout";

fn run() -> altair_bink::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(cmd), Some(path)) = (args.first(), args.get(1)) else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    let mut bink = Bink::open(std::io::BufReader::new(std::fs::File::open(path)?))?;
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    match cmd.as_str() {
        "info" => {
            writeln!(out, "revision {} {}x{} {} frames {}/{} fps flags {:#x}", bink.revision as char, bink.width, bink.height, bink.frames(), bink.fps_num, bink.fps_den, bink.video_flags)?;
            for (i, t) in bink.tracks.iter().enumerate() {
                writeln!(out, "track {i}: {} Hz, {} channels, {}, id {}", t.sample_rate, t.channels, if t.dct { "DCT" } else { "RDFT" }, t.id)?;
            }
            let keys = (0..bink.frames()).filter(|f| bink.frame_info(*f).is_some_and(|i| i.1)).count();
            let bytes: u64 = (0..bink.frames()).filter_map(|f| bink.frame_info(f)).map(|i| i.0 as u64).sum();
            writeln!(out, "keyframes {keys}, data {bytes} bytes")?;
        }
        "video" => {
            let limit = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(u32::MAX).min(bink.frames());
            let mut decoder = VideoDecoder::new(bink.width, bink.height, bink.revision);
            let mut buf = Vec::new();
            for f in 0..limit {
                let packet = bink.packet(f)?;
                let frame = decoder.decode(&packet.video).map_err(|e| {
                    eprintln!("frame {f}");
                    e
                })?;
                buf.clear();
                frame.write_yuv420(&mut buf);
                out.write_all(&buf)?;
            }
        }
        "audio" => {
            let track: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
            let info = *bink.tracks.get(track).ok_or(altair_bink::Error::Unsupported("no such track".into()))?;
            let mut decoder = AudioDecoder::new(&info)?;
            let mut samples = Vec::new();
            for f in 0..bink.frames() {
                let packet = bink.packet(f)?;
                samples.clear();
                decoder.decode(&packet.audio[track], &mut samples)?;
                for s in &samples {
                    out.write_all(&s.to_le_bytes())?;
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
