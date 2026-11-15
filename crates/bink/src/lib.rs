//! Bink 1 decoder (video and audio), written for the videos of Assassin's Creed (2007):
//! `BIKi` files, no alpha channel, "Bink audio DCT" audio.
//!
//! It is a rewrite based on the public description of the format (MultimediaWiki) and
//! on reading FFmpeg's decoder as a specification; not a single line is copied from it.
//! FFmpeg is also the oracle: `tools/binktest.py` compares every frame with its own.
//!
//! No dependencies, no `unsafe`: it must be able to run in the browser too.

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
    /// The file is not a Bink we know how to read.
    Unsupported(String),
    /// Invalid data inside a frame.
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
            Error::Invalid(m) => write!(f, "invalid Bink data: {m}"),
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;
