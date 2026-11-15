# RTTI and vtables

Verified on 2026-11-14 with `tools/rtti.py` on `AssassinsCreed_Dx9.exe` (Steam build).
The full detail (names, hierarchies, slots) is in `private/rtti-dx9.json`, outside git
because it contains the names of the game's classes. It regenerates in less than a second:

```sh
python3 -I tools/rtti.py "$AC1/AssassinsCreed_Dx9.exe" --out private/rtti-dx9.json
```

## Counts

| | DX9 | scimitar-recomp (DX9) | DX10 |
|---|---|---|---|
| TypeDescriptor | **4,376** | 4,376 | 939 |
| CompleteObjectLocator | 4,597 | — | 1,000 |
| Vtables | **4,597** | 4,597 | 1,000 |
| Total slots | **59,899** | 59,899 | 8,923 |
| Distinct functions in the slots | **28,717** | 28,717 | 5,061 |

The four numbers published by scimitar-recomp match ours exactly: the parser
is correct and the build is the same.

Other data on the DX9 one:

- `type_info::vftable` at `0x0170B880`.
- 4,323 classes (`.?AV`) and 53 structs (`.?AU`); 945 are template instances.
- 4,363 classes have at least one vtable; 234 vtables are secondary (multiple inheritance,
  non-zero offset). 367 hierarchies have multiple inheritance, only one also virtual.
- Slots per vtable: minimum 1, median 10, maximum 417.
- 25,637 functions appear in a single slot: they belong to a single class.
- The most shared function is `0x00F9F7A2`, in 2,721 slots: it is the CRT's `_purecall` (it calls
  the handler and then exits with error 0x19, "pure virtual function call"). It marks abstract methods.

## Namespaces

| TypeDescriptor | Namespace | What it is |
|---|---|---|
| 3,399 | `scimitar` | the engine and the game |
| 433 | (global) | largely Havok (`hk*`): 300 classes derive from `hkBaseObject` |
| 409 | `magma` | Ubisoft library: serialisation (`LoadVisitor`), `Acceptor`, memory pools; the menus are called `Magma*` |
| 75 | `Gear` | Ubisoft base library |
| 24 | `Os` | operating system abstraction |
| 10 | `std` | |

scimitar-recomp counts 3,407 names in `scimitar`: the difference of 8 depends on how the
namespace is attributed to templates.

## Hierarchy

Most inherited base classes:

| Derived | Base |
|---|---|
| 2,058 | `scimitar::Object` |
| 1,014 | `scimitar::ManagedObject` |
| 612 | `scimitar::IEventHandler` |
| 330 | `hkBaseObject` (Havok) |
| 311 | `scimitar::EventListener` |
| 230 | `scimitar::Event` |
| 207 | `scimitar::Resource` |
| 180 | `magma::Acceptor` |
| 154 | `scimitar::Operator` |
| 141 | `scimitar::Clip` |

The longest chain has 12 classes. 228 classes have no bases.

Families by name suffix, in the `scimitar` namespace: `*Data` 262, `*Event` 258,
`*Clip` 142, `*Handler` 140, `*Monitor` 75, `*Settings` 55, `*Manager` 49, `*Seed` 45,
`*Component` 43, `*Camera` 32, `*Activator` 28, `*State` 27.

Largest vtables: `scimitar::HumanGround` and `IHumanGround` (417 slots),
`magma::BinaryLoadVisitor` (199), `magma::LoadVisitor` (156), the cameras
(`FreeRoamingCamera`, `LowHighProfileCamera`, `HorseCamera`: 105).

**First indications for Phase 6** (hypothesis, from the names alone): `HumanGround`, with 417 virtual
methods, is in all likelihood the heart of the character's ground movement; the `*Event` and
`*Handler` classes are the event system the gameplay rests on; `*Data` and `*Settings` suggest
that many parameters are objects loaded from the `.forge` files.

## Cross-check with EaglePatch

- `0x00909C90`, which EaglePatch gives as `PadProxyPC::Update`, is **slot 9** of the vtable
  of `scimitar::PadProxyPC` (`0x016B13FC`).
- `Pad` (`0x016B1E4C`), `PadXenon` (`0x016B18AC`) and `PadProxyPC` have 19 slots each.
  EaglePatch calls slot 10 as `UpdatePad(InputBindings*)`.

## The DX10 one, for comparison

`private/rtti-dx10.json`, same tool. **The DX10 one has no RTTI for the `scimitar` namespace:**
its 939 types are libraries only (`magma` 385, Havok and globals 428, `Gear` 68, `Os` 24).
Engine and gameplay were compiled without RTTI.

All 1,000 vtables of the DX10 one have a namesake in the DX9 one, 999 with the same number of slots.
Pairing the slots gives **5,059 pairs of DX9 ↔ DX10 functions**. They are anchors for
carrying over to the DX9 one addresses known only on the DX10 one (for example those of the Definitive Parkour Mod),
by extending them along the call graph. It will be needed, if ever, in Phase 6.

## Limits

- The number of slots of a vtable is estimated: consecutive pointers into `.text` are counted
  up to the next pointer to a CompleteObjectLocator. If other function pointers followed a
  vtable, they would be counted as slots. The total matches scimitar-recomp,
  which however in all likelihood uses the same criterion.
- Classes without virtual methods, or compiled without RTTI, do not appear.
- `rtti.py` does not demangle template names: they stay in decorated form.
