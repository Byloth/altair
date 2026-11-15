//! Decodificatore Bink 1 (video e audio), scritto per i filmati di Assassin's Creed (2007):
//! file `BIKi`, senza canale alfa, audio "Bink audio DCT".
//!
//! È una riscrittura a partire dalla descrizione pubblica del formato (MultimediaWiki) e
//! dalla lettura, come specifica, del decodificatore di FFmpeg; nessuna riga ne è copiata.
//! FFmpeg è anche l'oracolo: `tools/binktest.py` confronta ogni fotogramma con il suo.
//!
//! Nessuna dipendenza, nessun `unsafe`: deve poter girare anche nel browser.

mod audio;
mod bits;
mod container;
mod tables;
mod video;

pub use audio::AudioDecoder;
pub use container::{AudioTrack, Bink, Packet};
pub use video::{Frame, VideoDecoder};

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// Il file non è un Bink che sappiamo leggere.
    Unsupported(String),
    /// Dati non validi dentro un fotogramma.
    Invalid(&'static str),
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Unsupported(m) => write!(f, "{m}"),
            Error::Invalid(m) => write!(f, "dati Bink non validi: {m}"),
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
