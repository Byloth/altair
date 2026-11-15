# Archivi `.forge`

Verificato il 2026-11-14 con `tools/forge.py` sui 18 archivi della build Steam (6,45 GB, 15.906
voci). Le liste complete di nomi e oggetti sono contenuti del gioco e stanno in `private/`.

```sh
python3 -I tools/forge.py summary "$AC1"
python3 -I tools/forge.py list "$AC1/DataPC.forge"
python3 -I tools/forge.py classes "$AC1/DataPC.forge" private/rtti-dx9.json
```

## La scoperta principale

**Ogni oggetto negli archivi dichiara la propria classe con il CRC-32 del nome, e quel nome è
lo stesso che compare nell'RTTI dell'eseguibile.** È il CRC-32 standard (quello di zlib)
calcolato sul nome breve della classe, senza namespace: `crc32("ShaderPermutation")` dà
`0x9467F2BB`, come riportato da scimitar-recomp.

Incrociando i CRC con `private/rtti-dx9.json`, **il 100% degli oggetti di primo livello ha una
classe riconosciuta**: 35.020 su 35.020 in `DataPC.forge`, 7.245 su 7.245 in
`DataPC_Assassination_Talal.forge`, 806 su 806 in `DataPC_Extra.forge`.

Conseguenza: per ogni tipo di dato del gioco sappiamo quale classe C++ lo carica, e di quella
classe abbiamo vtable e funzioni virtuali (`rtti.md`). Dati e codice si possono studiare insieme.

## Formato del contenitore (versione 25)

Tutti i valori sono little-endian e non allineati.

| Struttura | Campi |
|---|---|
| Header | `char[9]` `"scimitar\0"`, `u32` versione (25), `u64` offset di HeaderFat, `u32`, `u8` |
| HeaderFat | `u32` numero di file, `u32`, `u64`, `u64` (−1), `u32` max file per tabella, `u32` numero di tabelle, `u64` offset della prima tabella |
| Tabella | `u32` voci, `u32`, `u64` offset delle voci, `u64` tabella successiva (−1 = fine), `u32`, `u32`, `u64` offset della tabella dei nomi, `u64` |
| Voce (16 byte) | `u64` offset del record, `u32` id, `u32` dimensione dei dati |
| Record | `"FILEDATA"`, `char[128]` nome, altri campi; **0x1B8 byte** in tutto, poi i dati |

I dati di una voce sono grezzi (`GlobalMetaFile`, i suoni in streaming) oppure **due sezioni
compresse**, senza la parola iniziale che c'è in AC2 (è la differenza di 4 byte notata da
scimitar-recomp).

| Sezione compressa | Campi |
|---|---|
| Intestazione | `u64` marcatore `33 AA FB 57 99 FA 04 10`, `u16` versione (1), `u8` codec, `u16` (0x8000), `u16` (0x8000), `u16` numero di blocchi |
| Tabella dei blocchi | per blocco: `u16` byte decompressi, `u16` byte su disco |
| Blocchi | per blocco: `u32` checksum, poi i dati; compressi se su disco sono meno dei decompressi |

Il codec è sempre **1, LZO1X** (compresso con la variante 999; il decompressore è quello comune
a tutta la famiglia LZO1X). `forge.py` ne contiene uno in Python puro.

### Dentro una voce compressa: un pacchetto di oggetti

- **Prima sezione, l'indice:** `u16` numero di oggetti, poi per ciascuno `u32` id e `u32`
  dimensione.
- **Seconda sezione, i dati:** gli oggetti uno dopo l'altro. Ognuno inizia con `u32` CRC-32
  della classe, `u32` dimensione del corpo, `u32` lunghezza del nome, il nome, `u32` id,
  di nuovo `u32` CRC-32 della classe, poi i campi.

I campi sono serializzati in ordine, senza tag (formato "FastLoad"): per interpretarli serve
il codice della classe. Il checksum dei blocchi non è ancora identificato.

## Inventario degli archivi

| Archivio | Dimensione | Voci | Contenuto |
|---|---|---|---|
| `DataPC.forge` | 194 MB | 37 | avvio e dati globali: `Game Bootstrap Settings`, `Game Fix`, animazioni, missioni |
| `DataPC_Common.forge` | 667 MB | 1.715 | banche sonore (`BAO_0x…`) e dati comuni |
| `DataPC_Acre`, `_Damascus`, `_Jerusalem` | 775–809 MB | 869–1.387 | le tre città, divise in celle (`CellNNNNN_DataBlock`) |
| `DataPC_Kingdom`, `_Masyaf`, `_Arsuf` | 369–719 MB | 785–1.404 | il Regno, Masyaf, Arsuf, sempre a celle |
| `DataPC_Assassination_Talal` | 101 MB | 50 | una sola missione di assassinio ha un archivio suo |
| `DataPC_SolomonTemple`, `_Flashback_Paradise` | 197–245 MB | 34–122 | prologo e finale |
| `DataPC_Present_Room`, `_Memory_Corridor`, `_LoadingRoom2` | 10–286 MB | 23–90 | laboratorio Abstergo, corridoio della memoria, stanza di caricamento |
| `DataPC_Map_Menu` | 2 MB | 2 | il mondo del menu principale |
| `DataPC_Extra` | 24 MB | 404 | solo texture (403 `TextureMap` + 403 `TextureMapSpec`), tra cui quelle dei tutorial |
| `DataPC_StreamedSounds` | 450 MB | 477 | audio in streaming, non compresso |
| `DataPC_StreamedSoundsita` | 127 MB | 6.056 | voci in italiano, non compresse |

I mondi sono divisi in celle caricate in streaming: lo stesso schema del menu, dove un oggetto
`World` contiene `WorldArea`, `WorldAmbiance`, `LayeredSky` ed `Entity`.

## Classi negli archivi

`DataPC.forge` contiene 35.020 oggetti di 112 classi. Le più numerose: `SoundBao` 18.306,
`Animation` 12.387, `FX` 658, `TextureMap` 521, `Entity` 427, `Material` 188, `Mesh` 168,
`MaterialTemplate` 83 (102 MB: contiene gli shader), `AnimSet` 80, `Skeleton` 50.

`DataPC_Assassination_Talal.forge` contiene 7.245 oggetti di 56 classi: suoni, animazioni,
texture, mesh, più i dati del mondo (`GridCellDataBlock`, `GridPartition`, `RegionLayout`) e
pacchetti di comportamento (`CustomActionGamePlayPack`, `CustomActionReactionPack`,
`VocalActionPack`, `ActionBlock`).

## Le missioni: dati o codice?

**Entrambi: la struttura è nei dati, il comportamento è nel codice.**

In `DataPC.forge` ci sono circa 950 oggetti di 27 classi `MissionItem*`. I più numerosi:

| Oggetti | Classe | Cosa sembra essere |
|---|---|---|
| 339 | `MissionItemSceneSequencer` | sequenze di scena |
| 161 | `MissionItemList` | liste di elementi di missione |
| 91 | `MissionItemHighestPoint` | punti di osservazione |
| 85 | `MissionItemFreeMission` | missioni libere |
| 66 | `MissionItemVigilantes` | salvataggio dei cittadini |
| 22 | `MissionItemScholars` | studiosi |
| 11 / 10 / 10 | `MissionItemPickPocket`, `…EavesDropping`, `…Interrogation` | le tre investigazioni classiche |
| 8 / 4 / 4 / 4 / 3 | `MissionItemInformerAssassination`, `…Escort`, `…Flag`, `…FreeRun`, `…Kiosk` | le missioni degli informatori |
| 9 + 9 + 9 + 20 | `MissionItemPreBureau`, `…InitBureau`, `…WhileBureau`, `…PostBureau` | le fasi attorno all'ufficio degli Assassini |
| 15 | `MissionItemAlMualim` | i colloqui con Al Mualim |
| 8 | `MissionMemoryFile` | i blocchi di memoria |

Nel codice le classi `MissionItem*` sono 45 (comprese quelle di test rimaste nella build).

Che cosa implica per la Fase 6:

- **Riorganizzare le missioni esistenti è lavoro sui dati**: quali investigazioni servono, in
  che ordine, dove, con quali condizioni (`MissionItemList`, `MissionItemCountOr`). Non serve
  toccare il codice del gioco, serve saper leggere e riscrivere questi oggetti.
- **Un tipo di missione nuovo richiede codice**: ogni tipo è una classe C++. Si può però
  aggiungerla nel nostro runtime e registrarla presso il motore.
- Il prossimo passo, quando ci arriveremo, è decodificare i campi di `MissionItemList` e di un
  tipo semplice, come `MissionItemPickPocket`, leggendo la loro funzione di caricamento.

Nota: i nomi delle classi suggeriscono il ruolo, ma il significato dei campi non è verificato.

## Limiti

- Sono contati solo gli oggetti di primo livello di ogni pacchetto; quelli annidati no.
- Inventario delle classi fatto su 3 archivi su 18. Il decompressore in Python impiega quasi
  3 minuti su `DataPC.forge`: per l'inventario completo conviene riscriverlo in Rust (servirà
  comunque al runtime per i test).
- Campi senza nome nelle tabelle, tabella dei nomi e checksum dei blocchi: non decodificati.
- La funzione del motore che calcola il CRC non è stata trovata (`harness.md`); sappiamo però
  che il risultato è il CRC-32 standard.

## Lettore in Rust (`crates/forge`, 2026-11-15)

`altair_forge` (libreria: `Forge::open`, `entries`, `name`, `sections`, `decompress`, `raw`,
`lzo1x_decompress`, `crc32`) e il binario `altair-forge` (`list`, `digest`). È la riscrittura
di `tools/forge.py`, che resta il termine di confronto.

Verifica: `list` e `digest` (per voce: id, numero di sezioni, byte decompressi, CRC-32 dei
dati decompressi) **identici byte per byte** tra Python e Rust su `DataPC_Map_Menu` (2 voci),
`DataPC_LoadingRoom2` (23) e `DataPC_Extra` (404). `DataPC_Memory_Corridor`: 90 voci, 174 MB
decompressi in 1,5 s.

```sh
cargo build --release -p altair-forge
target/release/altair-forge digest "$AC1/DataPC_Extra.forge" > build/forge-check/rs.txt
python3 -I tools/forge.py digest "$AC1/DataPC_Extra.forge" > build/forge-check/py.txt
cmp build/forge-check/rs.txt build/forge-check/py.txt
```

Non ancora portati: l'indice degli oggetti dentro una voce e il riconoscimento delle classi
(`forge.py classes`).
