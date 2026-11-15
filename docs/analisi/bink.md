# Il decodificatore Bink (`crates/bink`)

Dal 2026-11-15. Sostituisce `ffmpeg` nella riproduzione dei filmati del gioco: Rust puro,
nessuna dipendenza, nessun `unsafe`, quindi utilizzabile anche nel browser (Fase 5).

## Cosa c'è nei file (verificato)

Cinque filmati in `videos\`, tutti **`BIKi`** (Bink 1, revisione `i`), 1280×720, senza canale
alfa né scala di grigi (flag video 0):

| File | Fotogrammi | Cadenza | Tracce audio |
|---|---|---|---|
| `DemoIntro.bik` | 3.020 | 29,97 | 4 |
| `UBI_LOGO.bik` | 257 | 29,97 | 4 |
| `LoadingScreen.bik` | 100 | 15 | 0 |
| `Ubisoft_Presents.bik` | 2 | 15 | 0 |
| `Warning_Disclaimer.bik` | 2 | 15 | 0 |

Le quattro tracce sono un 5.1 separato, tutte "Bink audio DCT": fronte (48 kHz stereo), centro
(48 kHz mono), bassi (2 kHz mono), retro (48 kHz stereo).

## Fonti e regola seguita

Il formato è descritto su MultimediaWiki e implementato in `libavcodec` di FFmpeg (LGPL).
I sorgenti di FFmpeg n6.1 sono in `ref/ffmpeg-bink/` (ignorata da git): **letti come
specifica, non copiati**, come per scimitar-recomp. Fanno eccezione le tabelle del formato
(`src/tables.rs`: ordine di scansione, sedici codici di Huffman, sedici ordini dei blocchi a
serie, matrici di quantizzazione), che sono dati e non codice: uno script le ha convertite da
`binkdata.h` e il file dichiara la provenienza. → `fonti.md`, `decisioni.md`.

## Struttura

| File | Contenuto |
|---|---|
| `container.rs` | intestazione, tracce, indice dei fotogrammi; un fotogramma = un pacchetto audio per traccia + il video |
| `bits.rs` | lettore di bit dal meno significativo |
| `video.rs` | i nove fasci di valori per riga, i dieci tipi di blocco 8×8, coefficienti DCT e residui per piani di bit, trasformata inversa intera |
| `audio.rs` | blocchi DCT con quantizzatori per banda critica, sovrapposizione di un sedicesimo, DCT-III veloce via FFT |
| `main.rs` | `altair-bink info | video | audio` |

Cose del formato da ricordare:

- Dalla revisione `h` i piani arrivano nell'ordine Y, Cr, Cb; ogni piano finisce a un multiplo
  di 32 bit; dalla `i` il pacchetto video comincia con 32 bit da saltare.
- I valori dei blocchi non stanno accanto ai blocchi: all'inizio di ogni riga il flusso porta,
  per ciascuno dei nove "fasci", quanti valori servono; i blocchi li consumano.
- La trasformata inversa scrive i pixel troncando a 8 bit, senza saturare, e i residui si
  sommano con riporto modulo 256: riprodurlo è necessario per l'identità al byte.
- Audio: la DCT inversa è `x[n] = (2/N)·(c[0] + Σ c[k]·cos(πk(2n+1)/2N))`. Il fattore 2/N l'ho
  trovato confrontando con il riferimento: senza, il segnale era N/2 volte più grande.

## Verifica: `tools/binktest.py`

`ffmpeg` fa da oracolo, come Unicorn per il traduttore.

| Cosa | Risultato |
|---|---|
| Video, tutti e cinque i file | **3.381 fotogrammi su 3.381 identici byte per byte** (yuv420p) |
| Audio, otto tracce | stesso numero di campioni; errore massimo 3,9 × 10⁻⁷ su campioni in [-1, 1] |
| Velocità | 3.020 fotogrammi a 720p confrontati in 9 s, ffmpeg compreso |
| Test unitari | 7 (`cargo test -p altair-bink`) |

```sh
cargo build --release -p altair-bink
python3 -I tools/binktest.py "$AC1/videos"
```

Nel gioco: loghi e filmato introduttivo si vedono con il decodificatore nostro, nessun
processo `ffmpeg` in esecuzione, audio dei filmati presente nel miscelato, zero blocchi
scartati. Schermata in `run/scatti/intro-decodificatore-nostro.png`.

## Limiti

- Solo ciò che serve a questi file: revisioni `f`-`i` (provata solo la `i`), niente alfa,
  niente revisioni `b` e `k`, niente Bink 2, niente audio RDFT.
- La traccia dei bassi (2 kHz) non viene suonata: il mixer lavora a 48 kHz e manca un
  ricampionatore per quella.
- L'audio non è identico al bit a `ffmpeg` (virgola mobile, FFT diversa): differenza non udibile.
