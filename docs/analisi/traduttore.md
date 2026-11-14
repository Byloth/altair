# Il ricompilatore (`acrek`)

Stato al 2026-11-14, primo giorno della Fase 1. Codice in `crates/` (Rust) e `runtime/` (C).

```sh
source ~/.bash_exports && cargo build --release
A=target/release/acrek; E="$AC1/AssassinsCreed_Dx9.exe"
$A info $E                         # hash, sezioni, import
$A rtti $E                         # conteggi RTTI e vtable
$A discover $E [--strict] [--out private/functions.json]
$A translate $E --out build/gen/x.c 0xFA6790 ...   # traduce in C le funzioni e ciò che chiamano
$A coverage $E                     # quante funzioni si traducono, e cosa blocca le altre
.venv/bin/python tools/difftest.py $E              # tradotto contro originale (Unicorn)
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

`mov movzx movsx lea push pop leave xchg`, `add adc sub sbb cmp and or xor test inc dec neg not`,
`shl shr sar rol ror`, `imul` (1, 2, 3 operandi), `mul div idiv` (32 bit), `cdq cwde cbw cwd
bswap bt`, tutti i `jcc`, `jecxz`, `loop`, `setcc`, `cmovcc`, `jmp`/`call` diretti e indiretti,
`ret`/`ret n`, `movs stos lods scas cmps` con `rep`/`repe`/`repne` e flag di direzione,
`cld std clc stc cmc`, `nop`.

## Copertura (`acrek coverage`)

| | |
|---|---|
| Funzioni che si traducono per intero | **99.735 su 116.090 (85,9%)** |
| Istruzioni in quelle funzioni | 2.253.121 su 3.787.598 (59,5%), contate per funzione |

"Si traduce" significa che il traduttore emette C per ogni istruzione: **non** che il risultato
sia stato verificato. Le funzioni verificate sono le tre del test differenziale.

Le funzioni non tradotte sono poche ma grandi (il 14% delle funzioni contiene il 40% delle
istruzioni). Primo ostacolo di ciascuna:

| Funzioni | Ostacolo |
|---|---|
| 4.162 | prefisso `lock` (`Interlocked*` in linea: `xadd`, `cmpxchg`, `inc`/`dec`) |
| circa 4.300 | x87: `fld` 2.858, `fldz` 499, `fild` 357, `fstp` 246, `fld1` 123… |
| circa 7.600 | SSE: `movaps` 2.562, `movss` 2.427, `xorps` 2.231, `movq` 188, `movsd` 71… |
| 163 | `rdtsc` |
| 127 | `shrd` / `shld` |
| circa 100 | varie: `stmxcsr`, `fnstcw`, `cpuid`, `rcr`, `pushfd`… |

Quindi l'ordine del lavoro è: `lock` e atomiche, SSE scalare e impacchettato, x87.

## Test differenziale (`tools/difftest.py`)

Traduce le funzioni degli scenari, le compila con clang in una libreria, poi esegue ogni
scenario due volte con gli stessi ingressi: sull'originale in Unicorn e sul codice tradotto,
in memorie guest identiche. Confronta per ogni chiamata `eax`, `ecx`, `edx`, i byte tolti dallo
stack e gli import chiamati; alla fine, heap di prova e stack.

| Scenario | Chiamate | Esito |
|---|---|---|
| `strlen` (`0xFA6790`), contenuti e allineamenti casuali | 300 | identico |
| `strcmp` (`0xFA9760`), stringhe uguali, diverse, troncate | 300 | identico |
| `memcpy` (`0xF9FB80`), ogni lunghezza e allineamento, più regioni sovrapposte (copia all'indietro con `std`) | 1.117 | identico |

Bug trovati dal test e corretti: tre forme di jump table di `memcpy` che il traduttore non
riconosceva (base spostata, indice da 1, indice negativo).

`memcpy` chiama una variante SSE2 (`0xFAFBBC`, usa `movdqa`) per le copie grandi e allineate:
non è ancora traducibile, e nessuno degli scenari la raggiunge.

## Limiti noti

- Flag non confrontati direttamente dal test (solo attraverso i loro effetti).
- Niente virgola mobile, SSE, atomiche: vedi copertura.
- I registri sono letti e scritti nel contesto a ogni istruzione: corretto ma lento.
  L'ottimizzazione (registri in variabili locali) viene dopo la correttezza.
- Eccezioni C++ e SEH: non gestite.
- Un solo file C per traduzione: per l'immagine intera servirà dividere in parti.
