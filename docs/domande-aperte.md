# Domande aperte

Quando una domanda trova risposta, sposta la risposta nel file di analisi giusto e
sostituisci qui la voce con un rimando.

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
