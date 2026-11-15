# The Bink decoder (`crates/bink`)

Since 2026-11-15. It replaces `ffmpeg` in the playback of the game's videos: pure Rust,
no dependencies, no `unsafe`, so usable in the browser too (Phase 5).

## What is in the files (verified)

Five videos in `videos\`, all **`BIKi`** (Bink 1, revision `i`), 1280×720, with neither alpha
channel nor grayscale (video flags 0):

| File | Frames | Frame rate | Audio tracks |
|---|---|---|---|
| `DemoIntro.bik` | 3,020 | 29.97 | 4 |
| `UBI_LOGO.bik` | 257 | 29.97 | 4 |
| `LoadingScreen.bik` | 100 | 15 | 0 |
| `Ubisoft_Presents.bik` | 2 | 15 | 0 |
| `Warning_Disclaimer.bik` | 2 | 15 | 0 |

The four tracks are a split 5.1, all "Bink audio DCT": front (48 kHz stereo), centre
(48 kHz mono), bass (2 kHz mono), rear (48 kHz stereo).

## Sources and rule followed

The format is described on MultimediaWiki and implemented in FFmpeg's `libavcodec` (LGPL).
The FFmpeg n6.1 sources are in `ref/ffmpeg-bink/` (ignored by git): **read as a
specification, not copied**, as for scimitar-recomp. The exception is the format's tables
(`src/tables.rs`: scan order, sixteen Huffman codes, sixteen orders of the run
blocks, quantisation matrices), which are data and not code: a script converted them from
`binkdata.h` and the file states its provenance. → `sources.md`, `decisions.md`.

## Structure

| File | Content |
|---|---|
| `container.rs` | header, tracks, frame index; one frame = one audio packet per track + the video |
| `bits.rs` | bit reader, least significant bit first |
| `video.rs` | the nine bundles of values per row, the ten 8×8 block types, DCT coefficients and residues by bit planes, integer inverse transform |
| `audio.rs` | DCT blocks with per-critical-band quantisers, one-sixteenth overlap, fast DCT-III via FFT |
| `main.rs` | `altair-bink info | video | audio` |

Things about the format to remember:

- From revision `h` the planes arrive in the order Y, Cr, Cb; every plane ends at a multiple
  of 32 bits; from `i` the video packet begins with 32 bits to skip.
- The block values do not sit next to the blocks: at the start of every row the stream carries,
  for each of the nine "bundles", as many values as are needed; the blocks consume them.
- The inverse transform writes the pixels truncating to 8 bits, without saturating, and the residues are
  added with wrap-around modulo 256: reproducing this is necessary for byte-exact identity.
- Audio: the inverse DCT is `x[n] = (2/N)·(c[0] + Σ c[k]·cos(πk(2n+1)/2N))`. I found the 2/N
  factor by comparing with the reference: without it, the signal was N/2 times larger.

## Verification: `tools/binktest.py`

`ffmpeg` acts as the oracle, like Unicorn for the translator.

| What | Result |
|---|---|
| Video, all five files | **3,381 frames out of 3,381 byte-for-byte identical** (yuv420p) |
| Audio, eight tracks | same number of samples; maximum error 3.9 × 10⁻⁷ on samples in [-1, 1] |
| Speed | 3,020 frames at 720p compared in 9 s, ffmpeg included |
| Unit tests | 7 (`cargo test -p altair-bink`) |

```sh
cargo build --release -p altair-bink
python3 -I tools/binktest.py "$AC1/videos"
```

In the game: logos and intro video are shown with our own decoder, no
`ffmpeg` process running, the videos' audio present in the mix, zero blocks
dropped. Screenshot in `run/shots/intro-decodificatore-nostro.png`.

## Limits

- Only what these files need: revisions `f`-`i` (only `i` tested), no alpha,
  no revisions `b` and `k`, no Bink 2, no RDFT audio.
- The bass track (2 kHz) is not played: the mixer works at 48 kHz and a
  resampler for it is missing.
- The audio is not bit-identical to `ffmpeg` (floating point, different FFT): the difference is not audible.
