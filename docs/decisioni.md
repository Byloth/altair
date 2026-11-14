# Decisioni

Voci più recenti in cima. Ogni voce: decisione, motivazione, cosa la farebbe rivedere.

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
