# Decisioni

Voci più recenti in cima. Ogni voce: decisione, motivazione, cosa la farebbe rivedere.

## 2026-11-15 · Decodificatore Bink nostro al posto di ffmpeg

**Decisione:** i filmati li decodifica `crates/bink`, scritto da noi; `ffmpeg` resta solo come
oracolo dei test (`tools/binktest.py`) e come ripiego (`ALTAIR_BINK=ffmpeg`).

**Perché:** `ffmpeg` come processo esterno era una dipendenza di sistema e nel browser non
esiste. Il formato Bink 1 è documentato pubblicamente e i file del gioco ne usano un
sottoinsieme piccolo (revisione `i`, niente alfa, audio DCT).

**Regola seguita:** come per scimitar-recomp, il codice di FFmpeg (LGPL) è stato letto come
specifica e non copiato. Le tabelle del formato sono dati: riportate con la provenienza.

**Verifica:** 3.381 fotogrammi identici byte per byte a ffmpeg; audio entro 4 × 10⁻⁷.

## 2026-11-15 · x87 su `long double` dell'host

**Decisione.** I registri x87 sono `long double` e le operazioni sono aritmetica C; la parola di
controllo x87 dell'host segue quella del guest. Nessuna emulazione software.

**Perché.** Sugli host x86 `long double` è il formato a 80 bit dell'x87: risultati identici
all'originale senza scrivere un emulatore, verificati su 4.201 funzioni. Il nativo viene prima
del web (decisione del 2026-11-14).

**Costo accettato.** Non funziona su ARM né su wasm, dove `long double` è un altro formato:
per il browser (Fase 5) servirà un x87 software dietro la stessa interfaccia (`f80`, `ST(i)`).

**Si rivede** all'apertura della Fase 5, o se serve un build nativo per ARM.

## 2026-11-15 · Nome completo: "AC1 Recompiled - Altaïr"

**Decisione.** Il titolo esteso del progetto diventa "AC1 Recompiled - Altaïr" (prima
"AC1 Recompiled"). Nel codice, nei comandi e nei percorsi resta `altair`, senza dieresi.

**Perché.** Scelta dell'utente: il nome tecnico entra nel titolo. La voce qui sotto, dello stesso
giorno, riporta ancora il titolo precedente.

## 2026-11-15 · Nome tecnico del progetto: Altair

**Decisione.** Il ricompilatore e tutto ciò che è codice si chiamano `altair` (prima `acrek`);
la cartella del progetto diventa `altair` (prima `ac-rek`). Il nome completo resta
"AC1 Recompiled". Un ricompilatore per un altro gioco prenderebbe il nome del suo protagonista.

**Perché.** Scelta dell'utente. Altair è un nome proprio e una stella, già usato in informatica
(l'Altair 8800 del 1975): come nome tecnico non richiama un prodotto ufficiale.

**Attenzione.** Esiste una libreria Python molto diffusa con lo stesso nome: da tenere presente
se un giorno si pubblicano pacchetti. Verso l'esterno si usa "AC1 Recompiled".

## 2026-11-15 · SSE in C portabile; registri parziali con scritture strette

**Decisione.** Le operazioni SSE sono funzioni C corsia per corsia (`runtime/altair_sse.h`),
senza intrinseci del processore. I registri a 8 e 16 bit si scrivono con `SET8L`/`SET8H`/`SET16`
(scritture strette nel campo a 32 bit), non con maschera e OR.

**Perché.** Il C portabile compila su ogni host, e su x86-64 produce le stesse istruzioni
dell'originale: 3.042 funzioni identiche su 6.000 lo confermano. Le scritture strette aggirano
una miscompilazione di clang 18.1.3 (`tests/compiler/clang18-partial-register.c`).

**Si rivede se** su ARM o wasm servono intrinseci per prestazioni o per la semantica dei NaN,
o se un clang più recente non ha più il bug.

## 2026-11-14 · Forma del codice generato

**Decisione.** Una funzione C per funzione guest, `void f_XXXXXXXX(Ctx *c)`; stack guest
identico all'originale (indirizzi di ritorno veri); flag come variabili locali; memoria guest
come base più indirizzo su 4 GB riservati; chiamate indirette tramite un registro di funzioni.
Il supporto è in C (`runtime/altair.h`), il ricompilatore in Rust. Dettagli in
`analisi/traduttore.md`.

**Perché.** È il modello di scimitar-recomp, che ha dimostrato di funzionare su questo
eseguibile; lo stack identico rende possibile il confronto byte per byte con Unicorn; i flag
locali lasciano l'ottimizzazione al compilatore C.

**Si rivede se** le prestazioni lo richiedono (registri in variabili locali) o se su wasm32 i
frame C risultano troppo profondi.

## 2026-11-14 · L'originale DX9 in esecuzione non serve ora: spostato in Fase 3, facoltativo

**Decisione.** Non si lavora per far partire `AssassinsCreed_Dx9.exe` originale su Windows
(si blocca all'avvio). L'attività esce dalla Fase 0 e diventa facoltativa in Fase 3.

**Perché.** La correttezza del codice tradotto si verifica con l'harness Unicorn, senza
Windows. Per il confronto visivo c'è la DX10 originale, che funziona. I metodi COM usati si
ricavano dal nostro runtime con stub tracciati.

**Si rivede se** in Fase 3 una differenza di rendering non si chiarisce con la DX10.

## 2026-11-14 · Gli indirizzi di scimitar-recomp valgono per la nostra build

**Decisione.** Usiamo indirizzi, conteggi e insidie di scimitar-recomp come validi per la
build Steam, senza rimappare nulla.

**Perché.** Entry point, indirizzi delle sezioni, TLS, cookie `/GS` ed export della loro build
Ubisoft Connect coincidono con la nostra. Ubisoft Connect aggiunge solo `.nidata` in coda.
Dettagli in `analisi/scimitar-recomp.md`.

**Si rivede se** un indirizzo preso da loro non corrisponde al codice che vediamo.

## 2026-11-14 · DX12 si ottiene tramite wgpu, non convertendo il renderer

**Decisione.** Il gioco continua a chiamare D3D9; il nostro runtime implementa D3D9 sopra
wgpu, che su Windows usa DX12 (o Vulkan), su macOS Metal, nel browser WebGPU.

**Perché.** Portare il motore a chiamare DX12 direttamente vorrebbe dire decompilare tutto il
renderer: mesi di lavoro per lo stesso risultato. Le migliorie grafiche previste (risoluzione,
MSAA, LOD, sostituzione texture, post-processing) passano tutte dallo strato di traduzione.

**Si rivede se** servono funzioni che wgpu non espone, come il ray tracing (oggi solo
sperimentale).

## 2026-11-14 · Eseguibile di riferimento: `AssassinsCreed_Dx9.exe` della build Steam

**Decisione.** Si ricompila la DX9 (SHA-1 `a9ea476e…432120`), non la DX10.

**Perché.**
1. Ha 4.376 nomi RTTI contro 939: è la fonte principale di nomi senza PDB.
2. scimitar-recomp (la nostra specifica) e EaglePatch (indirizzi Steam) lavorano sulla DX9.
3. Il D3D9 lo reimplementiamo noi, quindi il blocco all'avvio dell'originale su Windows non
   riguarda la versione ricompilata.

**Costo accettato.** Per confrontare con l'originale in esecuzione bisogna far partire la DX9
originale (DXVK o dgVoodoo2, da provare). Il differential testing per funzione con Unicorn non
ne ha bisogno. Il renderer DX10, con l'MSAA migliore, non ci serve: la grafica è nostra.

**Si rivede se** la build Ubisoft Connect di scimitar-recomp risulta molto diversa dalla Steam,
togliendo valore alla specifica.

## 2026-11-14 · Riscrittura, non fork di scimitar-recomp

Vedi il documento di progetto, sezione "Decisione: riuso o riscrittura". In breve: circa il
70% del loro codice (Metal, Cocoa, arm64) andrebbe comunque riscritto, e un fork renderebbe
tutto GPL-2.0.

## 2026-11-14 · Ordine: nativo → web → modifiche

Vedi il documento di progetto, sezioni "Sintesi" e "Fasi". Il gate è la Fase 4: gioco finito
in nativo.
