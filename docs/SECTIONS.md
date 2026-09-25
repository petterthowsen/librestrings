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
- [x] Decide on oversampling for sections: player 0 plays at the settings' 2×, so a section of one is still the solo cello, and players 1–11 at 1× (`section::PLAYER_OVERSAMPLING`). In the 8-player renders (`out/ab-section/phrase-8-1x.wav` against `-2x`, and `scale`) 1× and 2× couldn't be told apart (September 2026). 12 players cost about 2.8 + 11 × 1.6 ≈ 20%.
- [ ] Only if needed later: SIMD across players (PLAN.md §5). The string loop (bow solve, three fractional delay reads, the 16-stage dispersion cascade, torsion) is where the time goes.

### A2. The section engine (`strings-dsp/src/section.rs`)

- [x] `Section`: `MAX_PLAYERS = 12` `Performer`s cloned from one, of which `players` are active, with the same notes and controllers sent to every active player. Each active player's gain is 1/√N, so the power stays about the same at every size.
- [x] Players joining or leaving (a size change while playing) fade in or out over 50 ms (`FADE`). A player switched off finishes its note as if it were let go, and once faded out it is reset and not processed. A player switched on comes in with the next note.
- [x] Per-player humanization (`Humanization`, first guesses to be judged by ear). Each player draws a value in ±1 per quantity from its seed, and the spread scales it, so changing the spread keeps each player's character:
  - [x] detune: ±5 cents fixed plus ±2 cents of drift over about 3 s (`Performer::set_detune`, which moves the open strings and the pitch the ear aims for; the strings have 50 cents of memory below their open pitch)
  - [x] onset timing: each player comes in 0–25 ms late, each note ±8 ms around that, never early and never out of order. Player 0 is never late. A 64-note queue per player, no allocation
  - [x] vibrato: rate ±10%, depth ±25%, and phase (`Performer::reseed`)
  - [x] dynamics ±0.05, bow position up to 12% toward the bridge (never away: β ≤ 0.115), pressure ±0.08
  - [x] attack, legato and portamento times ±20%
  - [x] body: listed modes ±3% in frequency and ±15% in damping, and the dense modes from another seed
  - [x] bow wander seed (`Performer::reseed`)
- [x] Solo stays a section of 1 with no humanization: player 0 has none, and `one_player_is_the_solo_performer` checks it sample for sample. The solo `play phrase` render and the Schelleng map are bit-identical to before.
- [x] Divisi (`Polyphony::Divisi`): the notes of a chord are divided among the players, one note each, as the desks of a real section divide. A player keeps the note it is playing while the chord holds it, so only the players a note that has just come in needs move, and they end their note and start the new one. A note let go takes its players with it; they rest until the next chord, so a player never joins a note already sounding. Notes that arrive together (one sample, as a MIDI chord does) are divided once, before anyone starts, so nothing is re-struck; a live-played chord (notes a few ms apart) takes its players when they arrive, and a stroke still waiting for its humanizing delay is dropped rather than played for those few ms. The section divides only where it can — with more than one player; a solo's divisi is the double stops it can play. With more notes than players every player still plays one note, and a note that comes in with no player free takes the one whose note is closest to it in pitch, which ends that note. Tests: `divisi_divides_a_chord_among_the_players`, `divisi_takes_the_players_a_new_note_needs`, `divisi_lets_the_players_of_a_released_note_go`, `a_section_of_one_plays_its_chord_as_double_stops`, `divisi_with_more_notes_than_players_steals_the_closest`, `divisi_survives_a_size_change`, `a_divided_chord_is_reproducible` (tests/section.rs), the divisi part of `a_section_never_allocates`, and `divisi_divides_a_chord_in_the_engine` (the plugin's telemetry). Renders: `strings-render play divisi --players N` (the `divisi.score`), plus the same chords with the score's `poly` line taken out and `--divisi` against `--double-stops` — in `out/divisi/`: `divisi-8`, `divisi-solo`, `divisi-4-stage`, and a two-chord score at 1 and 8 players each way (a solo's two renders are byte-identical; the 8-player ones differ). Not listened to yet. Cost: routing only, and a chord is cheaper as divisi than as double stops, since each player bows one string rather than two (8 players: 14.8% of real time against 22.4%, about 3 dB quieter in all, with each note louder). The `divisi` score at 8 players renders at 15.7%.
- [x] Real-time test: `a_section_never_allocates` (size changes, retuning, "all notes off" with notes waiting).
- [x] Tune the `Humanization` defaults by ear: its spreads are in the plugin's tuning window ("Section"), and "Copy changes" gives the values for `Humanization::default`. Tuned in Bitwig (September 2026): detune ±7 cents with ±4 cents of drift over about 2 s, vibrato rate ±17%, dynamics ±0.1, pressure ±0.12, timing ±29%. Retuned later that month: no fixed detune, only ±7 cents of drift; entries up to 10 ms late (25); dynamics ±0.2, bow position +30% (12%), pressure ±0.3 (0.12), timing ±10% (29%).

Renders (`strings-render play <score> --players N`, mono, no placement yet): `phrase-{1,4,8,12}.wav`, `legato-{1,4,8,12}.wav`, and 8 players at 1× and 2× on `phrase` and `scale` (`phrase-8-1x.wav` …) for the oversampling decision (A1). Rendering cost: 12 players at 33% of real time on `phrase` and 43% on `legato`, where more strings ring on.

### A3. Placement (`strings-dsp/src/stage.rs`)

- [x] Stage model: metres, `x` to the audience's right, `y` upstage from the front of the stage, `z` up. The mics are a near-coincident pair on the centre line, `mic_distance` in front of the stage at 2.5 m high: two cardioids 17 cm apart, angled ±55° (as ORTF). A spaced omni pair images poorly from far away; this one places a player by time and level at any distance. The instruments are at 1 m.
- [x] Section layout (`Placement`): a centre (x, y), a width and a depth. The players fill rows across that area, front to back (player 0 at the front), each up to 15 cm off its seat by the seed. One player sits at the centre. Default: the cellos' place, 3.5 m to the right and 3 m upstage, 4 × 3 m.
- [x] Per player and mic: a fractional delay (distance ÷ c, minus the distance from the mic to the front of the stage), 1/r gain relative to a player 3 m upstage on the centre line (so moving the mics changes the balance, not the level), the cardioid's gain, and air absorption (0.15 dB/m at 10 kHz) as a one-pole lowpass. Delays and gains glide over 50 ms when anything moves.
- [x] Delay lines sized in `Stage::new` for the largest room at the sample rate. A player whose line holds only silence is skipped.
- [x] Output is stereo in the renderer (`play --players N`, or `--stage` for a solo). The plugin's output is A5.

### A4. Early reflections

- [x] Shoebox room, first-order image sources: 4 walls, the floor and the ceiling (6 images) per mic. Second order later if it sounds too sparse.
- [x] The direct sound is per player; the reflections come from the section's centre, from the sum of its players: 12 taps whatever the size. Revisit if they sound too clean.
- [x] Each reflection has the wall's reflection gain and a lowpass (absorption Low: 0.95 and 10 kHz; Medium: 0.84 and 6 kHz; High: 0.63 and 3 kHz), plus the air's.
- [x] Room presets (width × length × height, and how far the back wall is behind the front of the stage): Studio 12 × 16 × 6 (7), Chamber hall 16 × 26 × 11 (8), Concert hall 24 × 42 × 17 (11), Scoring stage 22 × 30 × 12 (12).
- [x] Early-reflection level, 0 (off) to 1.
- [ ] Listen to the renders below and tune the rooms, the absorption and the mic pair.

Renders (`out/ab-stage/`, stereo): `phrase-solo-front` (a solo at the centre front), `phrase-8-chamber` (the default), `-dry` (no reflections), `-concert-far` (mics at 12 m), `-studio-close` (1.5 m), `-chamber-low` / `-high` (absorption), `legato-8-chamber`, and `legato-12-left` (the section moved to the left). The cellos on the right come out 2–4 dB louder on the right channel; levels stay within 2 dB across rooms and mic distances. Rendering cost with the stage: solo 3.9%, 8 players 16% on `phrase`, 12 players 29% on `legato`, where more strings ring on.

### A5. Plugin parameters and editor

- [x] `players`: an int parameter 1–12 (1 is Solo), automatable and saved with the project. In the editor it replaces the Ensemble combo: `‹ Solo ›`, `‹ 8 players ›`.
- [x] Section position: x, y, width and depth (m). The defaults put a new instance at the front, in the middle (x 0, y 1.5), as a soloist; move a section to its place (the renderer's `Placement::CELLOS` is 3.5 m right, 3 m up).
- [x] Room preset, absorption and early-reflection level, and a Stage switch: off is the players' dry mono sum, as before the stage.
- [x] Mic distance (close ↔ far), a plain parameter until the stage view.
- [x] Stereo output (the mono layout gets the mean of the two mics).
- [x] Telemetry and the instrument view show player 0. The tuning window's changes reach every player: its string refits are fitted at both string rates (2× for player 0, 1× for the rest) and copied for each player on the editor's thread.
- [x] The status line's CPU warning is at 25% for a section (3% solo).
- [x] Engine cost (`cpu_cost`, `cpu_cost_players`): solo 3.7% on the stage and 3.0% dry; 4 players 9.2%, 8 players 16.3%, 12 players 23.3%. The stage costs about 0.7% (its glides snapped to their targets and silence flushed, or they ran into denormals; the reflections read with linear interpolation).
- [ ] Try it in Bitwig: two or three instances, set to the same room and mics.

### A6. Renderer and checks

- [x] `strings-render play --players N` and `--divisi`, so section renders can be A/B'd.
- [x] The stage parameters in the renderer (`--x --y --width --depth --room --absorption --mic-distance --reflections`), and stereo output.
- [ ] A/B renders: solo against 4, 8 and 12 players on `phrase` and `legato`; early reflections on and off; two rooms.
- [x] Test: a player on the centre line reaches both mics alike, reflections included; a section of one is the solo cello (`one_player_is_the_solo_performer`).
- [x] Test: moving a section left makes the left channel louder and earlier (`tests/stage.rs`, with the centre, distance, reflections and gliding).
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

- Bow changes at different times on long held notes (free bowing) need automatic bow changes (PLAN.md 4.5).
- Violin, viola and bass sections come with those instruments (Phase 5). All four are in; the viola and bass are seated by default at the centre (`Placement::VIOLAS`) and behind the cellos (`Placement::BASSES`), renderer only, with the cello's humanization. The violin was first: its sections play through the same engine (`play violin-phrase --instrument violin --players 8`), seated by default on the audience's left (`Placement::VIOLINS`, renderer only; the plugin's stage position is a parameter). Its humanization is the cello's.
