# Domande aperte

Quando una domanda trova risposta, sposta la risposta nel file di analisi giusto e
sostituisci qui la voce con un rimando.

## Fase 0: binario

- [x] ~~Steam e Ubisoft Connect coincidono indirizzo per indirizzo?~~ **Sì, quasi certamente**:
  entry point, sezioni, TLS, cookie ed export sono identici → `analisi/scimitar-recomp.md`.
  Manca solo un confronto byte per byte di `.text`, che richiederebbe la loro build.
- [ ] Il campo SEHandlerTable della load config vale `0x17D2F10`: c'è una tabella SafeSEH o
  il conteggio è zero? (scimitar-recomp dice "nessuna tabella SafeSEH")
- [ ] **Perché la DX9 originale si blocca all'avvio su Windows?** Driver, risoluzione,
  numero di core, EAX? Da provare con DXVK, dgVoodoo2 e con EaglePatch.
- [ ] **Numero esatto di descrittori RTTI e di vtable** nella DX9 (scimitar-recomp: 4.376 e 4.597).
  Con una scansione di stringhe loro trovano 4.323 nomi, noi 4.043: capire la differenza.
- [ ] **A cosa servono i 17 MB in più di `.data` nella DX10?** Curiosità: non blocca nulla.
- [ ] Cosa contengono `.data1` e `_RDATA` (presenti in tutti e due).

## Fase 2–3: runtime

- [ ] Quanto è profondo lo stack reale dei thread di gioco? (decide la fattibilità sul web)
- [ ] Quali formati di texture e shader model usa davvero la DX9 (SM2, SM3)?
- [x] ~~Si può ignorare EAX all'inizio?~~ Sì: scimitar-recomp fa risultare EAX assente e il gioco va.
- [ ] Il gioco soffre della race multi-CPU vista in ACR (menu che lampeggia con più di 1 CPU)?

## Fase 6: gameplay (dal documento di progetto)

- [ ] Missioni e investigazioni sono dati nei `.forge` o codice nell'eseguibile?
- [ ] Quanto della logica di movimento è parametrizzata nei dati?
- [ ] Come il sistema delle entità crea un personaggio con lo scheletro del giocatore?
- [ ] Fisica e animazioni dipendono dal frame rate?
