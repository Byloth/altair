# Il ricompilatore (`altair`)

Stato al 2026-11-14, primo giorno della Fase 1 (aggiornato dopo x87). Codice in `crates/` (Rust) e `runtime/` (C).

```sh
source ~/.bash_exports && cargo build --release
A=target/release/altair; E="$AC1/AssassinsCreed_Dx9.exe"
$A info $E                         # hash, sezioni, import
$A rtti $E                         # conteggi RTTI e vtable
$A discover $E [--strict] [--out private/functions.json]
$A translate $E --out build/gen/x.c 0xFA6790 ...   # traduce in C le funzioni e ciò che chiamano
$A coverage $E [--list]            # quante funzioni si traducono, e cosa blocca le altre
$A census $E                       # ogni istruzione: quante volte compare, quante non traducibili
$A candidates $E --max 500         # funzioni adatte al test differenziale
.venv/bin/python tools/difftest.py $E              # scenari scritti a mano
.venv/bin/python tools/difftest.py $E --auto 8000  # campione automatico, circa 3 minuti
.venv/bin/python tools/difftrace.py $E 0xADDR 0    # rigioca un caso e mostra le differenze
```

`altair` rifiuta qualunque eseguibile che non abbia lo SHA-1 della build Steam DX9
(`--any-exe` per forzare): è la regola 4 del progetto, applicata nel codice.

## Struttura

| Percorso | Cosa |
|---|---|
| `crates/pe` | Parser PE32: sezioni, import con celle IAT, export, callback TLS, SHA-1 |
| `crates/recomp/src/rtti.rs` | RTTI e vtable (porting di `tools/rtti.py`) |
| `crates/recomp/src/discover.rs` | Scoperta delle funzioni (porting di `tools/functions.py`) |
| `crates/recomp/src/translate.rs` | Decodifica di una funzione e traduzione in C |
| `runtime/altair.h`, `altair.c` | Supporto del codice generato: contesto, memoria guest, flag, registro delle funzioni, trap |
| `runtime/altair_sse.h` | Operazioni SSE/SSE2 in C portabile |
| `runtime/altair_x87.h` | x87 su `long double` a 80 bit |
| `build/gen/full/` (generato) | L'immagine intera tradotta: vedi `immagine.md` |
| `tests/compiler/` | Riproduzioni di bug del compilatore C trovati dal test |
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
- **Chiamate indirette** e funzioni non presenti nel file: `altair_call_indirect(c, indirizzo)`,
  che cerca nel registro delle funzioni tradotte e altrimenti passa a `c->host_call` (import e
  funzioni host). Se nessuno gestisce l'indirizzo scatta una trap.
- **Trap** (`altair_trap`): `int3`, divisioni non valide, destinazioni sconosciute. Riportano il
  controllo a `altair_run` con motivo e indirizzo, senza abortire il processo.
- Ogni istruzione tradotta porta in commento indirizzo e testo dell'originale.

Regole di decodifica di una funzione:

- un salto verso una partenza **certa** (entry, export, vtable, destinazione di `call`) è una
  tail call; le partenze deboli restano codice interno, al più duplicato in due funzioni;
- se il corpo contiene codice a indirizzi più bassi dell'entry, la funzione inizia con un
  `goto` all'entry (insidia n. 1 di scimitar-recomp);
- jump table: forma semplice, a due livelli, indicizzate da 1, con indici negativi (dopo `neg`)
  e con base spostata. Le ultime tre sono quelle del `memcpy` di MSVC.

## Istruzioni supportate

**Intere:** `mov movzx movsx lea push pop leave xchg`, `add adc sub sbb cmp and or xor test inc
dec neg not`, `shl shr sar rol ror rcl rcr shld shrd`, `imul` (1, 2, 3 operandi), `mul div idiv`
(8, 16, 32 bit), `cdq cwde cbw cwd bswap`, `bt bts btr btc bsf bsr`, tutti i `jcc`, `jecxz`,
`loop`, `setcc`, `cmovcc`, `jmp`/`call` diretti e indiretti, `ret`/`ret n`, `movs stos lods scas
cmps` con `rep`/`repe`/`repne` e flag di direzione, `cld std clc stc cmc`, `pushfd popfd sahf
lahf`, `xadd cmpxchg`, `nop pause wait prefetch*`.

**Atomiche:** prefisso `lock` su `add sub and or xor inc dec not xadd cmpxchg bts btr btc`, e
`xchg` con un operando in memoria. Diventano operazioni atomiche vere sulla memoria host
(`__atomic_*`, ordinamento sequenziale).

**SSE e SSE2** (registri XMM; `runtime/altair_sse.h`):
- spostamenti: `movaps movups movapd movupd movdqa movdqu movss movsd movd movq movlps movlpd
  movhps movhpd movhlps movlhps movmskps movmskpd pmovmskb`;
- aritmetica impacchettata e scalare: `add sub mul div min max sqrt` in versione `ps pd ss sd`,
  `rsqrtps rsqrtss rcpps rcpss`;
- logica: `and andn or xor` in versione `ps`, `pd` e intera (`pand pandn por pxor`);
- confronti: `cmpps cmppd cmpss cmpsd` (otto predicati), `comiss ucomiss comisd ucomisd`;
- conversioni: `cvtss2sd cvtsd2ss cvtps2pd cvtpd2ps cvtsi2ss cvtsi2sd cvttss2si cvttsd2si
  cvtss2si cvtsd2si cvtdq2ps cvtps2dq cvttps2dq`;
- rimescolamenti: `shufps shufpd pshufd unpcklps unpckhps unpcklpd unpckhpd punpcklbw punpcklwd
  punpckldq punpckhdq punpcklqdq punpckhqdq pextrw pinsrw`;
- interi impacchettati: `padd psub` (`b w d q`), `pcmpeq` (`b w d`), `pcmpgt` (`w d`), `pmullw
  pmulhw pmuludq`, scorrimenti `psll psrl psra` (`w d q`), `pslldq psrldq`;
- controllo: `ldmxcsr stmxcsr`.

Le operazioni SSE sono scritte in C portabile, corsia per corsia. Su un host x86-64 il
compilatore le traduce nelle stesse istruzioni dell'originale; arrotondamento, flush-to-zero e
denormali seguono il guest perché durante l'esecuzione l'MXCSR dell'host è quello del guest.
`rsqrt` e `rcp` sono calcolati esatti, come nell'emulatore di riferimento (le CPU reali danno
12 bit).

**x87** (`runtime/altair_x87.h`): stack di otto registri a 80 bit.
- carichi e salvataggi: `fld fst fstp` (32, 64, 80 bit e registri), `fild fist fistp fisttp`
  (16, 32, 64 bit), `fxch`, `fcmovcc`, costanti `fld1 fldz fldpi fldl2t fldl2e fldlg2 fldln2`;
- aritmetica: `fadd fsub fsubr fmul fdiv fdivr` con le varianti `p` (pop) e `fi` (intero),
  `fchs fabs fsqrt frndint fscale fprem fprem1`;
- confronti: `fcom fcomp fcompp fucom fucomp fucompp ficom ficomp ftst fxam` (codici di
  condizione nella parola di stato) e `fcomi fcomip fucomi fucomip` (flag della CPU);
- trascendenti: `fsin fcos fsincos fptan fpatan fyl2x fyl2xp1 f2xm1`, tramite la libm;
- controllo: `fnstsw fnstcw fldcw fninit fnclex ffree fincstp fdecstp`.

I registri sono `long double`, che sugli host x86 è proprio il formato a 80 bit dell'x87: il
compilatore emette le stesse istruzioni dell'originale. Precisione (24, 53 o 64 bit) e
arrotondamento seguono il guest perché la parola di controllo x87 dell'host, durante
l'esecuzione guest, è quella del guest. Il gioco parte dalla parola di controllo di Windows,
`0x027F` (53 bit). Non sono modellati i tag dei registri né i flag di eccezione nella parola
di stato.

**Non deterministiche:** `rdtsc` è un contatore a 1 GHz dal tempo host; `cpuid` descrive una CPU
fissa di classe SSE3. Le funzioni che le usano sono escluse dal test differenziale, come
quelle con istruzioni x87 trascendenti (l'emulatore le calcola in doppia precisione).

## Copertura (`altair coverage`, `altair census`)

| | Solo intere | Con atomiche | Con SSE | **Con x87** |
|---|---|---|---|---|
| Funzioni che si traducono per intero | 85,9% | 89,5% | 94,3% | 116.048 su 116.090 (99,96%), poi **tutte** (vedi sotto) |
| Istruzioni in quelle funzioni, contate per funzione | 59,5% | 65,1% | 74,5% | **99,9%** |
| Istruzioni distinte non traducibili | — | 10,9% | 2,9% | **336 su 3.223.348** |

"Si traduce" significa che il traduttore emette C per ogni istruzione, **non** che il risultato
sia verificato: per quello c'è il test differenziale qui sotto.

### Le ultime 42 funzioni (2026-11-15)

Dopo x87 restavano 42 funzioni non traducibili. `altair coverage --list` le elenca con origine
e motivo. Erano di due tipi.

**17 non erano funzioni.** Erano dati presi per codice dalla scoperta:
- 14 tabelle di indici degli switch a due livelli. I loro byte (`00 01 02 03 04 05 06 07…`) si
  decodificano come `add`, `push es`, `pop es`, `pop ds`. Stanno subito dopo la tabella di
  indirizzi, a un indirizzo allineato, e il codice ne carica l'indirizzo come immediato;
- 3 costanti "tonde" nei dati (`0xD20000`, `0x1640000`, `0x113F9F0`).

La scoperta ora scarta le partenze che contengono `push`/`pop` di un registro di segmento,
`enter`, `clts`, `loope`/`loopne`. Le funzioni scendono da 116.090 a **116.077**.

**25 erano codice vero**, ora tradotto:

| Funzioni | Cosa | Dove |
|---|---|---|
| 13 | MMX: `movq movd emms pshufw punpck* pcmpgtw pmulhw padd* psub* pack* psll* psra* cvttps2pi` | mixer audio (`0x8146F0`…, `0x833DF0`…) e due a `0x1023410` |
| 3 | `mov [mem], ss` e altri registri di segmento | il CRT che cattura il contesto di un'eccezione |
| 2 | `xlatb` | CRT |
| 2 | `fnstenv`, `fnsave` (con `fldenv`, `frstor`) | CRT, gestione degli errori matematici |
| 5 | `bsr` a 16 bit, `bts` su stringa di bit, `cvtdq2pd`, `movntps` | varie |

I registri MM sono le mantisse dei registri x87 fisici, come sull'hardware; le operazioni
riusano quelle a corsie di SSE. I registri di segmento restituiscono i selettori di un processo
a 32 bit su Windows a 64 bit (`cs` 0x23, `ds`/`es`/`ss` 0x2B, `fs` 0x53).

**Ora `altair coverage` dà 116.077 funzioni su 116.077 e `altair census` zero istruzioni non
traducibili.**

Verifica di quelle 25 (`difftest.py --funcs …`, 40 casi ciascuna): 17 identiche, di cui 9 a
meno dell'esponente dei registri MMX (vedi sotto); 7 saltate perché non girano con ingressi
casuali (le tre del contesto d'eccezione, le due con `xlatb`, quella con `bsr` a 16 bit, una
MMX); 1 diversa, `0xFB3476`, solo nei campi dell'ambiente x87 scritti da `fnstenv` che non
modelliamo (tag, puntatori all'ultima istruzione, bit riservati).

## Test differenziale (`tools/difftest.py`)

Traduce le funzioni, le compila con clang in una libreria, poi esegue ogni caso due volte con
gli stessi ingressi: sull'originale in Unicorn e sul codice tradotto, in memorie guest
identiche. Confronta i sette registri generali, gli otto registri XMM, lo stato x87 (cima
dello stack e gli otto registri fisici a 80 bit), i byte tolti dallo stack, gli import
chiamati e il contenuto finale di heap di prova, stack e sezioni scrivibili dell'immagine.

### Scenari scritti a mano

| Scenario | Chiamate | Esito |
|---|---|---|
| `strlen` (`0xFA6790`), contenuti e allineamenti casuali | 300 | identico |
| `strcmp` (`0xFA9760`), stringhe uguali, diverse, troncate | 300 | identico |
| `memcpy` (`0xF9FB80`), ogni lunghezza e allineamento, più regioni sovrapposte (copia all'indietro con `std`) | 1.117 | identico |
| `floor` (`0xFA1120`), ramo x87: mezzi interi, zeri con segno, infiniti, valori casuali | 316 | identico (prima della correzione: diverso alla prima chiamata) |
| `cpuid` (`0xFCA495`), il CRT cerca SSE2 passando dal bit ID di EFLAGS | 1 | stesso verdetto |

### Campione automatico (`--auto N`)

`altair candidates` sceglie le funzioni adatte: tutta la chiusura sulle chiamate dirette è
traducibile, piccola (al più 12 funzioni e 3.000 istruzioni), senza import, senza chiamate o
salti indiretti a destinazione ignota, senza istruzioni non confrontabili. Sono 19.028 su
116.048. Ne prende prima alcune per ogni tipo di istruzione, a partire dai più rari, poi le
altre in ordine pseudo-casuale ripetibile.

Per ogni funzione, 8 casi con ingressi generati: sei buffer pieni di puntatori ad altri buffer,
interi piccoli, float (anche zeri con segno, infiniti, NaN, denormali, limiti degli interi) e
valori casuali; argomenti, `ecx` ed `edx` scelti tra puntatori e interi; registri XMM iniziali
casuali; x87 vuoto con parola di controllo `0x027F`. Se l'originale non gira con quegli
ingressi il caso è saltato.

| | |
|---|---|
| Funzioni provate | 8.000 |
| **Identiche all'originale** in tutti i casi eseguiti | **4.199** |
| di cui identiche a meno di un NaN o dell'esponente di un registro MMX | 9 |
| Diverse | **0** |
| Saltate (nessun caso eseguibile con ingressi casuali) | 3.801 |
| Funzioni eseguite in casi identici, comprese quelle chiamate | 4.935 |
| Istruzioni in quelle funzioni | 142.138, di 213 tipi |
| Tempo | circa 3 minuti |

Istruzioni x87 presenti nelle funzioni verificate: `fld` 3.051, `fstp` 2.305, `fmul` 545, `fxch`
285, `fadd` 275, `fnstsw` 250, `fmulp` 195, `fldcw` 176, `fsub` 170, `fst` 136, `faddp` 130,
`fild` 125, `fistp` 109, `fldz` 98, `fnstcw` 90, `fsubp` 86, `fcom` 81, `fcomip` 70, `fcomp` 67,
`fcompp` 60, `fld1` 58, `fdiv` 47, e altre 20 con conteggi minori. I 176 `fldcw` mostrano che
il cambio di arrotondamento (la sequenza di MSVC per convertire un float in intero) è esercitato.

Istruzioni SSE nelle funzioni verificate (campione precedente, da 6.000): `movaps` 5.142,
`movss` 4.395, `shufps` 1.208, `mulps` 1.173, `cvtps2pd` 892, `addps` 830, e altre 60.

**Tradotte ma assenti dalle funzioni verificate:** `fisubr`, `paddq`, `pmuludq`, `paddsw`,
`pshufw`, `punpckhbw`, `cvtpi2ps`, `xlatb`, `fldenv`, `frstor`, `mov` da registro di segmento, e tra
le intere `rcl`, `bts`/`btr`/`btc`, `bsf`/`bsr`, `pushfd`/`popfd`, `sahf`/`lahf`, `cmpsb`,
`lodsb`. **Tradotte ed escluse per scelta dal confronto:** le x87 trascendenti, `fprem`,
`fprem1`. Poche occorrenze (meno di 5): `frndint`, `fldpi`, `fucom`, `fiadd`, `fimul`, `fidiv`.

Limiti del campione: comparire in una funzione verificata non garantisce che il caso sia
passato da quell'istruzione; metà delle funzioni è saltata perché richiede ingressi
strutturati che il generatore non sa costruire; lo stack x87 parte sempre vuoto.

### Cosa ha trovato il test

1. **Tre forme di jump table di `memcpy`** non riconosciute (base spostata, indice da 1,
   indice negativo). Corretto nel traduttore.
2. **Un bug di clang 18.1.3.** `x = (x & 0xffffff00) | (u8)mem[x]`, con `x` appena caricato
   dalla memoria, viene compilato da `-O1` in su perdendo i 24 bit alti; gcc è corretto.
   Riproduzione in 20 righe: `tests/compiler/clang18-partial-register.c`. Il codice generato
   ora scrive i registri a 8 e 16 bit con scritture strette dirette (`SET8L`, `SET8H`, `SET16`).
   **Conseguenza: il test differenziale va rifatto sul codice compilato con il compilatore e
   le opzioni finali**, non solo sul traduttore.
3. **NaN.** Quando i due operandi di un'operazione SSE sono entrambi NaN, l'hardware x86
   restituisce il primo; l'emulatore (QEMU) applica la regola dell'x87. Il codice tradotto
   segue l'hardware. Il test tollera differenze in cui entrambe le parti sono NaN, e le conta
   a parte.
4. **Stato che l'emulatore si porta dietro tra un caso e l'altro**, e che falsava i confronti:
   memoria residua nello heap, flag di direzione rimasto impostato da una funzione interrotta,
   MXCSR (scriverlo dall'esterno non aggiorna arrotondamento e flush-to-zero interni: va
   eseguito un `ldmxcsr`). Ora ogni caso parte da uno stato ripristinato, e le sezioni di sola
   lettura sono protette nell'emulatore e controllate nel tradotto.

5. **MMX nell'emulatore.** Dopo un'istruzione MMX l'hardware porta a tutti uno l'esponente
   del registro x87 che la ospita; l'emulatore lo lascia invariato. Il tradotto segue
   l'hardware; il test tollera la differenza e la conta con quelle sui NaN.
6. **Codice che prosegue dentro un'altra funzione.** Dopo una `call`, le routine matematiche
   del CRT continuano in codice che è anche l'inizio di un'altra funzione. Il traduttore la
   trattava come tail call senza includerla tra le funzioni da tradurre. Corretto.
7. **`frndint` e `fistp` ignoravano la modalità di arrotondamento x87** (2026-11-15, trovato
   dall'audio che gracchiava, vedi `runtime.md`). Erano tradotti con `rintl`, contando sul
   fatto che la parola di controllo x87 dell'host è quella del guest. Ma quando l'operando
   nasce da un `double` (`fld qword; frndint; fstp qword`), clang riduce
   `(double)rintl((long double)x)` a `rint(x)`: una chiamata in doppia precisione, che arrotonda
   secondo **MXCSR** e non secondo la parola di controllo x87. Il `floor` del CRT (ramo x87,
   `0xFB2B93`: mette RC verso il basso con `_controlfp`, poi `frndint` in `0xFCA5FE`)
   restituiva così `floor(11,5) = 12` e `floor(3,5) = 4`. Verificato con `objdump` sulla
   libreria (`call rint@plt`) e con il nuovo scenario `floor`. Ora `x87_rnd` (in
   `runtime/altair_x87.h`) arrotonda leggendo il campo RC di `c->fpcw` e usando solo `floorl`,
   `ceill`, `truncl`, che non dipendono da alcuna modalità. Il campione automatico non l'aveva
   visto: genera ingressi a caso e non imposta mai una modalità diversa dal più vicino.
8. **Il bit ID di EFLAGS andava perso.** Il CRT (`0xFCA495`) decide se usare i rami SSE2 delle
   funzioni matematiche invertendo il bit 21 con `pushfd`/`popfd` e rileggendolo, come si
   faceva per sapere se la CPU ha `cpuid`. `SET_EFLAGS` lo scartava: il verdetto era "niente
   SSE2" e tutto il CRT prendeva i rami x87, al contrario di qualunque PC vero. Ora il bit
   vive in `Ctx.eflags_id` (occupa quattro byte che erano di riempimento: la dimensione di
   `Ctx` non cambia). Scenario `cpuid`, che confronta solo il verdetto.

`tools/difftrace.py <exe> 0xADDR <caso>` rigioca un caso sui due motori e mostra indirizzi
eseguiti, registri e prime differenze di memoria.

## Agganci (2026-11-15)

Per osservare o cambiare il comportamento interno del gioco senza toccare il codice generato a
mano. `altair image --hooks runtime/hooks.txt`: le funzioni elencate (un indirizzo per riga)
cominciano con `if (altair_hook(c, indirizzo)) return;`. `altair_hook` (in `runtime/altair.c`)
chiama la funzione che il runtime ha registrato con `altair_set_hook`; senza registrazione
ritorna zero. Zero vuol dire "prosegui": l'aggancio ha solo guardato, o ha ritoccato gli
argomenti sullo stack. Diverso da zero vuol dire "fatto io": l'aggancio deve aver lasciato
registri e stack come dopo il `ret` della funzione.

Lato runtime (`crates/runtime/src/hooks.rs`): ogni aggancio conta le chiamate (comando `hooks`
del canale di comando), `ALTAIR_HOOK_LOG=N` scrive le prime N di ognuno con istante, `ecx`,
tre argomenti, indirizzo di ritorno e thread; la tabella `REPLACEMENTS` associa un indirizzo a
chi interviene. Cambiare l'elenco riscrive solo i file delle funzioni toccate: pochi secondi
di ricompilazione. Costo a regime: una chiamata indiretta per ingresso nella funzione.

Primo uso vero: la versione dell'Animus (`runtime.md`). Primo uso d'indagine: le interferenze
(`domande-aperte.md`), con 47 agganci accesi e poi spenti.

## Limiti noti

- Flag non confrontati direttamente dal test (solo attraverso i loro effetti).
- `fnstenv`/`fnsave` scrivono solo parola di controllo, parola di stato e registri.
- x87 con `long double`: funziona solo su host con formato esteso a 80 bit (x86). Per ARM e
  wasm servirà un'implementazione software dell'x87. È il vincolo più pesante verso il web.
- La parola di controllo x87 dell'host è quella del guest durante tutta l'esecuzione guest: le
  funzioni host chiamate dal gioco (il futuro runtime) dovranno ripristinarla e reimpostarla.
- x87: niente tag dei registri, niente flag di eccezione; trascendenti non verificate.
- Arrotondamento x87: `frndint` e `fist(p)` seguono il campo RC del guest per costruzione
  (`x87_rnd`); le quattro operazioni seguono invece la parola di controllo dell'host, che è
  tenuta uguale a quella del guest. Se il compilatore le riduce a `float` o `double` il
  risultato non cambia con l'arrotondamento al più vicino; con un'altra modalità sì (non verificato).
- SSE in C portabile: su host non x86 (ARM, wasm) vanno riviste la propagazione dei NaN, le
  conversioni e il controllo dell'arrotondamento.
- `stmxcsr` restituisce l'ultima parola di controllo impostata: i flag di eccezione non sono tracciati.
- Le atomiche usano puntatori host non allineati: va bene su x86-64, da rivedere su ARM e wasm.
- I registri sono letti e scritti nel contesto a ogni istruzione: corretto ma lento.
  L'ottimizzazione (registri in variabili locali) viene dopo la correttezza.
- Eccezioni C++ e SEH: non gestite.
- Un solo file C per traduzione: per l'immagine intera servirà dividere in parti.
