# Sections and the stage

Plan for playing the cello as a section of up to 12 players, placed on a shared stage with early reflections, and later a stage view that shows every LibreStrings instance in the project. The design background is PLAN.md §5 (Sections); this file is the working checklist. Update PLAN.md's phase table and STATUS.md as the items land.

**Phase A** gives a section that sounds right in Bitwig: players, humanization, placement and early reflections, set by ordinary parameters.
**Phase B** adds the stage view and instance sync. It is only a UI and a transport over the same numbers, so Phase A has to use the right numbers from the start (see "Design rules").

## Design rules

- **Absolute stage coordinates.** Every instance places its section in metres on one shared stage, with the listener (the mic pair) at a fixed point, not as a pan value. Two instances set to the same room then already sit on one stage, and the Phase B view only draws and drags those numbers.
- **Only relative delays.** A section 15 m from the mics would add 44 ms of latency. Subtract one stage-wide constant (the distance from the mics to the front of the stage) from every path, the same in every instance, so instances stay time-aligned with each other and with the host.
- **Build everything in `initialize`.** All 12 players are built up front; the size control only switches players on and off. Changing the size never builds anything on the audio thread.
- **The late reverb stays with the user's reverb plugin.** The plugin gives the direct sound and the early reflections, and nothing after them.
- **Keep it reproducible.** Every random choice per player comes from a seed built from (instance seed, player index), so a render and a reloaded project sound the same.

## Phase A: sections, placement, early reflections

### A1. Cheap players (prerequisites)

At 4.8% of real time per cello, 12 players would cost about 58% of a core, and PLAN.md's budget for a section is 25%. The instruments took about 30 ms each to build (formerly STATUS item 24, now removed).

- [x] Fit the string designs once per preset and share them: `Performer` (and everything it holds) is `Clone`, so a section builds one player and clones it: 48 µs per player at 2× instead of 29 ms. Only the per-player parts (body seed, humanization) change afterwards (A2).
- [x] Skip idle strings: `Instrument` stops processing a string once the bow is off it and its bridge force has stayed below 1e-5 N (100 dB under a pp note's peak, about −127 dBFS out) for a whole period, and starts again when the bow touches it. Strings that were never played already gave exact zeros but cost as much as a playing one; a released string decayed into denormals (about 3e-44 N after 10 s). The output is unchanged until a string falls silent. The engine's `cpu_cost` went from 4.8% to 2.85%; all tests pass.
- [x] Measure: `cpu_cost_players` (in `strings-plugin`, `--ignored`) plays the same notes on 1–12 cloned players:

  | Players | 2× | 1× |
  |---|---|---|
  | 1 | 2.8% | 1.6% |
  | 4 | 11.3% | 6.4% |
  | 8 | 23.0% | 12.8% |
  | 12 | 34.6% | 19.3% |

  Nearly all of it is the bowed string: one string costs 1.65% at 2× (0.82% at 1×), while the whole body with its 67 modes costs 0.4%. Cutting body modes would save little.
- [ ] Decide on oversampling for sections. 12 players at 2× are over the 25% budget and at 1× within it. At 1×, high notes lock onto whole-sample periods, but the players' detune may hide that in a section. Decide by ear with an A/B render once A2 exists (and consider 2× for player 0 only, the one the telemetry shows).
- [ ] If 2× is needed: SIMD across players (PLAN.md §5). The string loop (bow solve, three fractional delay reads, the 16-stage dispersion cascade, torsion) is where the time goes.

### A2. The section engine (`strings-dsp/src/section.rs`)

- [ ] `Section`: N `Performer`s built for `MAX_PLAYERS = 12`, of which `players` are active, with the same notes and controllers sent to every active player.
- [ ] Players joining or leaving (a size change while playing) fade in or out over about 50 ms, and a new player starts with the next note.
- [ ] Per-player humanization, seeded (PLAN.md §5):
  - [ ] detune: a fixed offset of a few cents plus a slow drift, on top of the intonation by ear
  - [ ] onset timing: note-ons and bow changes delayed by 0–30 ms, with one player at 0 so the section doesn't feel late live. A small fixed-size event queue per player, no allocation
  - [ ] vibrato: rate, depth and phase
  - [ ] dynamics offset, bow position (β) and pressure bias, within the performer's safe β range (β ≤ 0.115)
  - [ ] legato transition time
  - [ ] body: a different dense-mode seed and small mode frequency and Q changes
  - [ ] bow wander seed
- [ ] Solo stays a section of 1 with humanization off, so the solo cello sounds exactly as it does now. Check with a render.
- [ ] Real-time test: a scripted section performance allocates nothing (like `tests/realtime.rs`).

### A3. Placement (`strings-dsp/src/stage.rs`)

- [ ] Stage model: the stage has a size and a room around it. The listener is a stereo mic pair at a fixed point in front of the stage (spaced omnis, about 60 cm apart, to begin with).
- [ ] Section layout: a centre (x, y), a width and a depth. The players sit in rows inside that area (desks of two), jittered a little by the seed.
- [ ] Per player and mic: a fractional delay (distance ÷ c, minus the stage constant), 1/r gain (clamped near the mics), and an air-absorption lowpass that deepens with distance. The ITD and ILD come from the two mic paths, so there is no separate panner.
- [ ] Fractional delay lines sized in `initialize` for the room's largest distance at the highest sample rate.
- [ ] Output is now true stereo; the mono layout sums the two mics.

### A4. Early reflections

- [ ] Shoebox room, first-order image sources: 4 walls, the floor and the ceiling (6 images) per mic. Second order later if it sounds too sparse.
- [ ] The direct sound is per player; the reflections come from the section as a whole (the sum of its players, at the section's centre), so 12 players cost 6 × 2 taps, not 144. Revisit if the reflections sound too clean.
- [ ] Each reflection has a wall gain and a one-pole lowpass set by the absorption.
- [ ] Room presets (dimensions): Studio, Chamber hall, Concert hall, Scoring stage. Absorption: Low, Medium, High.
- [ ] Early-reflection level control, down to off for users who want their own reverb to do everything.

### A5. Plugin parameters and editor

- [ ] `players`: an int parameter 1–12 (1 is Solo), so it can be automated and is saved with the project. In the editor it replaces the Ensemble combo: `‹ Solo ›`, `‹ 8 players ›`.
- [ ] Section position: x, y (m), width and depth, with defaults that put the cello section on the right of the stage, where orchestras seat it.
- [ ] Room preset, absorption and early-reflection level.
- [ ] Mic distance (close ↔ far): moves the mic pair along the stage axis. Phase B puts it on a fader in the stage view; in Phase A it is a plain parameter.
- [ ] Telemetry and the instrument view still show one player (player 0).
- [ ] Engine CPU cost with 12 players in the status line.

### A6. Renderer and checks

- [ ] `strings-render play --players N` (and the stage parameters), so section renders can be A/B'd.
- [ ] A/B renders: solo against 4, 8 and 12 players on `phrase` and `legato`; early reflections on and off; two rooms.
- [ ] Test: with humanization off and one player at the stage centre, the section's left and right outputs equal the solo cello's (up to the delay and gain).
- [ ] Test: moving a section left makes the left channel louder and earlier.
- [ ] Listening in Bitwig: two or three instances (say celli with a solo cello in front) sound like one room, not a chorus effect. This is the phase's acceptance test.
- [ ] Update PLAN.md (Phase 6 row and a "Sections notes" section) and STATUS.md.

## Phase B: the stage view and instance sync

### B1. Instance sync

- [ ] Registry: a fixed table of slots (up to 32 instances) in a named shared-memory file under `$XDG_RUNTIME_DIR`, so it works whether the host runs all plugins in one process or each in its own (Bitwig's plugin hosting modes). Each slot has an instance ID, a heartbeat, a name, the instrument, the players and the section's position, written with a seqlock.
- [ ] A stage-wide block (room, absorption, mic distance, ER level) with a generation counter: the most recent change wins, and every instance copies it.
- [ ] A background thread per instance (started in `initialize`, never touched by `process`) keeps its slot and heartbeat up to date and copies changes into the engine through atomics. Slots whose heartbeat stops (the instance was removed or the host crashed) are freed after a few seconds.
- [ ] Moving another instance's circle writes a request into its slot, and that instance applies it, so each instance still owns its own parameters.
- [ ] Project recall: every instance saves the stage-wide block too. On load, the newest one wins.
- [ ] Stage name (default "Main"), so two projects or two separate orchestras don't share a stage.
- [ ] Decide which values stay automatable host parameters and which become persisted state set by the sync (a background thread can't set host parameters). Suggestion: keep `players` a parameter and make the stage values persisted state.

### B2. The stage view

- [ ] A button in the header that swaps the instrument view for the stage view and back.
- [ ] A top-down stage: the room outline, the stage area and the mic pair, drawn in code in the editor's clean, minimal style.
- [ ] Each instance is a circle labelled with its name ("Celli", "Violas", "Solo cello"), sized by its player count; this instance is highlighted. Drag to move; drag the edge to change the section's width.
- [ ] A close ↔ far mic fader.
- [ ] Room preset and absorption drop-downs.
- [ ] An editable instance name (defaults to the instrument, plural for a section).

### B3. Checks

- [ ] Tested with Bitwig's plugin hosting set to "together" and "individually"; in Reaper too.
- [ ] Removing an instance clears its circle from the others' stage view within a few seconds.
- [ ] Loading a saved project restores every position and the shared room.

## Later

- Divisi (a section split across the notes of a chord), after mono sections work (PLAN.md §5). Until then, a double stop in a section is played by every player.
- Bow changes at different times on long held notes (free bowing) need automatic bow changes (PLAN.md 4.5).
- Violin, viola and bass sections come with those instruments (Phase 5).
