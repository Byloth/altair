# Open questions

When a question is answered, move the answer to the right analysis file and
replace the entry here with a pointer.

## Development features left in the binary (2026-11-15)

Reconnaissance in `analysis/hidden-features.md`. Still to be understood:

1. ~~Who reads byte `+0x92` of the engine settings~~: searched, no reader
   found → `analysis/hidden-features.md`. Indicator made our own.
2. Whether `QuickTeleportHandler`, the debug camera and the profiler statistics have
   reachable code and data in the `.forge` files.
3. Whether the "too close" camera reported on Windows depended on `frndint`/`fistp`
   ignoring the x87 rounding (fixed today, `translator.md` point 7): to be retried
   before investigating further.

## Phase 0: binary

- [x] ~~Do Steam and Ubisoft Connect match address for address?~~ **Yes, almost certainly**:
  entry point, sections, TLS, cookie and exports are identical → `analysis/scimitar-recomp.md`.
  Only a byte-by-byte comparison of `.text` is missing, which would require their build.
- [ ] The SEHandlerTable field of the load config is `0x17D2F10`: is there a SafeSEH table or
  is the count zero? (scimitar-recomp says "no SafeSEH table")
- [ ] **Why does the original DX9 hang at startup on Windows?** Not investigated by choice
  (see `decisions.md`): to be picked up in Phase 3 only if needed.
- [x] ~~Exact number of RTTI descriptors and vtables~~: 4,376 and 4,597, identical to
  scimitar-recomp → `analysis/rtti.md`. The earlier 4,043 were an error in the regex of `pe.py`,
  which truncated template names; 4,323 are the classes alone, plus 53 structs.
- [x] ~~Are the vtable slots counted correctly?~~ Yes: only 13 out of 28,717 do not decode
  as a function → `analysis/functions.md`.
- [ ] Are the 3.7 MB of unreferenced `.text` really dead code? We will know at runtime: an
  indirect call to an untranslated address must be logged.
- [ ] Does the game throw C++ exceptions during normal play? This decides whether the roughly 19,000
  funclets and SEH handlers are needed.
- [ ] **What are the extra 17 MB of `.data` in the DX10 for?** Curiosity: it blocks nothing.
- [ ] What `.data1` and `_RDATA` contain (present in both).
- [ ] Which function computes the CRC-32 of the class names? The table at `0x01915AC0` has no
  direct references → `analysis/harness.md`.

## Phase 2–3: runtime

- [ ] How does the game choose the language of the voices (it looks for `DataPC_StreamedSoundsEng.forge`)? Registry, INI or locale?
- [ ] The videos: the game also opens the `.bik` files directly. Is a Bink decoder needed or are they skipped?
- [ ] Which COM methods does the game really call (D3D9, DirectInput, DirectSound)? To be traced at runtime.
- [ ] What are the 2,644 uses of `TlsGetValue` for: a single index (the engine's per-thread context) or many?
- [ ] How deep is the real stack of the game threads? (decides feasibility on the web)
- [ ] Which texture formats and shader model does the DX9 really use (SM2, SM3)?
- [x] ~~Can EAX be ignored at the beginning?~~ Yes: scimitar-recomp makes EAX appear absent and the game works.
- [ ] Does the game suffer from the multi-CPU race seen in ACR (menu flickering with more than 1 CPU)?

## Phase 6: gameplay (from the project document)

- [x] ~~Are missions and investigations data or code?~~ Structure in the data (`MissionItem*`
  objects in `DataPC.forge`), behaviour in the code (one class per type) →
  `analysis/forge.md`.
- [ ] Which fields do `MissionItemList` and `MissionItemPickPocket` have? To be read in their
  loading function.
- [ ] Complete inventory of the classes across all 18 archives, nested objects included.
- [ ] How much of the movement logic is parameterised in the data?
- [ ] How does the entity system create a character with the player's skeleton?
- [ ] Do physics and animations depend on the frame rate?

## x87 transcendentals: which oracle to verify them with? (2026-11-15)

`fsin`, `fcos`, `fptan`, `fpatan`, `fyl2x`, `f2xm1` have no scenarios in the differential test.
Unicorn (QEMU) is not a valid oracle for these: it computes them with the host's
double-precision math, so the last bits are not those of a real processor. The right
oracle is the hardware: a small native x86 program that executes the real instruction and compares with
our translation. To be done when bit-exact identity is needed (replay, comparison with
the original); for playing, differences of one unit in the last place do not matter. Step 6
of the plan of 2026-11-15 stops here: there were no other instructions noted as unverified.

## Lab: wrong camera and missing Animus glitches (2026-11-15)

**Glitches, investigation with the hooks (same day).** The user expects the first
glitch in the lab when Desmond says "… e mi avete legato a quella… cosa!" ("… and you strapped me to that… thing!").
Hooked 47 methods and followed the scene for 130 seconds (`ALTAIR_HOOK_LOG`, `hooks` command):

- `Glitcher` (`0xAD91B0`, the drawing of the effect, called from `0xAAFD4B` when an object
  says the effect is active): it runs for about 680 frames **only at the beginning**, during
  Desmond's awakening (white, blur), and then never again.
- `GlitchCameraClip` and `MissCinematicGlitchConditionClip`: **no instance is even
  constructed** (the four constructors hooked) from the menu up to two minutes into the lab.
- Running continuously instead are `CameraActionConditionClip` (the click that changes camera),
  `PadInputConditionClip` and `PresentTimeConditionClip` (`0xC01B00`: it compares a state
  index with a value of the clip, it is not a clock).

So in the lab, with our data, there is no "glitch with a special
shot" object: either in the original that glitch belongs to the scenes inside the
memories (where those clips would make sense) and not to this one, or it comes from a mechanism that
I have not identified yet. **Confirmation on the original is needed** (DX10, which works for the
user): in which scene and what it looks like. The hooks used are in `runtime/hooks.txt`, disabled.

**Update, same day: the empty shot is gone.** After the fix to
`frndint`/`fistp` and to the ID bit of EFLAGS (`analysis/translator.md`, points 7 and 8) the user
confirms on Windows that the wrong camera is no longer there. Verified here on WSL as well: eight
clicks in the lab, three alternating shots, all on the characters
(`run/shots/cam-foglio.png`). Which of the two fixes solved it has not been isolated:
hypothesis 3 below (translation error) was the right one. **Still to be retried**: the
glitches with the symbols and the special shot, and the "too close" camera in game.

Reported by the user while playing on Windows (screenshot `ac1-bug_wrong_camera.png`, outside the
repository). Not investigated yet: here only the reported facts and the starting points.

Reported facts:

- In the lab scenes a left click changes surveillance camera. One of the
  shots points at an empty area (windows and floor), and its distortion effect
  is frozen, while in the others it moves. Clicking on gets past it; the dialogue
  continues.
- In the original, at certain moments glitches appear with the Animus symbols
  overlaid, and clicking at that moment gives a special shot. In ours the
  glitches seem to be there only as a stronger distortion, without symbols, and
  the special shot was not obtained (the user does not rule out having clicked late).

Hypotheses, none verified:

1. The empty shot with the frozen effect is precisely the special one, reached outside
   its moment: without an active glitch it has neither pose nor time. The two reports
   would be a single one: the "glitch in progress" condition does not fire for us, or fires wrongly.
2. The glitches are tied to the timing of the dialogue, that is to the audio: our DirectSound
   does not offer position notifications (`IDirectSoundNotify`), and until today the game's audio
   thread was arriving late.
3. A translation error in a function never compared with the original (the differential
   test covers about half of the candidates).
4. The symbols are drawn by a path we do not execute. Unlikely: in the census
   there are no skipped draws or unhandled D3D methods along that path.

Where to start: the executable's RTTI has the classes `Glitcher`, `GlitcherSettings`,
`GlitchCameraClip`, `MissCinematicGlitchConditionClip`, `SecurityCamera`,
`SecurityCameraSettings`, `CameraSwitcher` (`private/rtti-dx9.json`). What is needed: their vtables,
a tool that records which of their methods are executed during the scene, and the
comparison with the original DX10, which works on the user's machine.

Reported in the same session, also to be understood: camera too close to the character
during play (`ac1-bug_shadows.png`).
