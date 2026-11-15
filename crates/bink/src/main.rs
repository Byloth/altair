//! `altair-bink`: strumento di prova del decodificatore. L'uscita è contenuto del gioco: non
//! va nel repository (i confronti si fanno in memoria o in `build/`).

use altair_bink::{AudioDecoder, Bink, VideoDecoder};
use std::io::Write;

const USAGE: &str = "uso: altair-bink info  <file.bik>              intestazione, tracce, indice
     altair-bink video <file.bik> [N]          i primi N fotogrammi (tutti) in yuv420p su stdout
     altair-bink audio <file.bik> <traccia>    la traccia in f32 little-endian intercalato su stdout";

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
            writeln!(out, "revisione {} {}x{} {} fotogrammi {}/{} fps flag {:#x}", bink.revision as char, bink.width, bink.height, bink.frames(), bink.fps_num, bink.fps_den, bink.video_flags)?;
            for (i, t) in bink.tracks.iter().enumerate() {
                writeln!(out, "traccia {i}: {} Hz, {} canali, {}, id {}", t.sample_rate, t.channels, if t.dct { "DCT" } else { "RDFT" }, t.id)?;
            }
            let keys = (0..bink.frames()).filter(|f| bink.frame_info(*f).is_some_and(|i| i.1)).count();
            let bytes: u64 = (0..bink.frames()).filter_map(|f| bink.frame_info(f)).map(|i| i.0 as u64).sum();
            writeln!(out, "fotogrammi chiave {keys}, dati {bytes} byte")?;
        }
        "video" => {
            let limit = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(u32::MAX).min(bink.frames());
            let mut decoder = VideoDecoder::new(bink.width, bink.height, bink.revision);
            let mut buf = Vec::new();
            for f in 0..limit {
                let packet = bink.packet(f)?;
                let frame = decoder.decode(&packet.video).map_err(|e| {
                    eprintln!("fotogramma {f}");
                    e
                })?;
                buf.clear();
                frame.write_yuv420(&mut buf);
                out.write_all(&buf)?;
            }
        }
        "audio" => {
            let track: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(0);
            let info = *bink.tracks.get(track).ok_or(altair_bink::Error::Unsupported("traccia inesistente".into()))?;
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
        eprintln!("errore: {e}");
        std::process::exit(1);
    }
}
