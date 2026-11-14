# Il ricompilatore (`acrek`)

Stato al 2026-11-14, primo giorno della Fase 1 (aggiornato dopo le atomiche). Codice in `crates/` (Rust) e `runtime/` (C).

```sh
source ~/.bash_exports && cargo build --release
A=target/release/acrek; E="$AC1/AssassinsCreed_Dx9.exe"
$A info $E                         # hash, sezioni, import
$A rtti $E                         # conteggi RTTI e vtable
$A discover $E [--strict] [--out private/functions.json]
$A translate $E --out build/gen/x.c 0xFA6790 ...   # traduce in C le funzioni e ciò che chiamano
$A coverage $E                     # quante funzioni si traducono, e cosa blocca le altre
$A candidates $E --max 500         # funzioni adatte al test differenziale
.venv/bin/python tools/difftest.py $E              # scenari scritti a mano
.venv/bin/python tools/difftest.py $E --auto 4000  # campione automatico, circa 50 secondi
```

`acrek` rifiuta qualunque eseguibile che non abbia lo SHA-1 della build Steam DX9
(`--any-exe` per forzare): è la regola 4 del progetto, applicata nel codice.

## Struttura

| Percorso | Cosa |
|---|---|
| `crates/pe` | Parser PE32: sezioni, import con celle IAT, export, callback TLS, SHA-1 |
| `crates/recomp/src/rtti.rs` | RTTI e vtable (porting di `tools/rtti.py`) |
| `crates/recomp/src/discover.rs` | Scoperta delle funzioni (porting di `tools/functions.py`) |
| `crates/recomp/src/translate.rs` | Decodifica di una funzione e traduzione in C |
| `runtime/acrek.h`, `acrek.c` | Supporto del codice generato: contesto, memoria guest, flag, registro delle funzioni, trap |
| `tools/difftest.py` | Test differenziale contro `tools/harness.py` |

## Scoperta delle funzioni in Rust

Stessi conteggi RTTI degli script Python (4.376 / 4.597 / 59.899 / 28.717). La scoperta impiega
0,35 secondi invece di 30 e dà **116.090 funzioni, 3.222.078 istruzioni** (97.045 con `--strict`).
Rispetto al prototipo:

- il numero di voci di una jump table viene dal `cmp x, N; ja` che la precede, comprese le
  tabelle a due livelli (tabella di byte più tabella di indirizzi): 497 tabelle su 641;
- le anomalie nel codice certo scendono da 16 a 7.

Restano i limiti 2–5 di `funzioni.md` (funzioni che non ritornano, confini, tabelle delle
eccezioni).

## Come è fatto il codice generato

Ogni funzione guest diventa `void f_XXXXXXXX(Ctx *c)`.

- **Registri** nel contesto (`c->eax`…); i registri a 8 e 16 bit sono maschere su quelli a 32.
- **Memoria guest**: `rd8/16/32(c, indirizzo)` e `wr…`, cioè `c->mem + indirizzo`. Il runtime
  riserva 4 GB di spazio virtuale, così ogni indirizzo guest è valido senza controlli.
  `fs:` somma `c->fs_base` (l'indirizzo guest del TEB).
- **Flag**: sei variabili locali (`cf zf sf of pf af`) calcolate dopo ogni istruzione che li
  scrive. Il compilatore C elimina quelle che nessuno legge. Tra una funzione e l'altra i flag
  non passano.
- **Salti interni**: `goto`. **Jump table**: uno `switch` sugli indirizzi di destinazione, che
  il codice calcola leggendo la tabella dalla memoria guest come fa l'originale.
- **Chiamate**: una `call` scrive il vero indirizzo di ritorno sullo stack guest e chiama la
  funzione C; `ret` lo toglie. Lo stack guest resta così identico byte per byte all'originale.
  Un salto verso un'altra funzione è una tail call (`f_x(c); return;`).
- **Chiamate indirette** e funzioni non presenti nel file: `acrek_call_indirect(c, indirizzo)`,
  che cerca nel registro delle funzioni tradotte e altrimenti passa a `c->host_call` (import e
  funzioni host). Se nessuno gestisce l'indirizzo scatta una trap.
- **Trap** (`acrek_trap`): `int3`, divisioni non valide, destinazioni sconosciute. Riportano il
  controllo a `acrek_run` con motivo e indirizzo, senza abortire il processo.
- Ogni istruzione tradotta porta in commento indirizzo e testo dell'originale.

Regole di decodifica di una funzione:

- un salto verso una partenza **certa** (entry, export, vtable, destinazione di `call`) è una
  tail call; le partenze deboli restano codice interno, al più duplicato in due funzioni;
- se il corpo contiene codice a indirizzi più bassi dell'entry, la funzione inizia con un
  `goto` all'entry (insidia n. 1 di scimitar-recomp);
- jump table: forma semplice, a due livelli, indicizzate da 1, con indici negativi (dopo `neg`)
  e con base spostata. Le ultime tre sono quelle del `memcpy` di MSVC.

## Istruzioni supportate

Intere: `mov movzx movsx lea push pop leave xchg`, `add adc sub sbb cmp and or xor test inc dec
neg not`, `shl shr sar rol ror rcl rcr shld shrd`, `imul` (1, 2, 3 operandi), `mul div idiv`
(8, 16, 32 bit), `cdq cwde cbw cwd bswap`, `bt bts btr btc bsf bsr`, tutti i `jcc`, `jecxz`,
`loop`, `setcc`, `cmovcc`, `jmp`/`call` diretti e indiretti, `ret`/`ret n`, `movs stos lods scas
cmps` con `rep`/`repe`/`repne` e flag di direzione, `cld std clc stc cmc`, `pushfd popfd sahf
lahf`, `xadd cmpxchg`, `nop pause wait prefetch*`.

Atomiche: prefisso `lock` su `add sub and or xor inc dec not xadd cmpxchg bts btr btc`, e `xchg`
con un operando in memoria. Diventano operazioni atomiche vere sulla memoria host
(`__atomic_*`, ordinamento sequenziale), non una simulazione.

Non deterministiche: `rdtsc` è un contatore a 1 GHz dal tempo host; `cpuid` descrive una CPU
fissa di classe SSE3 (Core 2), uguale su ogni host. Le funzioni che le usano sono escluse dal
test differenziale.

## Copertura (`acrek coverage`)

| | Solo intere (mattina) | Con atomiche e resto delle intere |
|---|---|---|
| Funzioni che si traducono per intero | 99.735 (85,9%) | **103.848 su 116.090 (89,5%)** |
| Istruzioni in quelle funzioni, contate per funzione | 59,5% | **2.464.880 su 3.787.598 (65,1%)** |

"Si traduce" significa che il traduttore emette C per ogni istruzione, **non** che il risultato
sia verificato: per quello c'è il test differenziale qui sotto.

Le funzioni che restano sono poche ma grandi (il 10% delle funzioni contiene il 35% delle
istruzioni) e sono tutte virgola mobile. Primo ostacolo di ciascuna:

| Funzioni | Ostacolo |
|---|---|
| circa 7.800 | SSE: `movaps` 2.675, `movss` 2.487, `xorps` 2.261, `movq` 195, `movsd` 72, `pxor` 59… |
| circa 4.400 | x87: `fld` 2.983, `fldz` 510, `fild` 368, `fstp` 252, `fld1` 125, `fnstcw` 38… |

## Test differenziale (`tools/difftest.py`)

Traduce le funzioni, le compila con clang in una libreria, poi esegue ogni caso due volte con
gli stessi ingressi: sull'originale in Unicorn e sul codice tradotto, in memorie guest
identiche. Confronta i sette registri generali, i byte tolti dallo stack, gli import chiamati
e il contenuto finale di heap di prova, stack e sezioni scrivibili dell'immagine.

### Scenari scritti a mano

| Scenario | Chiamate | Esito |
|---|---|---|
| `strlen` (`0xFA6790`), contenuti e allineamenti casuali | 300 | identico |
| `strcmp` (`0xFA9760`), stringhe uguali, diverse, troncate | 300 | identico |
| `memcpy` (`0xF9FB80`), ogni lunghezza e allineamento, più regioni sovrapposte (copia all'indietro con `std`) | 1.117 | identico |

Bug trovati e corretti: tre forme di jump table di `memcpy` che il traduttore non riconosceva
(base spostata, indice da 1, indice negativo). `memcpy` chiama una variante SSE2 (`0xFAFBBC`)
per le copie grandi e allineate: non è ancora traducibile, e gli scenari non la raggiungono.

### Campione automatico (`--auto N`)

`acrek candidates` sceglie le funzioni adatte: tutta la chiusura sulle chiamate dirette è
traducibile, piccola (al più 12 funzioni e 3.000 istruzioni), senza import, senza chiamate o
salti indiretti a destinazione ignota, senza `rdtsc`/`cpuid`. Sono 14.196 su 103.848. Ne prende
prima alcune per ogni tipo di istruzione, a partire dai più rari, poi le altre in ordine
pseudo-casuale ripetibile.

Per ogni funzione, 8 casi con ingressi generati: sei buffer pieni di puntatori ad altri buffer,
interi piccoli, float e valori casuali; argomenti, `ecx` ed `edx` scelti tra puntatori e interi.
Se l'originale non gira con quegli ingressi (accesso a memoria non mappata, limite di 300.000
istruzioni) il caso è saltato.

| | |
|---|---|
| Funzioni provate | 4.000 |
| **Identiche all'originale** in tutti i casi eseguiti | **1.753** |
| Diverse | 0 |
| Saltate (nessun caso eseguibile con ingressi casuali) | 2.247 |
| Funzioni eseguite in casi identici, comprese quelle chiamate | 2.101 |
| Tempo | 49 secondi |

Nelle funzioni verificate compaiono 73 tipi di istruzione, tra cui 25 con prefisso `lock`
(`xadd` 22, `cmpxchg` 3), `shrd` 11, `shld` 5, `rcr` 8, `mul` 26, `div` 16, `idiv` 10, `sbb` 60,
`adc` 14, `rol` 62, `ror` 17.

**Tradotte ma non ancora verificate**, perché compaiono solo in funzioni che usano la virgola
mobile o gli import: `rcl`, `bts`/`btr`/`btc`, `bsf`/`bsr`, `pushfd`/`popfd`, `sahf`/`lahf`,
`cmpsb`, `lodsb`. Vanno coperte con scenari mirati o quando SSE e x87 saranno tradotti.

Limiti del campione: comparire in una funzione verificata non garantisce che il caso sia
passato da quell'istruzione; le funzioni saltate (più di metà) richiedono ingressi strutturati
che il generatore non sa costruire.

Le prime differenze segnalate dal test automatico non erano bug del traduttore ma memoria
residua tra un caso e l'altro: ora ogni caso parte da heap, stack e dati globali ripristinati.

## Limiti noti

- Flag non confrontati direttamente dal test (solo attraverso i loro effetti).
- Niente virgola mobile (SSE, x87, MMX): vedi copertura.
- Le atomiche usano puntatori host non allineati: va bene su x86-64, da rivedere su ARM e wasm.
- I registri sono letti e scritti nel contesto a ogni istruzione: corretto ma lento.
  L'ottimizzazione (registri in variabili locali) viene dopo la correttezza.
- Eccezioni C++ e SEH: non gestite.
- Un solo file C per traduzione: per l'immagine intera servirà dividere in parti.
