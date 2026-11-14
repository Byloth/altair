# RTTI e vtable

Verificato il 2026-11-14 con `tools/rtti.py` su `AssassinsCreed_Dx9.exe` (build Steam).
Il dettaglio completo (nomi, gerarchie, slot) sta in `private/rtti-dx9.json`, fuori da git
perché contiene i nomi delle classi del gioco. Si rigenera in meno di un secondo:

```sh
python3 -I tools/rtti.py "$AC1/AssassinsCreed_Dx9.exe" --out private/rtti-dx9.json
```

## Conteggi

| | DX9 | scimitar-recomp (DX9) | DX10 |
|---|---|---|---|
| TypeDescriptor | **4.376** | 4.376 | 939 |
| CompleteObjectLocator | 4.597 | — | 1.000 |
| Vtable | **4.597** | 4.597 | 1.000 |
| Slot totali | **59.899** | 59.899 | 8.923 |
| Funzioni distinte negli slot | **28.717** | 28.717 | 5.061 |

I quattro numeri pubblicati da scimitar-recomp coincidono esattamente con i nostri: il parser
è corretto e la build è la stessa.

Altri dati della DX9:

- `type_info::vftable` a `0x0170B880`.
- 4.323 classi (`.?AV`) e 53 struct (`.?AU`); 945 sono istanze di template.
- 4.363 classi hanno almeno una vtable; 234 vtable sono secondarie (ereditarietà multipla,
  offset diverso da zero). 367 gerarchie hanno ereditarietà multipla, una sola anche virtuale.
- Slot per vtable: minimo 1, mediana 10, massimo 417.
- 25.637 funzioni compaiono in un solo slot: appartengono a una sola classe.
- La funzione più condivisa è `0x00F9F7A2`, in 2.721 slot: è `_purecall` del CRT (chiama
  l'handler e poi esce con l'errore 0x19, "pure virtual function call"). Marca i metodi astratti.

## Namespace

| TypeDescriptor | Namespace | Cos'è |
|---|---|---|
| 3.399 | `scimitar` | il motore e il gioco |
| 433 | (globale) | in gran parte Havok (`hk*`): 300 classi derivano da `hkBaseObject` |
| 409 | `magma` | libreria Ubisoft: serializzazione (`LoadVisitor`), `Acceptor`, pool di memoria; i menu si chiamano `Magma*` |
| 75 | `Gear` | libreria di base Ubisoft |
| 24 | `Os` | astrazione del sistema operativo |
| 10 | `std` | |

scimitar-recomp conta 3.407 nomi in `scimitar`: la differenza di 8 dipende da come si
attribuisce il namespace ai template.

## Gerarchia

Classi base più ereditate:

| Derivate | Base |
|---|---|
| 2.058 | `scimitar::Object` |
| 1.014 | `scimitar::ManagedObject` |
| 612 | `scimitar::IEventHandler` |
| 330 | `hkBaseObject` (Havok) |
| 311 | `scimitar::EventListener` |
| 230 | `scimitar::Event` |
| 207 | `scimitar::Resource` |
| 180 | `magma::Acceptor` |
| 154 | `scimitar::Operator` |
| 141 | `scimitar::Clip` |

La catena più lunga ha 12 classi. 228 classi non hanno basi.

Famiglie per suffisso del nome, nel namespace `scimitar`: `*Data` 262, `*Event` 258,
`*Clip` 142, `*Handler` 140, `*Monitor` 75, `*Settings` 55, `*Manager` 49, `*Seed` 45,
`*Component` 43, `*Camera` 32, `*Activator` 28, `*State` 27.

Vtable più grandi: `scimitar::HumanGround` e `IHumanGround` (417 slot),
`magma::BinaryLoadVisitor` (199), `magma::LoadVisitor` (156), le camere
(`FreeRoamingCamera`, `LowHighProfileCamera`, `HorseCamera`: 105).

**Prime indicazioni per la Fase 6** (ipotesi, dai soli nomi): `HumanGround`, con 417 metodi
virtuali, è con ogni probabilità il cuore del movimento a terra del personaggio; gli `*Event` e
`*Handler` sono il sistema a eventi su cui poggia il gameplay; `*Data` e `*Settings` suggeriscono
che molti parametri siano oggetti caricati dai `.forge`.

## Controllo incrociato con EaglePatch

- `0x00909C90`, che EaglePatch indica come `PadProxyPC::Update`, è lo **slot 9** della vtable
  di `scimitar::PadProxyPC` (`0x016B13FC`).
- `Pad` (`0x016B1E4C`), `PadXenon` (`0x016B18AC`) e `PadProxyPC` hanno 19 slot ciascuna.
  EaglePatch chiama lo slot 10 come `UpdatePad(InputBindings*)`.

## Limiti

- Il numero di slot di una vtable è stimato: si contano i puntatori consecutivi a `.text`
  fino al prossimo puntatore a un CompleteObjectLocator. Se dopo una vtable ci fossero altri
  puntatori a funzione, verrebbero contati come slot. Il totale coincide con scimitar-recomp,
  che però usa con ogni probabilità lo stesso criterio.
- Le classi senza metodi virtuali, o compilate senza RTTI, non compaiono.
- `rtti.py` non scioglie i nomi dei template: restano nella forma decorata.
