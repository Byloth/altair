# Domande aperte

Quando una domanda trova risposta, sposta la risposta nel file di analisi giusto e
sostituisci qui la voce con un rimando.

## Cambiare la versione dell'Animus mostrata dal gioco (2026-11-15)

Richiesta dell'utente: "1.28" → `2.0.0α`, o in ripiego `2.0.0-dev.1`. Stato:

- `α` non si può: i caratteri del gioco coprono solo Latin-1 (`analisi/forge.md`). Resta
  `2.0.0-dev.1`.
- Il testo sta nei dati compressi (`Animus_MGB`), che non si toccano (cartella in sola lettura).
- In memoria (comandi `find` e `findhex` del canale di comando, nel laboratorio): dieci copie
  di "Versione Animus 1.28", ognuna in un blocco di heap su misura, seguita dopo 2-6 byte dai
  dati di un altro blocco. Un testo più lungo non ci sta. L'oggetto che la possiede tiene
  puntatore e lunghezza (visto `{ptr, …, 0x14, 0x17}`): cambiare il puntatore con uno nostro
  farebbe liberare al gioco memoria non sua.
- Strada giusta, da fare: un **aggancio nel traduttore** (una funzione host chiamata
  all'ingresso o all'uscita di una funzione guest scelta) sulla funzione di magma che crea le
  stringhe dai dati, per sostituire il testo prima che venga allocato. Serve trovare quella
  funzione e aggiungere il meccanismo, che ancora non c'è.

## Funzioni di sviluppo rimaste nel binario (2026-11-15)

Ricognizione in `analisi/funzioni-nascoste.md`. Restano da capire:

1. ~~Chi legge il byte `+0x92` delle impostazioni del motore~~: cercato, nessun lettore
   trovato → `analisi/funzioni-nascoste.md`. Indicatore fatto nostro.
2. Se `QuickTeleportHandler`, la telecamera di debug e le statistiche del profiler hanno
   codice raggiungibile e dati nei `.forge`.
3. Se la telecamera "troppo vicina" segnalata su Windows dipendeva da `frndint`/`fistp` che
   ignoravano l'arrotondamento x87 (corretto oggi, `traduttore.md` punto 7): da riprovare
   prima di indagare oltre.

## Fase 0: binario

- [x] ~~Steam e Ubisoft Connect coincidono indirizzo per indirizzo?~~ **Sì, quasi certamente**:
  entry point, sezioni, TLS, cookie ed export sono identici → `analisi/scimitar-recomp.md`.
  Manca solo un confronto byte per byte di `.text`, che richiederebbe la loro build.
- [ ] Il campo SEHandlerTable della load config vale `0x17D2F10`: c'è una tabella SafeSEH o
  il conteggio è zero? (scimitar-recomp dice "nessuna tabella SafeSEH")
- [ ] **Perché la DX9 originale si blocca all'avvio su Windows?** Non indagato per scelta
  (vedi `decisioni.md`): si riprende in Fase 3 solo se serve.
- [x] ~~Numero esatto di descrittori RTTI e di vtable~~: 4.376 e 4.597, identici a
  scimitar-recomp → `analisi/rtti.md`. I 4.043 di prima erano un errore della regex di `pe.py`,
  che troncava i nomi dei template; 4.323 sono le sole classi, più 53 struct.
- [x] ~~Gli slot delle vtable sono contati bene?~~ Sì: solo 13 su 28.717 non si decodificano
  come funzione → `analisi/funzioni.md`.
- [ ] I 3,7 MB di `.text` non referenziati sono davvero codice morto? Si saprà a runtime: una
  chiamata indiretta verso un indirizzo non tradotto va registrata.
- [ ] Il gioco lancia eccezioni C++ durante il gioco normale? Decide se servono i circa 19.000
  funclet e handler SEH.
- [ ] **A cosa servono i 17 MB in più di `.data` nella DX10?** Curiosità: non blocca nulla.
- [ ] Cosa contengono `.data1` e `_RDATA` (presenti in tutti e due).
- [ ] Quale funzione calcola il CRC-32 dei nomi di classe? La tabella a `0x01915AC0` non ha
  riferimenti diretti → `analisi/harness.md`.

## Fase 2–3: runtime

- [ ] Come sceglie il gioco la lingua delle voci (cerca `DataPC_StreamedSoundsEng.forge`)? Registro, INI o locale?
- [ ] I filmati: il gioco apre i `.bik` anche direttamente. Serve un decodificatore Bink o si saltano?
- [ ] Quali metodi COM chiama davvero il gioco (D3D9, DirectInput, DirectSound)? Da tracciare a runtime.
- [ ] A cosa servono i 2.644 usi di `TlsGetValue`: un solo indice (contesto per thread del motore) o molti?
- [ ] Quanto è profondo lo stack reale dei thread di gioco? (decide la fattibilità sul web)
- [ ] Quali formati di texture e shader model usa davvero la DX9 (SM2, SM3)?
- [x] ~~Si può ignorare EAX all'inizio?~~ Sì: scimitar-recomp fa risultare EAX assente e il gioco va.
- [ ] Il gioco soffre della race multi-CPU vista in ACR (menu che lampeggia con più di 1 CPU)?

## Fase 6: gameplay (dal documento di progetto)

- [x] ~~Missioni e investigazioni sono dati o codice?~~ Struttura nei dati (oggetti
  `MissionItem*` in `DataPC.forge`), comportamento nel codice (una classe per tipo) →
  `analisi/forge.md`.
- [ ] Quali campi hanno `MissionItemList` e `MissionItemPickPocket`? Da leggere nella loro
  funzione di caricamento.
- [ ] Inventario completo delle classi su tutti i 18 archivi, oggetti annidati compresi.
- [ ] Quanto della logica di movimento è parametrizzata nei dati?
- [ ] Come il sistema delle entità crea un personaggio con lo scheletro del giocatore?
- [ ] Fisica e animazioni dipendono dal frame rate?

## Trascendenti x87: con quale oracolo verificarle? (2026-11-15)

`fsin`, `fcos`, `fptan`, `fpatan`, `fyl2x`, `f2xm1` non hanno scenari nel test differenziale.
Unicorn (QEMU) non è un oracolo valido per queste: le calcola con la matematica dell'host in
doppia precisione, quindi gli ultimi bit non sono quelli di un processore vero. L'oracolo
giusto è l'hardware: un programmino nativo x86 che esegue l'istruzione reale e confronta con
la nostra traduzione. Da fare quando servirà l'identità al bit (replay, confronto con
l'originale); per giocare, differenze di un'unità sull'ultima cifra non contano. Il Passo 6
del piano del 2026-11-15 si ferma qui: non c'erano altre istruzioni annotate come non verificate.

## Laboratorio: telecamera sbagliata e interferenze dell'Animus assenti (2026-11-15)

Segnalato dall'utente giocando su Windows (schermata `ac1-bug_wrong_camera.png`, fuori dal
repository). Non ancora indagato: qui solo i fatti riferiti e i punti da cui partire.

Fatti riferiti:

- Nelle scene del laboratorio un clic sinistro cambia telecamera di sorveglianza. Una delle
  inquadrature punta su una zona vuota (finestre e pavimento), e il suo effetto di distorsione
  è fermo, mentre nelle altre si muove. Continuando a cliccare si passa oltre; i dialoghi
  proseguono.
- Nell'originale, in certi momenti compaiono interferenze con i simboli dell'Animus in
  sovrimpressione, e cliccando in quel momento si ottiene un'inquadratura speciale. Da noi le
  interferenze sembrano esserci solo come distorsione più forte, senza simboli, e
  l'inquadratura speciale non si è ottenuta (l'utente non esclude di aver cliccato tardi).

Ipotesi, nessuna verificata:

1. L'inquadratura vuota con l'effetto fermo è proprio quella speciale, raggiunta fuori dal
   suo momento: senza un'interferenza attiva non ha né posa né tempo. Le due segnalazioni
   sarebbero una sola: la condizione "interferenza in corso" da noi non scatta, o scatta male.
2. Le interferenze sono legate ai tempi del dialogo, cioè all'audio: il nostro DirectSound
   non offre le notifiche di posizione (`IDirectSoundNotify`), e fino a oggi il thread audio
   del gioco arrivava in ritardo.
3. Un errore di traduzione in una funzione mai confrontata con l'originale (il test
   differenziale ne copre circa metà delle candidate).
4. I simboli sono disegnati per una strada che non eseguiamo. Poco probabile: nel censimento
   non ci sono draw saltati né metodi D3D non gestiti lungo quel percorso.

Da dove partire: l'RTTI dell'eseguibile ha le classi `Glitcher`, `GlitcherSettings`,
`GlitchCameraClip`, `MissCinematicGlitchConditionClip`, `SecurityCamera`,
`SecurityCameraSettings`, `CameraSwitcher` (`private/rtti-dx9.json`). Servono le loro vtable,
uno strumento che registri quali dei loro metodi vengono eseguiti durante la scena, e il
confronto con la DX10 originale, che sulla macchina dell'utente funziona.

Segnalata nella stessa sessione, anch'essa da capire: telecamera troppo vicina al personaggio
durante il gioco (`ac1-bug_shadows.png`).
