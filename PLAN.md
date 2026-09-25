# LibreStrings — Plan

A CLAP instrument plugin, written in Rust, that synthesizes bowed string instruments by physical modeling.

- **Long-term scope:** violin, viola, cello and double bass, each playable as a **solo** instrument or as a **section** of up to about 12 players (the limit depends on CPU cost). Sections are placed on a stereo stage and humanized.
- **First milestone:** solo cello. The order of instruments doesn't matter for the goal, and our measured reference data (bowed cello G strings, see Phase 0–1 notes) validates the string and bow model directly on the cello. The core model is instrument-independent (SI units), so everything calibrated here carries over to the violin, viola and bass.

Background research is in [docs/](docs/); the papers the model takes numbers from are listed in [docs/Literature.md](docs/Literature.md). This plan corrects several errors in that research; see [Research notes](#research-notes--corrections).

---

## 1. Guiding decisions

| Decision | Choice | Why |
|---|---|---|
| Language | Rust | Memory safety without GC, good SIMD story, user preference |
| Plugin format | CLAP only (for now) | Modern API, good for note expressions and polyphonic modulation. VST3 can be added later with little effort (see caveats) |
| Plugin framework | [`nih-plug`](https://github.com/robbert-vdh/nih-plug), exporting only CLAP | Mature. Handles parameters, smoothing, MIDI/CC and GUI hooks. Alternative: [`clack`](https://github.com/prokopyl/clack) (low-level CLAP bindings) if nih-plug gets in the way |
| String model | Digital waveguide (DWG) | Cost per sample doesn't grow with harmonic count, which makes 12-player sections affordable. Tuning is exact through fractional delays |
| Bow model | Scattering junction with a hyperbolic friction curve, solved in closed form (a quadratic), plus stick/slip hysteresis | Deterministic cost per sample, no iteration, well understood physically |
| Body | Post-string filter: a bank of 20–40 biquad resonators. Partitioned convolution is the alternative | Cheap, can be varied per player, has tweakable parameters |
| Dev workflow | Offline renderer first, plugin second | Iterating on WAV output and plots is much faster than inside a DAW |

---

## 2. Repository layout (proposed)

Crate names are placeholders.

```
Cargo.toml                 # workspace
crates/
  strings-dsp/             # pure DSP: no plugin/framework deps, no allocation in process()
    src/
      delay.rs             # fractional delay lines (Lagrange 3rd order, Thiran)
      filters.rs           # one-pole, biquad, allpass cascades
      bow.rs               # friction curve + junction solver + stick/slip state
      string.rs            # one DWG string (bridge seg + nut seg + bow junction)
      body.rs              # biquad-bank body resonator
      instrument.rs        # 4 strings + body; calibrated bow-force band
      performer.rs         # gesture layer: string choice, fingering, bow strokes and lift, legato, vibrato
      section.rs           # N players + humanization
      stage.rs             # stage placement, mic pair, early reflections
      presets.rs           # per-instrument physical data (violin, cello; later viola, bass)
  strings-render/          # CLI: scores -> WAV (+ CSV of internal signals), playability maps
    scores/                # example scores for `play`
  strings-plugin/          # nih-plug wrapper, CLAP export, param + MIDI mapping, egui editor
xtask/                     # `cargo xtask bundle strings-plugin --release` (nih_plug_xtask)
docs/
PLAN.md
```

`strings-dsp` must build and test with no audio I/O. Everything interesting is testable offline.

---

## 3. DSP design

### 3.1 The string (digital waveguide)

The model uses velocity waves. The bow point splits the string into two segments:

```
 bridge ──[ bridge-side delay: β·N/2 ]──(BOW)──[ nut-side delay: (1-β)·N/2 ]── finger/nut
   ↑ reflection filter Hb(z)                                   reflection -1 (later: finger filter)
```

- `N = fs / f0` is the loop length in samples, minus the phase delay of the loop filters at `f0` (tuning compensation).
- **Bridge reflection:** `-g · lowpass`. This sets decay time and brightness. Two loss models (`loss::Loss`):
  - **One-pole** (violin presets): a decay time for the fundamental plus a one-pole lowpass.
  - **Measured** (Phase 1b, cello reference string): a damping curve ζ(f) fitted to pluck data. It is realized as the DC gain × a one-pole × a 2nd-order Butterworth lowpass, fitted per semitone in the constructor. See "Phase 1b results".
- **Finger/nut reflection:** `-1` for open strings. A stopped note is simply a shorter nut-side delay.
- **Fractional delay:** 3rd-order Lagrange on the nut-side delay, because its length is modulated by vibrato, legato and portamento. Thiran allpass sounds cleaner for fixed lengths but produces transients when its length is modulated, so we keep it only as an option for the bridge side.
- **Output:** force at the bridge, `F = Z · (v_in − v_out)`, which then goes through the body filter.
- **Stiffness/dispersion (Phase 1b, done):** a cascade of 16 identical 1st-order allpasses at the nut reflection, with tuning re-compensated. It matters much more for bass and cello than for violin. See 3.6.

### 3.2 The bow junction

At the bow point, the incoming waves from both sides sum to `v_h`, the string velocity there if the bow applied no force. The bow applies force `F`:

```
v_s   = v_h + F / (2Z)          (string velocity at bow)
Δv    = v_b − v_s               (bow minus string)
v_out = v_in(other side) + F/(2Z)   on each side
```

Here `Z = sqrt(T·μ_lin)` is the string's wave impedance.

**Friction curve** (hyperbolic, the Smith/Woodhouse rosin fit):

```
μ(Δv) = sgn(Δv) · ( μ_d + (μ_s − μ_d) · v0 / (v0 + |Δv|) )
F     = F_b · μ(Δv)
```

**Solving it each sample, without iteration.** Let `D = v_b − v_h` and `k = F_b / (2Z)`.

1. **Sticking?** The force needed to make the string move with the bow is `F_stick = 2Z·D`. If we are currently sticking and `|F_stick| ≤ μ_s·F_b`, stay stuck: `v_s = v_b`.
2. **Otherwise, slipping.** For `Δv = x > 0`, the load line and the friction curve meet where
   `x² + (v0 + k·μ_d − D)·x + v0·(k·μ_s − D) = 0`.
   Mirror this for `x < 0`.
3. **Stick/slip hysteresis (Friedlander construction).** When sticking breaks, `c < 0` and the root is unique. While slipping, pick the root **farther from zero**, the one continuous with the previous slip state. Stay slipping as long as a real root exists; when the discriminant goes negative, jump back to stick.

   Without this rule the model chatters between states. This is the most common bug in bowed-string models.

> Verify the signs and the choice of root in the offline prototype against the expected Helmholtz waveform before building on it.

**Tone controls the bow gives us:**

- **`F_b`** (bow force) sets brightness and whether the tone is clean or raucous.
- **`v_b`** (bow speed) sets loudness.
- **`β`** (bow position, `0.04–0.2` of string length from the bridge) sets spectral tilt: sul ponticello at small β, sul tasto at large β. These come for free.

**Bow noise.** While the string slips, the friction force fluctuates at random around the curve: `F · (1 + level · n)`, with `n` band-limited noise of unit RMS (`bow::BowNoise`). Sticking hair adds none, so the noise comes in pulses at each slip, inside the loop. See "Bow noise".

**Aliasing.** The stick/slip transitions are corners, which alias at high `F_b`. Because a DWG is cheap, the option is to run the **whole string at 2× oversampling** rather than only the junction. **Decided (September 2026):** the strings run at 2× by default, the body at the sample rate; see "2× oversampling".

### 3.3 Body

- A bank of biquad resonators fed by bridge force.
- Tuned to each instrument's signature modes. Violin:
  - A0 at about 275 Hz
  - CBR
  - B1− and B1+ at about 450–550 Hz
  - the "bridge hill" at about 2–3 kHz
  - plus a broadband tail
- Cello first: the same mode families sit lower. Phase 2 uses 97 Hz (A0), 173 Hz (the main body resonance, at the wolf note of that cello), 200, 209 and 281 Hz from Zhang, Woodhouse & Stoppani (JASA 2016), T1 at 140 Hz from Bynum & Rossing, and bridge hills at about 1.3 and 2.2 kHz. Damping and levels are estimates.
- **Above the listed modes** a seeded bank of 48 modes (300 Hz–6 kHz, random frequencies, signs and levels around a smooth envelope with the bridge hills) stands in for the dense modal region, following Woodhouse's statistical view. The seed makes the body reproducible, and sections can vary it per player.
- **Fitted to recordings** (Phase 4): the dense modes' envelope, and on the violin, viola and bass the listed modes' levels, are fitted to the Iowa notes' spectral envelope (`compare`, then `fit-body`; see "The body fitted to recordings"). The numbers in the bullet above are the Phase 2 starting point.
- **Alternative:** partitioned convolution with a measured bridge-admittance or radiation IR. It is more realistic, but licensing and data sourcing are open (see caveats).
- For sections, each player gets small random variations of mode frequencies and Q, so the section doesn't sound like one instrument copied 12 times.

**Not commuted synthesis.** Commuted synthesis would pre-filter the excitation with the body. That only works for LTI systems (plucked or struck strings), not for the nonlinear bow. The body stays **after** the string. It will apply to pizzicato later.

### 3.4 Instrument: four strings plus fingering

- Each instrument has 4 `String`s, all running every sample, so open strings can ring sympathetically once string-to-string coupling is added later.
- **String selection:** pick the string that plays the note at the lowest reasonable position. Prefer the current string for legato (hysteresis), and let a param bias toward lower strings (the "sul G" color).
- **Finger position:** sets the length of the nut-side delay. Vibrato and portamento modulate it.
- **Stopping the old note:** when the bow leaves a string, that string keeps decaying naturally. Nothing is cut off.

### 3.5 Instrument data (approximate, refine later)

| Instrument | Strings (tuning) | Scale length | Notes |
|---|---|---|---|
| Violin | G3 D4 A4 E5 | ~328 mm | |
| Viola | C3 G3 D4 A4 | ~370–420 mm | Size varies a lot between instruments |
| Cello | C2 G2 D3 A3 | ~690 mm | Milestone 1. Stiffness matters more. Measured reference for the G string (mdw, see Phase 0–1 notes) |
| Double bass | (C1) E1 A1 D2 G2 | ~1040–1060 mm | Stiffness and dispersion are clearly audible. Longer loops need more delay memory. The E string has an orchestral C extension (1.34 m open) |

Per string we store tension, linear density (which gives `Z`), loss (one-pole or a measured damping curve), bending stiffness and, optionally, torsion. Per instrument we store body modes, bow friction parameters, and typical ranges of `β`, `v_b` and `F_b`.

### 3.6 Bending stiffness and torsional waves

The measured comparison (see "Measured comparison" under Phase 0–1 notes) shows the model's Helmholtz region is about 5× too small. It fails worst close to the bridge (β ≈ 0.02–0.05). Tuning loss and friction closes only part of the gap. Two pieces of string physics are missing, and the paper provides data for both. Add them one at a time. After each, re-run `strings-render measured`, the physics tests and the `schelleng` map.

**Bending stiffness (dispersion).**
- **Physics:** a stiff string's partials are sharp: `f_n = n·f0·sqrt(1 + B·n²)`, with inharmonicity `B = π²·EI / (T·L²)`. High frequencies travel faster, so the Helmholtz corner spreads out instead of staying sharp.
- **For the measured string A T1:** EI = 3.03e-4 N·m², so B ≈ 4.2e-5 and partial 20 is about +15 cents sharp. Violin strings have much smaller B; the cello and bass need it most.
- **Implementation (done):** `filters::DispersionAllpass`, a cascade of 16 identical first-order allpasses (Van Duyne & Smith; Rauhala & Välimäki), after the nut-side delay read. Its coefficient is fitted by golden-section search to the stiff-string partials up to 3.5 kHz, including the bridge lowpass's own phase. Tuning compensation subtracts the cascade's phase delay at `f0` from the nut-side delay.
  - **At the nut, not the bridge.** The cascade adds about 60 samples of delay on the cello G string. On the bridge side that would move the bow's effective position far off at small β, where the bridge segment is only a few samples long.
  - **Per-semitone table.** B scales as f² for stopped notes, so the constructor fits one coefficient per semitone, up to 3 octaves above the open string. `set_frequency` interpolates between them, so vibrato stays cheap and smooth.
  - **Accuracy limit.** Identical sections follow the n² curve only up to a bandwidth that grows as M^(1/3). On the cello G string, 16 sections give ≤ 1.5 cents error up to 2.5 kHz and ≤ 3.2 cents up to 3.4 kHz. Above that the model is under-dispersed. Distinct coefficients fitted by least squares got stuck in local minima. An Abel–Välimäki–Smith biquad design missed the low partials by tens of cents, because its staircase phase is too coarse near DC. Revisit if the bass needs more (its B is about 5× larger).
- **Data:** a `bending_stiffness` (EI, N·m²) field on `StringSpec`. B is derived from it.
- **Checks:**
  - a pluck test that the partial frequencies follow `f_n` within a few cents;
  - tuning still within ±1 cent;
  - the effect on the measured comparison.

**Torsional waves.**
- **Physics:** the bow grips the string's surface, so friction also twists the string. At the contact point the string's velocity is the transverse velocity plus the rolling velocity `r·ω`. The bow therefore sees the transverse and torsional impedances in parallel. (Correction: an earlier version said the torsional impedance is much lower than Z. Referred to the surface it is roughly `κ·μ·c_t`, several times Z, because torsional waves are about 5× faster. The string gives way somewhat more easily at the bow: Z·Z_t/(Z+Z_t) ≈ 0.77·Z.) Woodhouse and co-workers found torsion affects the stick/slip trigger, attacks and the minimum bow force.
- **Implementation:** a second, short waveguide per string for the torsional waves, coupled only at the bow junction.
  - The junction solve keeps its closed-form quadratic and Friedlander hysteresis. Only the load line changes: `v_s = v_h + v_h,tors + F·(1/(2Z) + 1/(2Z_t))`, with `Z_t` the torsional impedance referred to the string surface.
  - The torsional fundamental is typically several times the transverse one, and its damping is much higher (low Q).
  - Bridge force (the output) still comes from the transverse waves only.
- **Data:** the paper lists the manufacturer's `Z_to` (0.039 for A T1, in units of g·m²/s²), which the text calls "torsional stiffness". Read as GJ, it gives GJ ≈ 0.13·EI, and neither reading yields a plausible surface impedance, so it isn't used. The preset instead uses estimates, documented in `presets::reference`:
  - f_t = 5.5·f0, as measured on a steel cello G string by Mores (PLOS One 2019: 543 Hz vs 98 Hz);
  - Z_t = κ·μ·c_t with κ = 0.6, giving 3.5 kg/s (3.3·Z);
  - Q = 50, since Mores finds torsional Q about 10× below the transverse Q.
- **Implementation (done):** `TorsionSpec` on `StringSpec`, with two round-trip delay lines, one on each side of the bow, both reflecting with −1. The damping has constant Q (Woodhouse & Loach 1999): a loss filter at the nut-side reflection, fitted per semitone like the measured transverse loss (see "Constant-Q torsional loss"). Until then the loss per period was the same for every mode, so mode k had k times the fundamental's Q. The torsional lines scale with the stopped pitch. The bridge-side line is clamped to 2 samples, which matters only below β ≈ 0.02.
- **Cost:** roughly one extra short delay line and a filter per string. That is cheap next to the transverse loop, but it matters for 12-player sections.
- **Checks:** Helmholtz motion still passes the physics tests, and the measured comparison shows the effect, especially the lower force limit and small β.

---

## 4. Performance layer (gestures → physics)

A physical model is only as playable as its control mapping. This layer turns MIDI into bow speed, bow force, bow position and finger position over time.

### 4.1 Controls (initial)

| Control | Default CC | Maps to |
|---|---|---|
| **Dynamics** | CC11 (expression pedal) | A path through (`v_b`, `F_b`, `β`) space. See 4.2. As in SWAM, there is no separate expression gain: the plugin's Volume is the only gain |
| **Vibrato depth** | CC1 (mod wheel) | Depth of finger-position modulation. 0 means none. Rate is a parameter |
| **Pressure** | — (parameter) | Flautando at 0, normal at 0.5, scratch at 1. See 4.2 |
| Velocity | — | Detached notes: the attack (how fast the bow gets going and how hard it bites). Legato: the transition, from instant (hard) to a slow portamento (soft). See 4.3 |
| **Polyphony** | — (parameter) | Mono, or double stops (4.3) |
| **Fingering** | — (parameter) | Near the nut & open, mid position, near the bridge: which strings play (4.3) |
| **Bow lift** | Keyswitches: white keys from the first C below the instrument's lowest note (cello: C1 off string, D1 on string; violin: C3, D3; C4 = middle C). Also a parameter | What the bow does at the end of a note (4.3) |

Later: MPE (per-note pressure and pitch) and CLAP note expressions.

### 4.2 Keeping it playable: Schelleng-normalized force

Raw `F_b` is unplayable: the usable range moves with pitch, bow speed and bow position. So the **dynamics** control drives bow speed, and force is expressed as a position `p ∈ [0,1]` between the playability limits, interpolated in log space:

```
F_max = 2·Z·v_b / ((μ_s − μ_d)·β)
F_min = Z²·v_b / (2·R·β²·(μ_s − μ_d))      (R = effective bridge resistance)
F_b   = F_min^(1−p) · F_max^p
```

- These are the corrected formulas; the research doc has the β dependence the wrong way round.
- The default `p` is around 0.4–0.6.
- A "bow pressure" parameter biases `p`: toward 0 gives a flautando, surface sound; toward 1 gives a crunchy, pressed sound. **As built:** the pressure control runs from flautando (band position 0, the lower edge) through normal (0.65 at the control's middle) to scratch (1.3, above the band, raucous); see "Phase 3 notes: playing like SWAM".
- As players do, the bow also moves slightly toward the bridge at high dynamics.
- **The bow keeps its distance from the bridge** (as built): β is a fraction of the *vibrating* length, so high on a string the mapped β would put the bow a few mm from the bridge. The performer keeps it at least `bow_distance · Z` away (0.024 m per kg/s: 3.5 cm on the C string, 1.4 cm on the A), so β rises in high positions. See "High positions: the bow's distance from the bridge".
- **The bow keeps its place on the string while the finger moves** (as built): its distance from the bridge (β × the vibrating length) stays put when the note changes, so β of the new length changes instead, and it drifts to the new note's position over `bow_follow` (0.15 s). A trill is bowed at a place between its notes rather than jumping with each one, and a slide leaves the bow where it was. β stays within 25% of the note's own position, so a long slide doesn't leave the band. The bow lands at the note's position for a stroke from off the string; strings the bow doesn't touch wait at their note's position.
- `R` has to be derived from the bridge loss filter. Calibrate empirically in the renderer.
- **Measured in Phase 1:** the simulated upper edge follows `F_max` closely, but the lower edge sits about 5–10× above the formula's `F_min` (same 1/β² slope). The dynamics mapping should anchor on `F_max` or on a calibrated lower edge, not on the raw `F_min`.
- **Against a real string** (see "Measured comparison" under Phase 0–1 notes): the measured lower edge follows roughly 1/β (fitted exponents −0.9 to −1.3), not 1/β². This is in line with Schoonderwaldt et al. (2008) and Mansour et al. (2017). A calibrated lower edge should be fitted to measurements, not derived from the formula.
- **As built (Phase 2):** `instrument::ForceLimits` stores both edges per string as `F = c·Z·v_b·β^α`, fitted by `strings-render calibrate` to the model's own simulated maps (the performer needs forces where the *model* plays Helmholtz, and promptly: a cell counts only if it is Helmholtz within 0.15 s; see "Force band from prompt settling"). The measured lower edge is lower still; see "Phase 2 notes". The performer uses p = 0.65, tilted to 0.8 at pp and 0.5 at ff, with a bite at the start of each stroke and a slow wander (see "Phase 3 notes: tuning and first listening"). Once a quiet stroke is going, the cello and bass ease down the band (see "Soft, dark pp").

### 4.3 Articulations: one way of playing

As in SWAM, there are no articulation modes. How a note is played follows from whether it overlaps the last one, its velocity, its length and the bow lift (off string or on string, a parameter and keyswitches).

| Played as | Physics |
|---|---|
| **Detached note** (no overlap) | A new bow stroke; the direction alternates (or as a bow keyswitch set it). Velocity sets the attack: off the string, the bow lands (12 ms) and accelerates over 35–150 ms with a bite of 0.2 × velocity in the band; on the string, the force grips for 10 ms before the bow moves, then it accelerates over 65–300 ms with a bite of 0.38 × velocity lasting 70 ms (hard presses are a martelé). Quiet dynamics slow both (`pp_attack`). The stroke lasts as long as the key, at least 60 ms |
| **Détaché** (separate notes, sustain pedal down) | As in SWAM: under the pedal a key let go leaves the bow going, so the next separate note starts with a bow change (12 ms) instead of a lift and a landing. Overlapping notes still slur. The pedal let up ends the stroke; MIDI "all notes off" lets it up |
| **Bow change within a note** | A bow keyswitch (the white keys after the bow lift's: cello E1 down-bow, F1 up-bow) against the stroke reverses the bow, its velocity setting the new attack; with the stroke, or between notes, it sets the next stroke's direction. The dynamics brought to zero and resting there for `auto_bow_change` (0.15 s) also changes bow, once, at that quiet moment (SWAM changes bow when expression rests at zero); a stroke begun at zero doesn't |
| **End of a note, off string** | The bow lifts as it slows (150 ms), and the string rings on, stopped or open: the finger stays down until the next note goes to another string. A note shorter than that is thrown off as quickly as it was played, still moving: spiccato-like. Phase 4: a bouncing bow (a mass on a spring) |
| **End of a note, on string** | The bow **stops on the string** (75 ms) and stays there until the next note; the stopped bow damps the string, so short notes are staccato |
| **Legato** (overlapping notes) | No new bow stroke. The landing note's velocity sets the transition: pressed hard, a finger drops or lifts within the hand's position (6 ms) and a shift of the hand slides quickly (20 ms); below velocity 0.8 the finger slides, up to 250 ms at velocity 0 (portamento), and stays on its string wherever the string can play the note (a finger can't slide across strings). From an open string the finger lands at the nut and slides up; to one it slides down and lifts. Pressed hard, to or from an open string the finger is placed. A note on another string is a crossing: the bow force ramps from the old string to the new one |
| **Double stops** (overlapping notes, double-stop polyphony) | Two notes on adjacent strings, bowed together, if one hand can reach both (stopped notes within 4 semitones of each other). A third note leads on from the nearer note and pairs with the farther one (a line over a held note). A pair that doesn't fit plays legato |

All CC and keyswitch assignments are defaults; they become user-adjustable once there is a GUI.

### 4.4 Vibrato

- Modulates finger position, which modulates the nut-side delay.
- Depth ±0.1–0.35 semitones, set by the CC. Rate 5–7 Hz, as a parameter.
- Humanized: slow random drift in rate and depth, and a delayed onset on long notes.

### 4.5 Deferred

Tremolo, trills, chords of three or four strings, pizzicato (pluck exciter on the same string engine), harmonics, col legno, con sordino, automatic bow changes when the bow runs out.

---

## 5. Sections (later phase)

A section is **N independent players**. Each player is a full `Instrument` plus `Performer`, with its own humanization, placed on a stage. The working checklist is [docs/SECTIONS.md](docs/SECTIONS.md).

**Humanization (per player, seeded so results are reproducible):**
- Detune (a few cents, drifting slowly)
- Onset and bow-change timing jitter (~5–30 ms)
- Vibrato rate, depth and phase
- Dynamics offset
- Bow position and pressure bias
- Legato transition time
- Body mode variations

**Stage placement:**
- Each player has a 2D stage position and gets its own delay (distance and ITD), panning (ILD), distance attenuation, and a gentle air-absorption lowpass.
- Optionally a few early reflections; the late reverb is left to the user's reverb plugin.
- The solo mode uses the same placement code with N = 1.

**Cost:**
- One DWG string is on the order of tens of operations per sample. 12 players × 4 strings × (string + bow) is cheap.
- The body bank dominates: 12 × 30 biquads.
- **SIMD over players:** all players share the same structure, so process 4 or 8 players per vector lane. The delay reads at different lengths are gathers.
- Measure before optimizing.

**Divisi**, meaning polyphonic sections that split players across notes, comes after mono sections work.

---

## 6. Real-time and engineering rules

- No allocation, locks or I/O in `process()`. Checked by `tests/realtime.rs`, a counting global allocator around a scripted performance (Phase 2); [`assert_no_alloc`](https://crates.io/crates/assert_no_alloc) remains an option for the plugin's audio thread.
- Allocate all delay lines at `initialize()` for the lowest note at the highest supported sample rate.
- Handle denormals: flush-to-zero, or add a tiny DC or noise offset in the feedback loops.
- Smooth every control that reaches the physics. A jump in `F_b` or `v_b` is audible and can destabilize the model.
- Clamp `F_b`, `v_b` and `β` to safe ranges regardless of input.
- Add a per-voice NaN/Inf guard that resets the voice (debug assert plus a silent recovery in release builds).
- Support common sample rates (44.1–192 kHz). All physical parameters are in SI units and converted using `fs`.

### Testing

- **Unit tests:** friction solver roots, including hysteresis cases; delay tuning accuracy (measure `f0` of a free string within ±1 cent); stability with the bow off (energy only decays).
- **Physics checks in the renderer:**
  - Bridge force in the Helmholtz regime is sawtooth-like.
  - Sweeping `F_b` across `F_min` and `F_max` reproduces the expected breakdowns: surface sound below, raucous noise above.
- **Golden renders:** gesture scripts rendered to WAV and compared with a tolerance. They catch regressions and are for listening, not exact equality.
- **Benchmarks** with `criterion`: cost per voice and per section.

---

## 7. Phases

Each phase ends with something audible.

| Phase | Deliverable | Done when |
|---|---|---|
| **0. Scaffold** ✅ | Cargo workspace, `strings-dsp`, `strings-render` CLI writing WAV (`hound`) plus CSV of internal signals | `cargo test` passes; the renderer writes a plucked (free) string to WAV |
| **1. One bowed string** ✅ | DWG string, bow junction and hysteresis, bridge loss, tuning compensation | A violin A string produces stable Helmholtz motion; the Schelleng sweep behaves as expected; tuning within ±1 cent |
| **1b. Measured string physics** ✅ | Bending stiffness, torsional waves (3.6) and measured frequency-dependent damping, checked against the measured cello G string with `strings-render measured` | Simulated Helmholtz region close to the measured one at all three bow speeds (area within about ±30%, Helmholtz present at small β); physics tests still pass. **Met with the bow hair and its width:** 83–86% of the measured area; at β < 0.05, 62% of the measured points. The string physics alone gets about half ("Phase 1b results"); see "The bow's width" |
| **2. Solo cello (offline)** ⚠️ | Cello presets for C2 G2 D3 A3 (G from the measured string; the others from published string data), 4 strings, string selection, fingering, legato, vibrato, biquad body, performer layer with the 4 articulations and its force mapping calibrated on the measured limits (4.2), bow-hair damping so a bow stopped on the string silences it quickly | Scripted phrases (scales, legato lines, staccato runs) sound like a cello, not a synth. **Built and tested objectively; the first listening is done** ("Phase 3 notes: tuning and first listening"); "sounds like a cello" stays open. See "Phase 2 notes" |
| **3. CLAP plugin** ⚠️ | nih-plug wrapper, CLAP-only export, CC1/CC11/vibrato mapping, keyswitches, parameters, real-time safety | Playable in Bitwig and Reaper; no allocations in the audio thread; CPU cost measured. **Built:** plugin, editor and standalone app; engine at 3.5% of real time dry, 4.2% on the stage (strings at 2×); a tuning window for the model's numbers. **Plays in Bitwig; not yet tried in Reaper.** See "Phase 3 notes" |
| **4. Realism pass** | Thermal friction, finger damping at note changes (a first, frequency-independent version is in: "Phase 3 notes: tuning and first listening"), the bow's width, bow noise, oversampling decision, bouncing-bow spiccato, sympathetic string coupling | A/B against recordings; clear improvement on attacks and legato transitions (the mdw attack data gives a measured target for attacks). **Started:** `strings-render compare` against the Iowa cello notes ("Phase 4: comparison with recorded notes"); the bow's width on the cello and bass ("The bow's width"); bow noise ("Bow noise"); thermal friction, opt-in ("Thermal friction"); attacks against the mdw Guettler diagrams, which set the cello's μs to 0.9, and the bass's with it ("Attacks against measured data"); the violin's, viola's and bass's recorded sets ("The other instruments' recorded notes"); the spectral envelope against the recordings, and all four bodies fitted to it ("The body fitted to recordings") |
| **5. More instruments** ⚠️ | Violin, viola and double bass presets and bodies | Each instrument is convincing across its range. **All four built** (objective checks pass), and each has been heard once; the viola's and bass's recorded notes now compare too. See "Phase 5: the violin" and "Phase 5: viola and double bass" |
| **6. Sections** ⚠️ | N-player engine, humanization, stage placement, SIMD across players | 12-player section within the CPU budget; sounds like a section, not a chorus effect. **Built** (docs/SECTIONS.md Phase A): up to 12 humanized players on a stage with early reflections, in the plugin; 12 players at 27.7% of real time without SIMD (over the 25% budget). Not yet heard in a host |
| **7. Extended techniques** | Tremolo, trills, pizzicato, double stops, harmonics, mutes, MPE | — |

**CPU budgets** (per instance, one core at 48 kHz):
- Solo: < 5% (raised from 3% for 2× oversampling; the engine measures 3.5% dry, 4.2% on the stage)
- 12-player section: < 25% (players may run at 1×)

---

### Phase 0–1 notes

- Run the tests with `cargo test`. Renderer: `cargo run --release -p strings-render -- <pluck|bow|schelleng|measured> --help`. Output goes to `out/` (gitignored).
- Free-string tuning is within ±1 cent for all four violin strings, stopped up to an octave, at 44.1, 48 and 96 kHz.
- Moderate bowing gives one slip per period with a slip fraction close to β, and the bridge force has the sawtooth shape of Helmholtz motion.
- `strings-render schelleng` prints the simulated regime map next to Schelleng's prediction. About 80% agree; the disagreement is the lower-limit offset described in 4.2.
- String loss is split between the bridge and nut reflections in proportion to segment length, so a segment trapped by a sticking bow still decays.

### Measured comparison: cello G string on a monochord (September 2026)

Data: mdw Vienna string "A T1" (steel/tungsten cello G2, 98 Hz, rigid terminations), 2000 points × 3 bow speeds, with bow force, bow velocity and bridge force at 50 kHz. See `docs/Violin Reference Recordings.md` §3 and Lampis, Chatziioannou & Scavone, Proc. Mtgs. Acoust. 58, 035013 (2025). Run `cargo run --release -p strings-render -- measured` (about 15 s). It simulates every measured point at its own measured (β, F_b, v_b) with `presets::reference::MONOCHORD_CELLO_G_A_T1` and classifies both bridge-force signals with the same classifier.

- **Classifier:** `analysis::classify_bridge_force` uses the bridge force only: sharp drops per period, how sharp they are, and periodicity after removing DC. On simulated violin strings it agrees with the contact-state `classify` on 77–93% of points. On the measured data it reproduces the paper's Fig. 2a qualitatively. It counts the paper's "multiple flyback" and "S-motion" as multi-slip or raucous.
- **The model's Helmholtz region is far too small:** about 130 simulated vs 700 measured Helmholtz points at 0.05 and 0.1 m/s, and 97 vs 392 at 0.2 m/s. At v_b = 0.1 m/s and β = 0.1 the measured band is 0.31–1.89 N, the simulated one 0.95–1.39 N. H/not-H agreement is 70–82%.
- **Small β is where it fails most:** measured Helmholtz extends up to 4 N for β ≈ 0.02–0.05. The model gives only multi-slip there, whatever the loss or friction settings.
- **Sensitivity** (Helmholtz points, 0.05 / 0.1 / 0.2 m/s; default 132 / 129 / 97):
  - More high-frequency loss helps most. Loss pole 0.7 gives 258 / 225 / 148, and 0.8 gives 353 / 270 / 192; it saturates at about half the measured area.
  - Friction (μ_s 1.0, μ_d 0.2, v0 0.05–0.2) and the fundamental's t60 change it by ±30 points at most.
- **Damping doesn't match either:** with pole 0.5 the model's modal damping is ζ ≈ 3e-4 at mode 10. The measured string has about 8.5e-4 there (corrected from an earlier rough reading of 1.4e-3), rising steeply with mode number (paper Fig. 1). A one-pole loop filter can't follow that curve. (Fixed in Phase 1b with the measured loss model.)
- **Not changed:** model defaults. Pole 0.5 was chosen for the violin at 48 kHz, and loss alone doesn't close the gap.
- **Likely missing physics:** bending stiffness (EI = 3.0e-4 N·m² is measured) and torsional waves (Z_to is given in the paper), both planned in 3.6. Beyond those: finite bow width and bow-hair compliance (§8). Re-run `measured` after each one. (Phase 1b tried the first two; neither helped. See below.)

### Phase 1b results: stiffness, torsion and damping (September 2026)

Helmholtz points per bow speed (0.05 / 0.1 / 0.2 m/s; measured 701 / 700 / 392). The last column counts Helmholtz points at β < 0.05, where the measurement has 529 of 2400; the figure in brackets is how many of those agree with the measurement.

| Model | Helmholtz points | at β < 0.05 |
|---|---|---|
| **One-pole loss (Phase 1)**, flexible, no torsion | 132 / 129 / 97 | 32 (31) |
| + bending stiffness | 51 / 56 / 62 | 15 |
| + torsion only | 13 / 26 / 31 | 21 |
| + both | 12 / 26 / 43 | 6 |
| One-pole, pole 0.8, flexible, no torsion | 353 / 270 / 192 | 134 (44) |
| One-pole, pole 0.8 + stiffness + torsion, Q 15 | 260 / 273 / 198 | 90 (83) |
| **Measured damping**, flexible, no torsion | **337 / 277 / 165** | **156 (143)** |
| + bending stiffness | 249 / 211 / 90 | 153 (134) |
| + torsion only | 170 / 187 / 135 | 187 (171) |
| + both (preset default) | 119 / 154 / 107 | 127 (108) |
| + both, torsion Q 15 | 225 / 242 / 117 | 182 (164) |
| Measured damping, curve exponent 2.5 / 4.5 (flexible, no torsion) | 320 / 265 / 157 and 338 / 270 / 158 | 146 / 159 |

**Stiffness and torsion**
- **Verified:**
  - partials of the stiff string within 2 cents of `f_n` up to 2.5 kHz (`stiff_string_partials_are_sharp`, run with one-pole loss, because with the measured damping the upper partials die too fast to measure their pitch);
  - tuning within ±1 cent with all three, up to two octaves and at 44.1–96 kHz;
  - violin behavior and Schelleng maps unchanged, byte for byte (violin presets use the one-pole loss and have neither stiffness nor torsion).
- **Both cost Helmholtz points, through real extra slips.** The contact state confirms the extra slips; the classifier isn't miscounting. With one-pole loss at β = 0.1, v_b = 0.1 m/s, F_b = 1.1 N the flexible string has 1.0 slips per period, stiffness 4.0, torsion 5.3. Dispersion spreads the Helmholtz corner into precursors. Torsional reflections return about 5.5 times per period, sharp, because the torsional loop has no frequency-dependent loss. Both trigger early slips.

**Measured damping**
- **Data:** ζ per mode digitized from the paper's Fig. 1 (A T1), listed in `presets::reference`.
- **Curve:** `ζ(f) = 2.9e-4 + 5.4e-4·(f / 1 kHz)^3.54`, within ±17% rms (log) of the data, excluding mode 3, which has a wide spread.
- **Filter fit:** a grid search over (pole, cutoff), with the DC loss in closed form, fits the loss per period of the partials up to 8 kHz, capped at 3 nepers.
- **Design limit:** pluck-measured ζ is within 0.68–1.45 × the curve over modes 1–15, the same at 48 and 96 kHz (`measured_damping_sets_partial_decay`). The fit is too low around modes 2–4 and too high around modes 9–11.

**What we learned**
- **Damping is the biggest single improvement:** 2.5× the Helmholtz points of Phase 1 and 5× at small β, with H/not-H agreement 81 / 76 / 85%. It doesn't depend on the extrapolation above 1.7 kHz (exponent 2.5–4.5 changes little), so it is set by the measured modes. A clean band now forms at β ≈ 0.05–0.12, 1–3 N.
- **Still about half the measured area.** Two gaps remain. The lower force limit sits too high: the measurement has Helmholtz motion down to 0.2–0.3 N at β ≈ 0.1–0.2, where the model gives multi-slip or no slipping. At β < 0.05, measured Helmholtz motion reaches 2–4 N; the model only shows it in scattered cells.
- **Stiffness and torsion still subtract overall, even with realistic transverse damping.** Torsion helps at small β (187 against 156) and costs at larger β. More torsional damping (Q 15) recovers most of the loss. Mansour, Woodhouse & Scavone (2017) found torsion changes the minimum bow force less than Schelleng's correction predicts (docs/Literature.md). The torsional Q and a frequency-dependent torsional loss are the least-grounded parameters here; the torsional frequency rests on one measurement of a different steel cello G string (Mores 2019).
- **Model defaults:** the reference string keeps all three (measured EI and damping, estimated torsion), since it describes the physics rather than the best score. The ablations above show each one's effect.
- **Next candidates:**
  - frequency-dependent torsional loss (a lowpass in the torsional loop), with the torsional Q from the literature rather than fitted (done: "Constant-Q torsional loss");
  - finite bow width and bow-hair compliance (§8);
  - the friction model (thermal friction, Phase 4), which Woodhouse links to the minimum bow force.
- `strings-render measured` takes these overrides:
  - `--t60` / `--loss-lowpass`: switch to the one-pole loss;
  - `--damping-exponent`;
  - `--bending-stiffness`, `--no-torsion`, `--torsion-impedance`, `--torsion-ratio` and `--torsion-q`.

### Constant-Q torsional loss (September 2026)

The torsional loop used to lose the same per period at every frequency, so torsional mode k had k times the Q of the first: at Q = 50, mode 10 rang with Q 500. Measured torsional Q is about the same for every mode (Woodhouse & Loach 1999, via Woodhouse & Galluzzo 2004), so mode k should lose `π·k/Q` nepers per period. The "sharp torsional reflections" that set off extra slips (Phase 1b results) were those under-damped high modes.

- **Implementation:** the torsional loop gets the same filter as the measured transverse loss (a one-pole times a Butterworth lowpass), fitted by `DampingCurve::design` to a flat ζ = 1/(2Q). It sits at the nut-side reflection, and its phase delay at the torsional fundamental comes off the nut-side line. It is fitted per semitone with the other designs, so stopped notes and `apply_design` retune it. The fit holds Q within 43–59 over the torsional modes up to 8 kHz at Q = 50 (`constant_q_design_follows_the_modes`).
- **Measured string** (`strings-render measured`; Helmholtz points at 0.05 / 0.1 / 0.2 m/s, then at β < 0.05, agreeing cells in brackets; measured 701 / 700 / 392 and 529):

| Model | Before | Constant Q |
|---|---|---|
| Measured damping + stiffness + torsion (preset) | 119 / 154 / 107, 127 (108) | 211 / 194 / 104, 174 (155) |
| Measured damping + torsion (flexible) | 170 / 187 / 135, 187 (171) | 263 / 276 / 159, 188 (173) |
| Preset + cello bow hair | 304 / 317 / 181, 274 (261) | 335 / 325 / 186, 287 (274) |

  H/not-H agreement: 70 / 71 / 84% → 75 / 72 / 84% (preset), 79 / 76 / 85% → 81 / 76 / 85% (with hair). For comparison, flexible without torsion has 337 / 277 / 165 and stiffness without torsion 249 / 211 / 90.
- **What changed:** torsion alone now costs about 10% of the flexible string's Helmholtz points, where it cost 37%, and it still adds some at small β. The default string gains a third, most of it at the slow bow speeds. With the bow hair the gain is small (+5%), and the area is 47% of the measured one: the hair already absorbed much of what the torsional reflections set off. The lower force limit moves down only a little, and the gap to the measured one stays (STATUS.md).
- **Cello:** `calibrate` finds 1340 Helmholtz cells of 5376 (1267 before). Refitted, the edges are c = 7.4–12.1 (upper, β^−0.44 to β^−0.57) and β^−1.0 (C) to β^−1.5 (A) (lower); the G's band at β = 0.1, v_b = 0.1 m/s becomes 0.9–3.0 N (was 1.0–3.1), and band positions 0.5–0.8 give Helmholtz motion in 94–98% of cells (was 92–97%). The `schelleng --instrument cello` maps at 0.1 m/s gain Helmholtz cells on every string (C 19 → 22, G 21 → 27, D 36 → 43, A 40 → 47), all from multi-slip; the raucous region is unchanged.
- **Listening:** A/B renders of `phrase`, `legato` and `staccato` (old force limits in both): a clear improvement by ear.
- **Cost:** one more one-pole and biquad per string. The plugin engine goes from 2.2% to 2.6% of real time, the renderer from 2.4% to 2.8%, and building a cello from 25 to 28 ms.
- **The steady-state refit was not used.** It reaches lower (at the performer's β, 0.07–0.115, the lower edge moves 0–20% down and the upper 2–7%), so the same band position means 3–8% less force on every string. Steady notes stay Helmholtz, but quiet attacks on the open G lose their margin: with the bow's wander, one pp attack took 0.31 s instead of 0.08 s (`notes_stay_helmholtz_while_the_bow_wanders`). Moving the normal pressure (0.65 → 0.7, which gives the old force back) or the dynamics tilt (0.15 → 0.18–0.2) only moved the failure to other notes (C2 and E2 attacks at ff, E5 and C5 pitch). The attacks need more margin than the steady band shows; see "Force band from prompt settling" for the fix.
- **C5 locks to whole-sample periods at 48 kHz.** Its period is 91.7 samples; at mf the bowed string holds 92 samples (−5.0 cents) for 0.1–0.25 s at a time, and the ear pulls it back in steps, so 50 ms windows read −5 to +8 cents while the mean slip period is on target. The old code did the same, less often: the undamped torsional ringing seems to have dithered the slip onset. At 96 kHz both versions stay within ±0.3 cents. See the open question on oversampling.
- **Pitch measurement fix:** `analysis::measure_partial` jumped from a phase hop of 8 periods straight to the whole signal. At C5 the bowed period alternates between 91 and 92 samples (19 cents apart), so the short hop could be more than 2 cents off and the long one then unwrapped a whole cycle wrong (4.2 cents over 0.8 s). The torsion change moved C5 onto that edge in the performer test. The hop now grows at most 4× per pass.

### Force band from prompt settling (September 2026)

The steady-state refit after the constant-Q torsional loss put quiet open-G attacks in trouble (see above). Tracing the failing attack: at 1.035 N (the refit's force; 1.11 N with the old band) the string settled into a stable double slip (two slips per period) for 250 ms before Helmholtz motion took over. Near the lower edge the string is bistable. `calibrate` classified each cell from 0.4 s on, so a cell that spent 300 ms in double slip still counted as Helmholtz, while the performer needs Helmholtz within 0.15 s.

- **Change:** `calibrate` counts a cell as Helmholtz only if it is already Helmholtz from 0.15 s (the bow's speed ramps up over 0.05 s) as well as from 0.4 s on. The band-position check uses the same rule.
- **Result:** 977 Helmholtz cells of 5376 (1340 counting slow settling). Against the old band, at the performer's β (0.07–0.115) the lower edge rises 14–49% and the upper falls 3–10%. The force at normal pressure (p = 0.65) moves only −6% to +2%, so normal playing is where it was tuned by ear. For the G at β = 0.1, v_b = 0.1 m/s the band is 1.3–3.0 N. Band positions 0.5–0.8 give prompt Helmholtz motion in 90–98% of checked cells.
- **Robustness across seeds** (`notes_stay_helmholtz_across_wander_seeds`: the range check with the bow's wander, 30 notes × 24 seeds, counting failed checks): the steady-state refit 35, the old band 14 (the default seed happened to pass), the prompt-settling band 4. The weakest note is still the open G at pp (3 failures, one never settling), plus one C5 at ff 21 cents off: one whole-sample period (STATUS.md).

### 2× oversampling (September 2026)

At 48 kHz a high note's slip snaps to whole samples: C5 (91.7 samples) held 92-sample periods (−5 cents) for 0.1–0.25 s, and with the bow's wander a C5 at ff could end 13–21 cents off. At 96 kHz it stays within ±0.3 cents.

- **Implementation:** `Instrument::new(spec, sample_rate, oversampling)` runs the four strings at `oversampling` × the sample rate (1 or 2), holding the performer's bow inputs over both steps, and decimates the summed bridge force with a 63-tap halfband FIR (Kaiser β = 7: flat to 20 kHz, at least 65 dB down from 28 kHz; `filters::HalfbandDecimator`). The body stays at the sample rate. `PerformerSettings::oversampling` sets it (default 2), fixed once the performer is built. String designs must be fitted at `Instrument::string_sample_rate`; the plugin publishes it for the tuning window. `strings-render play --oversampling 1|2` compares them.
- **Force band:** refitted at 96 kHz (`calibrate --sample-rate 96000`): within a few percent of the 48 kHz fit on C, G and D, the A string's upper edge 2–8% lower. Band positions 0.5–0.8 give prompt Helmholtz motion in 91–99% of checked cells.
- **Quiet attacks:** at 2× the open G's pp attack got worse (7 of 24 wander seeds slow or stuck in double slip, against 2 at 1×). Swept over 48 seeds, the lever was the attack's acceleration, as Guettler's attack diagram predicts: at low force the bow must accelerate gently. `pp_attack` 0.8 → 1.6 (a pp attack takes 2.4× the time of an ff one, against 1.7×) gives no slow attack for any of ten notes C2–E5 at pp, mf or ff on 48 seeds, at the cost of 5–20 ms later Helmholtz at pp. Raising the pressure tilt to 0.25 nearly did the same (3 of 48); the bite made it worse (0.4: 24 of 48).
- **Robustness** (`notes_stay_helmholtz_across_wander_seeds`, 720 notes): 0 failed checks at 2×; at 1× 3, all pitch (C5 at ff 13–21 cents, E4 at pp 10 cents).
- **Cost:** the plugin engine 2.5% → 4.8% of real time, the renderer 2.7% → 6.4%. Building a cello still takes about 27 ms. The solo budget is now 5%.

### Phase 2 notes: solo cello (September 2026)

Run `cargo run --release -p strings-render -- play <scale|legato|staccato|phrase|file.score> -o out/x.wav` (score format in `strings-render/src/score.rs`, examples in `crates/strings-render/scores/`). The whole instrument (4 strings, body, performer) runs at about 2.4% of real time on one core at 48 kHz, measured by the renderer (no `criterion` benchmark yet).

**Cello presets** (`presets::cello`)
- G is the measured string A T1. C, D and A take Larsen Standard tensions (700 mm) and share the G's bending stiffness, damping curve and torsion estimates (f_t = 5.5·f0, Z_t = 3.3·Z, Q 50).
- The damping floor is raised by 7e-4 for energy lost into the body (the monochord has rigid terminations): the open G decays in about 11 s instead of 39 s. An estimate.

**Bow hair** (`string::BowHair`, off by default, on in the cello preset)
- A spring and dashpot (Kelvin–Voigt) between the bow stick and the contact. It keeps the closed-form junction solve and the Friedlander hysteresis: the bow sees `v_h + k·x/R` and an admittance raised by `1/R`; the deflection is integrated with backward Euler.
- **It doesn't silence a stopped bow.** With plausible values (k = 10⁴–10⁵ N/m, R = 3–30 kg/s) the spring reactance k/ω dominates at the string's frequencies, so little energy reaches the dashpot; soft hair lets the string ring as if the bow were lifted. More fundamentally, a point damper near the bridge barely reaches the low modes.
- **What stops a note is the stroke:** if the force eases with the bow speed as the bow decelerates, the string stays in the Helmholtz band and its amplitude follows the bow down (−30 to −55 dB within 100 ms of the stop, against a few dB with the force held). The performer does this (force band evaluated at the current speed, with a 15% floor).
- **It enlarges the Helmholtz region** of the measured string: `measured --hair-stiffness 1000 --hair-damping 3` gives 304 / 317 / 181 Helmholtz points (0.05 / 0.1 / 0.2 m/s), against 119 / 154 / 107 with a rigid bow (measured 701 / 700 / 392), and 274 at β < 0.05 (261 agreeing with the measurement; rigid bow 127, measured 529); H/not-H agreement 79 / 76 / 85%. It plateaus for k = 300–1000 N/m at R ≈ 3 kg/s. R is close to the wave impedance of the hairs in contact; k is softer than the hair ribbon alone and stands for the whole contact. Both are fitted, not measured.

**Force calibration** (`strings-render calibrate`, about 6 s)
- Upper edges follow β^−0.5 (c = 8.0–11.4), not Schelleng's β^−1; lower edges β^−0.9 (C) to β^−1.6 (A). For the G at β = 0.1, v_b = 0.1 m/s the band is 1.0–3.1 N, the measured one 0.31–1.89 N.
- Band positions 0.5–0.8 give Helmholtz motion in 92–97% of checked open-string cells.
- Superseded: the band is now fitted to prompt settling (see "Force band from prompt settling").

**Pitch of the bowed string**
- The model's bowed pitch drifts from the string's tuning: it flattens with force (−5 to −15 cents at p 0.65 for β < 0.12) and high stopped notes sharpen at low force (stiffness: the Helmholtz pitch locks above f0).
- **Flat zone at β ≈ 0.124–0.156** (1/β ≈ 6.4–8.1) on the cello strings with hair and torsion: up to 45 cents flat, still one slip per period. It needs torsion; hair alone has a raucous zone at β ≈ 0.09–0.116 instead; the torsional Q (8–50) doesn't change it. The dynamics mapping keeps β at 0.115–0.07.
- **Intonation by ear:** the performer measures the period between slips on the bowed string and corrects the finger (time constant 0.15 s), as a player does. Stopped notes end within ±5 cents; open strings can't be corrected and stay 6–14 cents flat at mf–ff.

**Performer** (`performer.rs`)
- Dynamics drives bow speed (0.04–0.5 m/s, log) and β (0.115–0.07); force is the band position times the band at the current speed.
- Attacks: the force leads the speed (the full band force from the start) and the speed rises as a quarter sine (finite initial acceleration). Quiet attacks only start cleanly high in the band and slowly, so the pressure tilts up and the attack lengthens at low dynamics. Every tested note (C2–E5, pp–ff) reaches Helmholtz motion within 35–100 ms.
- Détaché alternates bow direction. Overlapping notes play legato: a finger glide on the same string (12–140 ms by velocity), or a string crossing that moves the force over 30 ms. The string with the lowest position is chosen, with 5 semitones of hysteresis in legato and an optional bias toward lower strings.
- Staccato grips, bites (extra pressure for 30 ms), accelerates, and stops on the string: 25–48 dB down 100 ms after the stop. Spiccato is a sin²-shaped touch (30–65 ms) at full speed and leaves the string ringing.
- Vibrato: finger modulation with delayed onset, slow random drift of rate and depth, none on open strings.
- Pitch and bow position update at 3 kHz; the bow every sample. A counting-allocator test checks that playing never allocates.

### Phase 3 notes: CLAP plugin (September 2026)

Build the bundle with `cargo xtask bundle strings-plugin --release` (writes `target/bundled/Strings.clap`), or play without a DAW: `cargo run --release -p strings-plugin --features standalone -- --backend alsa` (or `jack`). On Linux the build needs the X11/XCB and JACK headers listed in CLAUDE.md.

**Plugin** (`crates/strings-plugin/src/lib.rs`)
- nih-plug (pinned to a September 2026 commit; the project is active), CLAP export only, egui editor. Mono instrument, the same signal on every output channel (stereo or mono layouts).
- The performer is built in `initialize` (about 30 ms), only again if the sample rate changes. `process` handles MIDI sample-accurately, never allocates (nih-plug's `assert_process_allocs` aborts on an allocation in debug builds) and returns `KeepAlive` so strings ring on. nih-plug sets flush-to-zero. A non-finite output resets the instrument and counts the reset (shown in the status row).
- **Controls:** CC11 dynamics, CC1 vibrato (changed from CC1/CC11/CC21 after comparing with SWAM; see "Phase 3 notes: playing like SWAM"), CC123 releases all notes gracefully (`Performer::release_all`), CC120 silences at once. Dynamics, vibrato, pressure, bow lift, polyphony, fingering and volume are also host parameters. **Whichever changed last wins:** a parameter only acts when its value changes, so a CC keeps its value until the parameter moves.
- **Keyswitches:** the white keys from the first C below the lowest note (cello: C1 off string, D1 on string, E1 down-bow, F1 up-bow; they chose sustain, staccato and spiccato until "Phase 3 notes: playing like SWAM").
- **CPU:** 2.1% of real time on one core at 48 kHz for the engine (performer plus telemetry, 256-sample blocks, notes changing every 0.5 s; `cargo test --release -p strings-plugin cpu_cost -- --ignored --nocapture`). The standalone's load meter showed about 4% while idle-playing a note; probably CPU frequency scaling under light real-time load. To confirm in a DAW.

**Editor** (`crates/strings-plugin/src/editor/`, layout from ROADMAP.md)
- Status row: sample rate, block size, DSP load (smoothed and peak, yellow above the 3% budget), output peak, NaN resets; toggles for the computer keyboard and a debug view.
- Instrument and ensemble selection (only cello and solo are enabled), bow-lift buttons that follow keyswitches, polyphony and fingering.
- The instrument view is a placeholder drawing. The strings are drawn in parts (nut to finger, finger to bridge, afterlength) from the performer's state: the finger where the vibrating length starts, the bow at β moving along its length with the bow velocity, and Helmholtz motion (a corner on a parabolic envelope) on the vibrating part, slowed down for display with an amplitude from the string's bridge force. It is stylized, not the simulated string shape.
- Performer status, below the instrument (as SWAM shows it): the last three articulations, newest on top (`Articulation`: soft, détaché or accented attack, staccato attack or martelé, legato, shift, portamento, string crossing, double stop, bow lift, spiccato, bow stop), and beside them the down-bow or up-bow mark of the current or last stroke. The performer keeps the last three in a fixed array; the plugin publishes player 0's with the telemetry.
- CC64, the sustain pedal, plays détaché (see 4.3); the status shows whether it is down.
- Readout: note, string, position, pitch, β, bow speed and force, and the motion from slips per period (Helmholtz at one per period, timed from slip onsets; exact for periodic motion). The debug view adds the calibrated force band and the bow's position in it, and per-string pitch, contact and level.
- Bottom: a piano from C1 to C6 (mouse: lower on a key is louder, dragging plays legato; keyswitches colored, out-of-range keys grey), the computer keyboard in the tracker layout (Q = C, 2 = C♯ … P = E, by physical key position; Z/X transpose by octave) with a velocity setting, and faders for the parameters. A marker on a fader shows the performer's value when a CC has moved it away from the parameter.
- Editor and audio thread share only atomics (telemetry, published once per block) and a lock-free queue (notes from the editor).

### Phase 3 notes: tuning and first listening (September 2026)

**First listening** (the plugin, played from the computer keyboard): pitch and releases fine. Attacks lack the thick bite of a cello; held notes are static; low notes lack weight ("more violin than cello"); spiccato sounds plucked. There is no room yet (Phase 6, with the sections), so the instrument is heard dry.

**Measuring bowed spectra.** Measure harmonics at the *measured* pitch. Open strings play 6–14 cents flat at mf–ff (STATUS.md), so a probe at n × the nominal f0 misses the upper partials, which then look like a 40–50 dB cliff above h5–h8. At the measured pitch the bridge force is a clean sawtooth: h2–h8 within about 5 dB of 1/n on all four strings, open and 7 semitones up, at dynamics 0.1–0.9 and band positions 0.25–0.65, with and without bow hair. The string isn't what makes the cello sound light; the body is.

**Body.** With the six listed modes and dense modes from 300 Hz, the body passed 6–9% of the power of D3 and A3 below 300 Hz, and the low harmonics sat 7–15 dB under the 400 Hz–1 kHz ones. The dense modes now start at 150 Hz (59 modes, the same density) under a broad rise at 250 Hz (0.6 octave, gain 1.5): 52% and 32% below 300 Hz, centroids 839 → 571 Hz (D3) and 991 → 626 Hz (A3), C2 13% → 62%. The output gain drops from 0.1 to 0.065 to keep the level (the new body is about 4 dB louder). The rise is an estimate, to be judged by ear.

**Performer** (`PerformerTuning`, which now holds the timing constants)
- **Bite:** a détaché stroke starts `attack_bite` = 0.2 × velocity higher in the band, fading over 80 ms (raised cosine). Every tested note still reaches Helmholtz motion within 150 ms.
- **Wander:** pressure ±0.06 of the band, bow speed ±8% and bow position ±3% drift on raised-cosine segments (mean 0.8, 1.1 and 1.4 s), never above the β mapping's top. ±5% β moved E4 at pp 8.7 cents: the bowed pitch of high stopped notes depends on β (stiffness). With a steady bow stopped notes still end within ±5 cents; with the wander within ±10.
- **Finger damping:** a stopped string loses `finger_loss` = 0.015 Np per reflection at the finger (t60 about 1.6 s at D4, 2.8 s at E3), so a spiccato or a released note rings shorter than an open string. The finger stays down after the note, so a stopped note rings on as an open one does (it used to ease off at the key-up, which cut short stopped notes but not open ones). When the next note goes to another string the finger eases off: another 0.08 Np, fading in over 50 ms (t60 about 0.25 s at D4). Open strings ring on. 0.02 Np moved E4's bowed pitch at pp out of tolerance. Both values are estimates: the fingertip's loss is really frequency dependent (Phase 4). `BowedString::set_termination_loss` scales the nut-side reflection; the Schelleng maps are unchanged with it at zero.

**Tuning window** (`crates/strings-plugin/src/tuning.rs`, `editor/tuning_window.rs`; "Tuning" in the status row)
- About 90 numbers in groups: the performer's timings and gestures (`PerformerSettings` and `PerformerTuning`), friction and bow hair, the body (listed modes, dense modes, hills, output gain) with its response plotted over the preset's and the partials of the playing note, and the strings' damping curve, bending stiffness and torsion (shared by the four strings as in `presets::cello::string`).
- Changed values are marked; "Copy changes" copies them as `field = value  # was …`, where the field is a path in `tuning::Tuning` (`live.performer.*`: the performer defaults; `live.body.*`, `live.friction.*`, `live.hair.*`, `strings.*`: `presets::cello`).
- Live changes reach the audio thread as one `LiveTuning` copy through a lock-free queue. `Body::set` retunes the resonators in place, keeping their state, with room for 12 listed and 200 dense modes.
- The strings' filters take about 30 ms to fit. The editor fits them on a thread of its own when the mouse is released, and `BowedString::apply_design` swaps them in without allocating; the string rings on. The old filters go back to the editor to be freed. Torsion can be tuned down to 2 × f0 (the delay memory allows it).
- Changes aren't saved with the plugin. A new engine (a new sample rate) gets them again from the editor.

### Phase 3 notes: playing like SWAM (September 2026)

Changes to the controls after looking at how Audio Modeling's SWAM strings are played (ROADMAP.md "Plugin playing").

**Controls.** CC11 (the expression pedal) plays the dynamics and CC1 (the mod wheel) the vibrato. The expression control, a gain after the body, is gone: dynamics already sets loudness through the bow, and Volume remains. CC21 no longer does anything. Velocity already shaped the attack (the bow's acceleration, 35–120 ms, and the bite, 0.2 × velocity in the band) and the staccato and spiccato strokes; that is unchanged.

**Pressure** (`PerformerSettings::pressure_range`). The control runs from flautando (0) through normal (0.5) to scratch (1), piecewise linear in band position: 0.0 → 0.65 → 1.3. Dynamics tilt, wander and bite add on top, clamped to that range. Measured with a steady bow, six notes C2–C5 at three dynamics:
- Below the lower edge (p < 0) notes break into multiple slips, and at pp several miss their pitch by up to a semitone, so flautando stops at the edge. There most notes are still Helmholtz, 3–8 dB quieter than normal.
- p = 1.0 is already 25–40 cents flat at mf–ff (the flattening effect); by 1.2 nearly every note is raucous. At 1.3–1.4 all are raucous, 5–10 dB louder than normal, with the pitch unsteady (tens of cents). That is scratch.
- The model can't play a real flautando on the cello: players also move toward the fingerboard, but β stays below 0.115 (the flat zone). The cello's flautando is only the lower force. The violin's moves toward the fingerboard too (`PerformerSettings::tasto`; see "Phase 5: the violin").

**Hand position and legato slides.** A legato line used to glide on every step on one string: 50 ms at velocity 90 for a whole tone, where a cellist drops a finger. String crossings were clean (the new string's finger is placed before the bow arrives). The performer now keeps a hand position, the lowest finger position it covers, shared by all strings, with `hand_span` = 4 semitones (first to fourth finger with an extension). A legato note within it changes in the finger-placement time (6 ms); only a shift slides, and only between two stopped notes (to or from an open string, the finger is just placed). The landing velocity now adds a portamento on top (see "One way of playing" below). Shifting up puts the new note under the first finger, shifting down under the last. Tested: a whole tone within the hand settles in under 12 ms, a shift at low velocity takes over 60 ms.

**Fingering modes** (`Fingering`, replacing `string_bias`). Each mode plays notes up to a number of semitones above a lower string's open pitch on that lower string: 0 near the nut (the lowest positions, open strings), 7.5 in mid position (just past a fifth, so every open string but C is played stopped on the string below), 12.5 near the bridge (up to an octave up the lower string). Ties go to the lower string. Legato still stays on its string within `legato_stick`, and a portamento (landing velocity below `portamento_velocity`, from a stopped note) wherever the string can play the note.

**Double stops** (`Polyphony::DoubleStops`).
- While a note sounds, a new one joins it on the adjacent string: the lower note on the lower string, stopped notes within `hand_span` of each other (an open string pairs with anything). The sounding note keeps its string if any pair allows it; otherwise the cheapest pair by the fingering's cost.
- With two sounding, the new note leads on from the nearer one (moving its finger, as a voice) and pairs with the farther one; if that doesn't fit, it pairs with the nearer one; if nothing fits, it plays legato from the nearer note alone.
- Releasing either note leaves the other on its string, unless the other key follows within `chord` (30 ms): then the stroke ends on both, as a chord released on a keyboard should (on the string, the bow stops on both). Notes pressed together join in the grip or the attack.
- Both strings get the same bow speed and position; each gets the force of its own band. Vibrato moves both fingers together; intonation by ear listens to the older note, and the other string uses the correction learned on it.
- The editor shows both notes and a contact mark on each string the bow touches. `strings-render play doublestops` renders an example.

**One way of playing** (`BowLift`, replacing the sustain, staccato and spiccato articulations; PLAN.md 4.3). A detached note is a new stroke whose velocity sets the attack; its end depends on the bow lift; overlapping notes play legato with the landing velocity setting the transition. The articulations follow from the gesture: staccato is short notes on the string, martelé the same pressed hard, spiccato-like short notes off the string, détaché long notes off it, portamento soft legato.
- The old staccato played a fixed stroke (100–160 ms) whatever the key length, and spiccato a scripted sin² touch; both are gone. Now a stroke lasts as long as the key, at least `min_stroke` (40 ms, later 60 ms: at 40 ms a tap at mid velocity ended before the bow reached its speed, and fast taps sounded clipped), so a tap still plays.
- On the string the stroke starts from a grip (15 ms of force before the bow moves) and accelerates over `grip_attack` = 100 ms at velocity 0 to 8 ms at 1; the old staccato always took 8–20 ms. At the end the bow stops (40 ms) and rests on the string until the next note, however long (it used to lift after 0.4 s). Switching to off string lifts a resting bow.
- **Retuned by ear in the plugin** (September 2026, on the violin, for both instruments): the attack off the string at velocity 0 takes 0.15 s (was 0.12); the bow change 10 ms (12); portamento below velocity 0.8 (0.6); a chord's keys let go within 40 ms (30). On the string: a 10 ms grip (15), then the bow accelerates over 300 ms at velocity 0 to 65 ms at 1 (100 to 8 ms), with a bite of 0.38 (0.25) lasting 70 ms (30); the stop takes 75 ms (40). The slower attacks put a mid-velocity mf note's Helmholtz motion at up to about 155 ms, so the range checks allow 180 ms (150); the seed sweeps fail 4 (cello) and 5 (violin) of 1152 checks. The slower stop leaves a short D4 on the string −19 dB 0.3 s after the key is up (−25 dB or less before); lower notes are damped as before. The violin's `pp_attack` stays 0: at 1, its G4 and F♯4 at pp stick in multiple slips. A slower ear (`ear_time` 0.7) left high stopped notes 10–30 cents off for over a second, and stays 0.15.
- **Retuned again** (September 2026, both instruments): on the string the bow now accelerates over 100 ms at velocity 0 to 10 ms at 1 (was 300 to 65 ms), and the ear is faster (`ear_time` 0.1, was 0.15). The seed sweeps fail 8 (cello) and 4 (violin) of 1152 checks.
- Off the string, a stroke shorter than the release (150 ms) lifts over its own length without slowing, so a short note is thrown off and rings; a longer one slows to 40% as it lifts, as before.
- Checked with 120 ms notes at velocities 0.2–0.9 on D3, D4 and C5: in both bow lifts every stroke reaches Helmholtz motion in 35–86 ms, a little later when soft. C2 and G2 are too low to judge in 120 ms. On the string, G2, D3 and D4 are more than 25 dB down 0.3 s after the key; off it they ring within 15 dB of the stroke's peak 0.1 s after.
- Legato transitions: a whole tone within the hand settles in under 12 ms pressed hard and 120–300 ms at velocity 0.1; a shift pressed hard takes 12–40 ms. This replaces the velocity glide of the hand-position change above, which made every soft or medium legato slide.
- The bow direction alternates with every detached note, as SWAM's drawing shows; it already did, and the editor's bow moves with it.
- **Bow changes in fast détaché** (found with the `ostinato` score: sixteenths at 150 bpm in the lowest octave). Off the string, a note 20 ms after an 80 ms stroke comes before the bow has lifted (the contact only falls to 0.85), so the stroke starts from a bow moving the other way. It used to reverse over the whole attack (35–120 ms), putting the bow change 20–40 ms into the note with the full force on a nearly still bow: an accented open C2 after a D2 on the same string stuck to the bow and was dragged (the bridge force almost all slow, the output down to −50 dB for two notes). Now a stroke that starts while the bow moves first changes bow, slowing to zero over `bow_change` = 12 ms (the force easing with the speed), then accelerates as the velocity says. Every ostinato note now speaks within 7 dB, on the string and off it. The phrase's détaché notes after the legato line change too; the legato and staccato scores are unchanged (their bows are at rest when the next stroke starts).

### Phase 4: comparison with recorded notes (September 2026)

The first A/B against real cello recordings. `strings-render compare` takes the University of Iowa MIS cello arco set (2012; `scripts/fetch-reference-data.sh iowa-cello`): anechoic, mono 44.1 kHz, one file per string and dynamic (pp, mf, ff), each a chromatic run of long single notes up two octaves of that string, without vibrato, the bow lifted at the end. Each recorded note is played again by the performer at the same sample rate, on the same string (`Performer::set_string`, a "sul G"), at dynamics 0.1 / 0.5 / 0.9, velocity 64, off the string, as long as the recorded stroke. Both go through the same measurements after a 40 Hz high-pass: the attack (from 30 dB below the sustain level to 3 dB below), the ring after the stroke (to −30 dB), pitch and vibrato (YIN), the harmonic spectrum in five bands (partials 1, 2–3, 4–7, 8–15, 16+, relative to all harmonic power) and the harmonic-to-noise ratio (HNR), both over 6-period Hann frames at the frame's own f0. It also writes `out/compare/<dyn>-sul<string>.wav`: each note recorded, then the model, at the same sustain level, for listening.

About the recordings: the gaps between notes are digital silence; below 20 Hz there is rumble only 7 dB under a quiet note's fundamental (hence the high-pass, without which the HNR reads 7 dB); the player plays a median 15 cents sharp, and some notes nearly a quarter tone off, so notes are numbered from the run's first note (the file name) rather than rounded from the pitch; one extra segment and a few ambiguous notes are skipped or labeled a semitone off (about 290 of 297 notes pair up).

Results, medians per dynamic over all strings (recording / model):

| | attack | ring | HNR | band 1 | band 4–7 | band 8–15 | centroid (partial no.) |
|---|---|---|---|---|---|---|---|
| pp | 370 / 25 ms | 0.39 / 0.17 s | 27 / 40 dB | −1 / −5 | −19 / −6 | −32 / −19 | 1.4 / 2.6 |
| mf | 320 / 10 ms | 0.34 / 0.17 s | 32 / 43 dB | −2 / −4 | −12 / −7 | −23 / −20 | 1.7 / 2.5 |
| ff | 30 / 10 ms | 0.51 / 0.19 s | 32 / 41 dB | −3 / −6 | −10 / −7 | −18 / −18 | 2.1 / 2.6 |

- **The model's spectrum doesn't follow the dynamics.** The recording brightens from pp to ff by 7–11 dB in partials 4–7 and 9–14 dB in 8–15, on every string. The model's stays the same or even darkens (C string centroid 5.7 at pp, 3.9 at ff). At pp the model is far too bright on the low strings (partials 4–7: −4 dB against −20 on the C string); at mf–ff on the A string it is too dark above partial 8 (−33 against −18 to −23). The level spread from pp to ff matches (13–15 dB recorded, 15–19 dB model). Candidates: what sets the slip corner's sharpness (force, the pressure tilt at low dynamics, bow width, friction), and the body's level at the low partials.
- **The fundamental is weak on the low strings:** C string −9 to −16 dB of the harmonic power against −1 to −8 recorded (the recording's microphone distance and room may add to it; the body's low end is estimated, STATUS.md) **Fixed** from A2 to D3: see "The body's low end".
- **The model is about 10 dB cleaner** (HNR 37–45 dB against 26–34 dB) at every dynamic: there is no bow noise.
- **High positions on a lower string fail.** Notes 18–24 semitones up a string (sul C F#3–B3, sul G D4–G4, sul D B4–C#5, mostly at mf–ff) play 17–57 cents sharp with an HNR of 9–16 dB: not clean Helmholtz motion. (First written up as 7–18 semitones, a miscount.) **Fixed:** see "High positions: the bow's distance from the bridge".
- **Attacks and rings.** The recorded pp and mf notes swell in over 300–450 ms (the player's slow start at the heel), ff notes in 30 ms; the model reaches its level in 10–40 ms at every dynamic, because the bite front-loads the stroke. After the stroke, the recording rings 0.25–0.55 s to −30 dB and the model 0.13–0.3 s: the model's finger eased off when the note ended (finger damping, "Phase 3 notes: tuning and first listening"), while this player keeps it down. The finger now stays down until the next note goes to another string, as this player's does (not measured again).

Not yet done: recordings with vibrato and connected playing. The Guettler attack comparison (the mdw data) is in "Attacks against measured data".

### High positions: the bow's distance from the bridge (September 2026)

The high positions of the comparison (above) fail on the lower strings from about 16 semitones up on the C string, 18 on the G, and not at all on the D and A up to two octaves. A probe (each string 0–24 semitones up by `set_string`, pp / mf / ff, pitch and slips per period in the sustain) shows notes playing Helmholtz-like but 20–60 cents sharp with 0.7–0.9 slips per period, or raucous. The bow sticks through some periods and slips twice in others. The performer's ear holds the slip period in tune; the output isn't.

- **Not the oversampling** (the same at 1×). Leaving out stiffness or torsion only moves where it starts.
- **It is the bow hair with a bow too close to the bridge.** With a rigid bow every note is clean up to 24 semitones. With the hair, a larger β (0.2–0.12 instead of 0.115–0.07) makes every note clean, and a smaller one (0.09–0.05) makes it much worse. β is a fraction of the vibrating length, and the performer kept it fixed, so two octaves up the bow sat a quarter as far from the bridge as on the open string: about 1 cm on the C string at ff. Failures begin when the bow comes within about 18–22 mm of the bridge on the C and G strings; the lighter D and A strings play clean closer than that.
- **Tried first: retuning the hair.** With each candidate's own recalibrated force band (`calibrate --hair-stiffness --hair-damping`), stopped notes 1–24 semitones up, all strings and dynamics (288 notes), fail: k = 1000 N/m, R = 3 kg/s (current) 25; R = 6: 18; R = 10–20: 17–36; k = 3000–10000: 13–60. A soft spring with more damping (k = 100, R = 10) fails only 1, and keeps the measured Helmholtz area (315 / 315 / 173 against 335 / 325 / 186), but it breaks five performer tests: pp attacks take 0.26 s, a bow stopped on the string no longer silences it, and scratch no longer goes raucous. The hair stays as it was. A rigid bow can't be calibrated at all (536 of 5376 cells settle promptly).
- **The fix: the bow keeps its distance from the bridge.** `PerformerSettings::bow_distance`: the bow comes no closer to the bridge than `bow_distance · Z` (m, with the string impedance Z in kg/s), so β = max(mapped β, distance / vibrating length). The default 0.024 m per kg/s gives 3.5 cm on the C string, 2.5 on the G, 1.5 on the D and 1.4 on the A. Open strings and low positions are unchanged (it takes over from about 8 semitones up at ff, 17 at pp on the C string). Scaling with Z matches where the strings fail, and players do bow heavier strings farther out.
  - A single distance for all strings works only in a narrow window. At 2.5–3 cm the sul C F#3 attack at mf takes 100–150 ms and fails the seed sweep on 8 of 24 seeds; at 3.5–4 cm high notes on the D and A strings go multi-slip at pp (β up to 0.16). Scaled with Z, the probe is clean from 3.5 to 4.5 cm on the C string (only the open-position A pp at 6 semitones, 21 cents sharp, fails, as before).
  - A β power law in the stopped position (β·(f/f_open)^γ, capped) can't satisfy both ends either: 4–20 failures.
- **Results.** The range tests now include six high positions (sul C F#3 and B3, sul G D4 and G4, sul D B4 and D5); without the minimum distance they are raucous or 29–69 cents sharp. The seed sweep has 3 failed checks over 1152 notes (sul G D4 at mf settles slowly on 2 of 24 seeds). `compare`: the high positions (18–24 semitones up the C, G and D strings) are within 3 cents, except sul D C#5 (−5 cents at mf and ff) and ff sul D B4 at +17.5 cents, clean (HNR 36 dB) and unchanged from before; the medians don't change. A/B renders: `play sul` (a phrase up the G string, then the C string, using the new `string` score event) and `play phrase --fingering bridge`, in `out/ab-bow-distance/`; by ear the change sounds better.
- β above 0.115 on high stopped notes is inside the flat zone (STATUS.md item 8); the performer's ear corrects stopped notes, so it doesn't show.

### The body's low end (September 2026)

The low strings' weak fundamental (item 43) comes from the body, not the string. `strings-render compare --bridge` measures the strings' summed bridge force instead of the output. There the fundamental is 2–4 dB below the harmonic power on the C and G strings at every dynamic, as in the recording (1–5 dB), and partials 2–3 match too.

- **The recording's fundamental isn't strong everywhere.** Per note, C2–F2 (below A0) are 7–25 dB down in the recording as well, and the old body matched them. The gap was from G2 to D3 (98–147 Hz): −9 to −31 dB in the model against −1 to −3. The six listed modes are narrow (ζ 1.2–2.5%), and the dense modes start at 150 Hz, so the body passed 104–131 Hz 17–33 dB below its level at 200–400 Hz.
- **The fix: two more listed modes,** 118 Hz (ζ 0.03, gain −1.3) and 144 Hz (ζ 0.04, gain +1.7), standing for modes a real body has between A0 and the dense ones. They are fitted, not from the literature. The target per note is the body's fundamental against its partials 2–3, taken as the recording's value minus the bridge force's, pooled over the C and G strings and the three dynamics, smoothed over 3 semitones. The fit is a random search over two modes (95–210 Hz, ζ 0.02–0.08, either sign). It is kept only if A3 and Eb4 lose less than 2 dB and no note gets more than 4 dB weaker than before. Mean error: 8.0 dB before, 4.0 after. The body above 400 Hz is unchanged within 0.3 dB.
- **Rejected:**
  - A broad resonance at 80 Hz (ζ = 1), which fills the whole low end: it makes C2–F2 far too strong (−3 against the recording's −7 to −25), and the open A (220 Hz) 7 dB weaker.
  - Starting the dense modes at about 100 Hz: 5–8 dB mean error depending on the seed, and it reseeds every dense mode up to 6 kHz.
  - A single mode or broad fill: either leaves A#2–C3 10 dB weak or makes F#2 and F#3 go dead (−24 and −14).
- **Results** (`compare`, band 1 median, pp / mf / ff): C string −14 / −9 / −16 → −11 / −8 / −9, and per note A2–D3 went from −6 to −31 dB to −1 to −8 at mf–ff (recorded −1 to −5), and from −12 to −38 to −3 to −15 at pp (A2 is the weakest). G string −6 / −5 / −6 → −5 / −2.5 / −2.4 (recorded −1.4 / −2.2 / −2.5). D and A strings unchanged. The C and G strings are 1–3 dB louder; band 4–7 drops 1–3 dB relative to the harmonic power, toward the recording.
- **Still off:** G2–Ab2, just above A0 (−7 to −17 against −1 to −2), and Ab3–C4 around 220 Hz on the C string (−10 to −13 against 0 to −2), which is where the estimated 200 Hz (−) and 209 Hz (+) modes cancel. Both depend on the listed modes' estimated levels. Fitting one recorded cello's individual modes more closely would overfit.
- A/B renders in `out/ab-body-low-end/`: phrase, legato, staccato, ostinato and sul, before and after, plus `-after-matched` copies at the old RMS (the new body is 1–2.5 dB louder). By ear the change sounds good (September 2026).

### Phase 5: the violin (September 2026)

`presets::violin::INSTRUMENT`, playable in the renderer (`play violin-scale --instrument violin`; scores `violin-*`) and the plugin (the Instrument box).

- **Strings:** the Phase 1 violin strings unchanged (typical synthetic-core tensions, one-pole loss, no stiffness or torsion), so the physics tests and the violin Schelleng map (`schelleng --instrument violin`, rigid bow) stay the Phase 1 reference. Violin strings are much less stiff than cello strings (3.6); a plain steel E string's B (≈ 6e-5) moves its 5th partial about 1 cent.
- **Bow hair: the cello's** (k = 1000 N/m, R = 3 kg/s), not refitted: the same kind of bow. With a rigid bow the G string's Helmholtz band was narrow and patchy (bracketed in 21 of 48 calibration columns; band positions 0.5–0.8 gave prompt Helmholtz motion in 54–83% of cells), its attacks took 150–250 ms and the open G at pp stayed in triple slip. With the hair: 47 of 48 columns, 92–100%, and 1346 Helmholtz cells of 5376 against 1015. Hair damping 1 kg/s and stiffness 3000 N/m did about as well.
- **Body:** the signature modes from Woodhouse's measurements of one violin (euphonics.org 5.3): A0 272 Hz, CBR 407, B1− 462, B1+ 551; the bridge hill at 2.3 kHz; 60 dense modes from 500 Hz to 10 kHz, falling above 5 kHz. Damping, signs and levels are estimates.
- **Force band:** `calibrate --instrument violin --sample-rate 96000`. Band positions 0.5–0.8 give prompt Helmholtz motion in 88–100% of checked cells (the E string at 0.8 is lowest). The upper edges follow β^−0.74 to β^−0.88, the lower β^−1.5 to β^−2.1.
- **Performer:** the cello's timings, with its own bow positions (below). Per-instrument values live in `InstrumentSpec` (`beta`, `tasto`, `flautando`, `pp_attack`, `output_gain`, `seat`) and reach the performer through `PerformerSettings::for_instrument`; `PerformerSettings::default()` is the cello's. The output gain (0.5 against 0.065) matches the cello's median level on the example scales: the violin strings' impedance is about a third of the cello's.
- **Checks:** the range tests play G3–A6 and six high positions (12 and 19 semitones up the G, D and A strings) at pp, mf and ff: all Helmholtz within 150 ms and in tune (stopped notes within 5 cents with a steady bow; 10 with the wander), at normal pressure and at flautando. The seed sweep (`violin_notes_stay_helmholtz_across_wander_seeds`) has 5 failed checks over 1152 notes, all quiet attacks (with the cello's mapping it had 3, all the open D at pp). E7 still plays Helmholtz and in tune at mf.
- **Bow positions (the violin's own):** dynamics move β from 0.16 (pp) to 0.065 (ff), and the flautando end of the pressure control moves the bow on to β 0.2 (sul tasto, `tasto`) at band position 0.3 (`flautando`) instead of the band's lower edge. Players bow a violin from about 0.04 to 0.2. The violin strings have no flat zone (that needs torsion), so the limits are the performer's attacks and high notes:
  - **Far from the bridge, the performer's slow quiet attacks hold multiple slips.** The plain string with a steady bow is Helmholtz across the band at β 0.2, even at 0.05 m/s, but the performer's pp attack (0.17 s to full speed, with the full force from the start) locks it into 2–3 slips per period, for good. At fixed β and pp, failed checks over 8 seeds: 1 of 128 at 0.13, 8–17 at 0.15–0.16, 38 at 0.19 (with the cello's `pp_attack` 1.6). Without slowing quiet attacks (`pp_attack` 0) β 0.16 fails 1 of 128. The cello's low strings need the slow attack (Guettler's diagram: double slips at low force), the violin's don't, so `pp_attack` is per instrument: 0 for the violin.
  - **Close to the bridge, high notes on the E string miss their pitch** (tens of cents either way) below β ≈ 0.065 at mf–ff: A6 and E6, where the bow's minimum distance (0.024 m per kg/s, 4 mm on the E string) doesn't hold it back. At 0.06, 4 of 128 checks fail at ff; at 0.045, 34.
  - **Flautando at the band's lower edge fails far from the bridge:** at β 0.16–0.25, 107–147 of 384 checks (slow attacks and multiple slips), against 8 with the cello's mapping. At band position 0.3 and β 0.2 it passes: 3 failed checks over 1152 notes (24 seeds) at flautando, 2 halfway (pressure 0.25), 5 at normal pressure; clean with a steady bow. Flautando and tasto values were searched only coarsely (0.3–0.5, 0.2–0.22); the results are noisy from one pair to the next.
  - **Heard as:** the example scores are 0.5–1.3 dB quieter than with the cello's mapping. Flautando (`play violin-tasto`) is 2.9 dB below normal (0.3 dB before); on a held A5 its 5th partial falls by 20 dB (the bow at 1/5 of the string) and partials 8–9 by 5–12 dB, while partials 3–4 rise 1–3 dB. A/B renders in `out/ab-violin-beta/` (with level-matched copies), listened to (September 2026): they sound pretty good.
- **Cost:** 1.2% of real time in the renderer, 2.1% for the plugin's engine on the stage (cello 3.6%), 13.4% for 12 players (cello 23.6%). Building a violin takes 0.24 ms: its one-pole loss needs no fitting.
- **Plugin:** the instrument is a non-automatable parameter inside the one plugin (the CLAP ID stays). `initialize` builds the chosen instrument; changing it while active builds the new engine on nih-plug's background thread, `process` swaps it in (cutting off what was sounding) and hands the old one back to be freed there. The tuning window follows the engine's instrument (telemetry), and its changes carry the instrument they were made for, so a late change for the old instrument never reaches the new engine. The keyswitches move with the instrument (violin: C3 and D3), and so does the on-screen keyboard (violin C3–C8).
- **The cello is unchanged:** its renders are byte-identical to before, solo and as a section.

### Phase 5: viola and double bass (September 2026)

`presets::viola::INSTRUMENT` and `presets::bass::INSTRUMENT`, in the renderer (`play viola-scale --instrument viola`, `play bass-scale --instrument bass`; scores `viola-*`, `bass-*`) and the plugin (the Instrument box: violin, viola, cello, double bass).

- **Viola, built like the violin:** flexible strings with the one-pole loss (decay times continued down from the violin's, guesses), the cello's bow hair, the violin's bow positions (β 0.16–0.065, sul tasto to 0.2 at band position 0.3) and fast quiet attacks (`pp_attack` 0). Strings: Larsen Original medium (A 8.0 kg, D, G and C 4.9 kg) at 370 mm, Larsen's length for its chart. Body: A0 230 Hz, body modes around 350 and 440 Hz taken as B1− and B1+ (Jóhannsson, a maker's measurements), CBR at 310 Hz (the violin's ratio to B1−), the bridge hill at 2 kHz (an estimate, below the violin's 2.3 kHz), dense modes from 400 Hz. The scores are the cello's an octave up (the same tuning), plus `viola-tasto`.
- **Viola checks:** `calibrate --instrument viola --sample-rate 96000` finds the band in 44–48 of 48 columns; band positions 0.5–0.8 give prompt Helmholtz motion in 92–100% of cells. The range tests (the cello's notes an octave up, with high positions 18–24 semitones up the C, G and D strings) pass at pp, mf and ff with a steady bow and with the wander, on the first try; the seed sweep fails 10 of 1152 checks, all at pp: B3 on 5 of 24 seeds (multiple slips or raucous, 26–93 cents flat), G4, and the open G3 and D4 (multiple slips), as the violin's quiet attacks do.
- **Double bass, built like the cello:** the cello's damping curve, torsion estimates and bow hair, with bending stiffness EI = 5e-3 N·m² (B = 1.4–1.5e-4, about 3.5 × the cello G's; not measured: a 0.85 mm steel core, less for rope cores). Strings: Spirocore Orchestra medium on a 3/4 bass at 1.06 m (G 67.2, D 68.3, A 70.5, E 72.8 lb). Body: A0 66 Hz, the coupled T1/A1 at 115 Hz and A2 at 155 Hz (Brown 2004, four basses at the mdw), a broad rise at 90 Hz and a bridge hill at 700 Hz (both estimates, the cello's scaled), dense modes from 80 Hz to 4 kHz falling above 1.5 kHz.
- **The bass E string's Helmholtz band is narrow.** `calibrate --instrument bass` finds it in only 22 of 48 columns (A 28, D 37, G 38), and only one or two force rows deep in the Schelleng maps, multiple slips below at every β; band positions 0.5–0.8 give prompt Helmholtz motion in 79–88% of E-string cells (92–100% on the D and G). It is not one of the estimates: without torsion, without stiffness, with neither, with the one-pole loss (lowpass 0.5 and 0.7) or with other bow hair (k 300–10000 N/m, R 3–20 kg/s) it is the same or worse. Hair damping 9 kg/s did best (742 Helmholtz cells of 5376 against 608, the E string's band in 34 columns), but gave no clear gain in the playing tests, so the bass keeps the cello's bow. At β = 0.1 the band's width (upper/lower edge 1.8) is close to the cello C string's (2.0).
- **Bass bow speed and distance from the bridge** (`InstrumentSpec::speed` and `bow_distance`, new, per instrument like `beta`): with the cello's values (0.5 m/s at ff, 0.024 m per kg/s) high positions at ff went raucous or missed their pitch by 30–80 cents; 68 of 192 checks failed over 4 seeds. A closer bow was much worse (0.008–0.016: 108–181 failures), as on the cello (PLAN.md "High positions"). The bass plays at up to 0.3 m/s at ff and keeps 0.032 m per kg/s from the bridge (11.9 cm on the E string, 4.6 cm on the G): farther (0.035–0.04), high notes at pp hold multiple slips. Over 24 seeds, the high positions at ff failed 36 of 120 checks at 0.35 m/s, 3 at 0.3; a lower pressure (0.55) did as well, but the speed is per instrument already. The other instruments keep 0.5 m/s and 0.024 and render byte-identical.
- **Bass checks:** the range tests play E1–G4 (C1–G4 since the C extension) and high positions as bassists play them: up to an octave on the E string, 17 semitones on the A, thumb position on the D (19 and 24 up). All pass with a steady bow and with the wander; the seed sweep fails 4 of 1152 checks, ff thumb-position notes 33–72 cents flat. 18–23 semitones up the E string the bow's distance puts it at β 0.4, and those notes hold multiple slips at pp: they are left out (bassists play them on the D and G strings).
- **The bass's open E plays 20–25 cents flat at mf–ff,** more than the other instruments' open strings (6–14): the flattening effect on the heaviest string. The range tests allow open bass strings 30 cents (20 for the others).
- **Plugin:** keyswitches sit at the highest C that keeps all four (C, D, E, F) below the instrument's lowest note: violin C3, viola C2, cello C1, bass C0 (at C1 the bass's E and F keys would be its open E1 and F1). The Instrument box lists the instruments high to low. The telemetry used to store the instrument's enum value and read it as a position in that list; it now stores the position.
- **Levels:** output gains 0.4 (viola) and 0.06 (bass) match the cello's median level on the example scales within 1 dB (cello −27.4, viola −27.6, bass −27.2 dBFS). Across the scales each body is as uneven as the cello's (9–11 dB between the loudest and quietest note). The viola's sul C Bb4 at dynamics 0.85 is the loudest solo moment (−1.6 dBFS peak): loud already in the bridge force, and near B1+.
- **Cost:** plugin engine on the stage: viola 2.1% of real time, bass 2.6% (cello 3.6%); 12 players 13.6% and 17.4%. Building a bass takes 92 ms (the cello 60 ms), a viola 0.25 ms.
- **Renders** in `out/viola/` and `out/bass/`: every score, and `phrase` and `legato` by 8 players on the stage. Listened to (September 2026): they sound good.

### The bass's C extension (September 2026)

The bass plays down to C1, as orchestral basses with a C extension do. The E string runs on past the nut a major third farther (`presets::bass::EXTENDED_LENGTH`, 1.06 × 2^(4/12) = 1.335 m) at the same tension, so its impedance is the E string's and it sounds C1 open; it is named "C". From E1 up it vibrates at the lengths it had without the extension, so its stiffness, loss and torsion are unchanged there. E1 is a stopped note (the finger's damping and the performer's intonation), where a real extension stops it with a metal gate.

- **Two force bands on one string** (`InstrumentSpec::extension`, `Extension`): `calibrate --instrument bass` finds the open C's band in 15 of 48 columns (lower 2.579·β^-0.66, upper 5.610·β^-0.57), about 15% higher than the E string's at the β of stopped notes. With the open C's band alone, G1, B1 and E2 at ff went raucous or missed their pitch by 60–105 cents; with the E string's band alone, C1 and D1 settled late (0.19–0.31 s). The performer now blends the two, log-linearly in the force, over the four semitones between open C and the gates, and uses the E string's band from E1 up (`InstrumentSpec::force_limits_at`; the editor's force band too). `calibrate` maps the string stopped at the gates as a fifth string (`Extension::gated`) and reproduces the old E-string fit exactly. Other instruments have no extension and render byte-identical.
- **Checks:** the range tests add the open C1 and D1 (E1 is now stopped, 4 semitones up the C string). All pass with a steady bow and with the wander; the seed sweep fails 3 of 1296 checks, none on the new notes (ff thumb-position notes on the D string, as before). The rendered open C1 at mf plays 15.6 cents flat.
- **Renders:** `bass-scale` now descends through D1 to a long open C1. `out/bass-c/before/` and `out/bass-c/after/` hold every `bass-*` score before and after: `phrase`, `staccato` and `sul` are identical; the others use the lowest string (the open E is now a stopped E1, and the undamped lowest string rings sympathetically at C instead of E).
- **Cost:** unchanged on the stage (2.65% of real time, 12 players 17.3%); building a bass takes 99 ms (92 before).

### The bow's width (September 2026)

The bow used to touch the string at one point. A real hair ribbon is about 10–15 mm wide, and it can slip at one edge while the other still sticks (Pitteroff & Woodhouse 1998). Now `BowHair::width` spreads the contact over up to four points (`string::MAX_CONTACTS`), a whole number of samples apart and centered on the bow position, with short lines between them (whole-sample reads, so no interpolation filters the waves as they cross). Each point takes an equal share of the bow force and of the hair's stiffness and damping, and has its own stick/slip state. The number of points is the most whose span is within half a sample or 10% of the width, so it depends on the string's wave speed and the sample rate: on the cello at 96 kHz, 12 mm is 4 points on the C, G and D strings and 3 on the A. Torsional waves get lines between the points too, at least one sample each (physically about a fifth of the transverse gap). The frame's state and bow-point velocity are the middle point's; its friction force is the sum. Width 0 is one point and renders byte-identical.

- **Measured string** (`measured --hair-stiffness 1000 --hair-damping 3 --bow-width W`; Helmholtz points at 0.05 / 0.1 / 0.2 m/s, measured 701 / 700 / 392, then at β < 0.05 with agreeing cells in brackets, measured 529):

| Width | Helmholtz points | at β < 0.05 |
|---|---|---|
| 0 (point) | 335 / 325 / 186 | 287 (274) |
| 4 mm | 394 / 398 / 234 | 293 (282) |
| 8 mm | 512 / 502 / 301 | 296 (283) |
| **12 mm** | **602 / 579 / 334** | **328 (314)** |
| 16 mm | 684 / 643 / 371 | 352 (338) |

  At 12 mm the area is 83–86% of the measured one, within Phase 1b's ±30%. The H/not-H agreement goes from 81 / 76 / 85% to 86 / 85 / 90%, and the lower force limit at β = 0.1 from 0.38 / 0.88 / 1.66 N to 0.24 / 0.44 / 1.03 N (measured 0.21 / 0.31 / 0.76). The width matters, not the number of points: at 12 mm, at most 2 points give 625 / 597 / 351 and at most 8 give 565 / 548 / 329. The hair's k = 1000 N/m, R = 3 kg/s are still the best of k 300–3000, R 2–5. The width also helps the flexible string with the one-pole loss (324 / 271 / 151 → 608 / 467 / 260) and the string without torsion. The robot in the measurement bowed with the hair flat, so the whole ribbon touched; 12 mm is an estimate of a cello bow's ribbon, not measured.
- **Presets:** the cello's hair (`presets::cello::HAIR`) is 12 mm wide, the bass's 14 mm. Their force bands are recalibrated: the cello's has 1028 Helmholtz cells (977 before) and band positions 0.5–0.8 give prompt Helmholtz motion in 96–100% of cases (91–99%); the bass's 731 (712). The prompt lower limit hardly moves (the G string at β = 0.1, v_b = 0.1 m/s: 1.17–3.12 N, before 1.2–2.9 N): the width lowers the steady-state limit more than the prompt one. The seed sweeps fail 0 of 1152 cello and 0 of 1296 bass checks (3 and 3 before).
- **Not the violin and viola.** With a 10 mm ribbon the violin's band shrinks from 1346 to 688 cells (D string found in 16 of 48 columns, from 48), 813 with up to 6 points, and even 3 mm leaves 959; the viola's 1114 to 704 at 11 mm. The upper limit falls. On the cello, 22 mm (the violin's ratio of width to string length) also costs a little (912 cells) the same way, which fits the differential slipping Pitteroff & Woodhouse describe (a slip has to spread across the ribbon); their numbers are not checked here. Violinists tilt the bow, so fewer hairs touch, but with no measured violin string to fit to, both keep a point contact.
- **Against the recorded notes** (`compare`, heard output, medians; before → after, recorded in brackets): at pp the low strings are darker, C string partials 8–15 −8 → −14 dB (−32), centroid 4.7 → 3.0 (1.3), G string −15 → −21 (−30), centroid 3.4 → 2.5 (1.5); partials 4–7 barely move (C −4 → −5, G −5 → −7; recorded −20). The C string's fundamental at pp comes up from −10 to −5 dB (−1). On the bridge force the C string at pp goes from −10 to −19 dB in partials 8–15 while ff is unchanged, so the width darkens quiet notes more than loud ones. But the spectrum still doesn't brighten with the dynamics (C centroid 3.0 / 2.5 / 3.2 at pp / mf / ff; recorded 1.3 / 1.8 / 2.2), the A string at mf–ff is darker still above partial 8 (−33 → −36 dB; recorded −23), and the model is 2–7 dB cleaner (HNR 42–50 dB; recorded 26–34).
- **Pitch:** stopped notes high on the A string play about 10 cents flatter (E4 needs a 22-cent correction, 12 before), which the ear corrects. Its correction carries from note to note on a string, so after a legato shift from E4 to C4 it eases by about 15 cents over 300 ms (the legato test now runs with the ear off). The open D at a given pressure plays a few cents less flat than before. In a section, players pressing harder and closer to the bridge than the default (the humanization's pressure and β) play open strings far flatter: 29 and 46 cents at pressure 0.77 and 0.89 with β × 0.86 and 0.73, against 16–20 before, because the new band's upper edge sits higher near the bridge, where the model plays flat (STATUS.md item 7).
- **Cost:** the solo cello takes 4.2% of real time on the stage (3.7% before), 12 cello players 27.7% (24.5%; the budget is 25%). The contact loop costs more than the solves: at most 2 points still take 25.5%.
- **Renders:** `out/ab-bow-width/` has `scale`, `legato`, `phrase`, `staccato`, `bass-scale`, `bass-phrase` and an 8-player `phrase`, before and after, plus long notes at pp, mf and ff (`dynamics.score`, `bass-dynamics.score`). Listened to (September 2026): the change sounds better.

### Soft, dark pp (September 2026)

STATUS.md item 40: recorded pp notes are far darker than the model's (partials 4–7 about −20 dB of the harmonic power against −6 to −8 on the C and G strings). A probe bowed steady notes with fixed speed, position and force and measured the bridge force's harmonic bands.

- **Force helps, down to a floor.** On the C string at pp (0.05 m/s, β 0.11), band position 0.8 (the pp tilt) gives −6.1 dB in partials 4–7 and −11.7 in 8–15; 0.2 gives −8.4 and −20.0. Once a note has settled it stays Helmholtz far below the prompt band (band position −1, 0.39 N), but no darker: −9.6 and −18.8, about an ideal sawtooth rounded by the string's losses. At ff the band position hardly matters (−9.8 to −9.4 in 4–7).
- **Nothing else in the bow darkens it past that floor:** bow width 0–24 mm, hair stiffness 300–3000 N/m, hair damping 1–10 kg/s (10 gives multiple slips) and friction v0 0.03–0.3 m/s.
- **What is missing:** Cremer's theory (via Woodhouse & Galluzzo 2004) has the Helmholtz corner "significantly rounded when the normal force … is small". The model's friction curve with its stick/slip jump keeps sharpening the corner even at low force, so the spectrum doesn't follow the force. The candidate is Woodhouse's thermal (plastic) friction model, which also bears on the lower force limit (STATUS.md items 4–5) and matched measured attacks better than the friction curve (Galluzzo). Not done yet.
- **As built:** the pressure tilt keeps quiet attacks high in the band, but held notes don't need it. Once a stroke is going (its attack over, no bow change pending), the band position eases down by `quiet_ease` × (1 − dynamics) over `ease_time` (0.3 s), fully only at the pressure control's middle (flautando and scratch are unchanged). A new stroke, a bow change or a crossing to another string starts it high again. Per instrument (`InstrumentSpec::quiet_ease`): 0.5 on the cello and bass, 0 on the violin and viola, whose low pp notes fell into multiple slips or lost their pitch (the viola's seed sweep failed 14 checks).
- **Against the recorded notes** (`compare`, heard output, medians, before → after, recorded in brackets): C string pp partials 4–7 −5.5 → −6.6 dB (−19.9), 8–15 −15.1 → −18.5 (−32.0), the fundamental −5.3 → −4.8 (−1.1); G string pp 4–7 −7.5 → −8.2 (−20.3), 8–15 −21.5 → −23.2 (−31.0). mf barely moves. The seed sweeps fail 0 of 1152 cello and 0 of 1296 bass checks.
- **Listening files:** `out/ab-quiet-ease/before/` and `after/` (`compare`'s pairs for pp and mf, each recording then the model). Listened to (September 2026): the change sounds good.

### Bow noise (September 2026)

STATUS.md item 42: the model's harmonic-to-noise ratio was 10–20 dB above the recorded notes' (43–49 dB against 27–32), and at mf the recording was heard as brighter, with more obvious bow noise and a more obvious onset.

- **Model: friction noise while slipping.** The friction force the junction returns is multiplied by `1 + level · n` while the string slips, `n` being uniform white noise through a one-pole lowpass (`cutoff`), scaled to unit RMS so its density below the cutoff doesn't depend on the sample rate. The string velocity follows the load line, so the noise enters the string like any force, and the string and body shape it. Sticking hair moves with the string and adds none: the noise comes in pulses at each slip (Chafe's "pulsed noise"), and more of it at attacks, where the string slips longer and irregularly. Each contact point of a bow with width draws its own noise at √n × the level, so the sum fluctuates by `level` whatever the width. Zero level renders byte-identical to before.
- **Measuring the noise's spectrum.** `compare` now also prints the noise halfway between partials in four bands (<1 k, 1–2 k, 2–4 k, 4–8 kHz; dB relative to the harmonic power). In a Hann frame exactly 6 periods long those frequencies fall on the window's zeros for every partial, so the partials don't leak in. The recorded notes have the same shape at every dynamic: about −38 / −49 / −47 / −54 dB.
- **Fit.** The level sets the HNR almost alone: at 0.065 (cutoff 2 kHz), medians at pp / mf / ff are 31.1 / 32.4 / 31.2 dB (recorded 26.7 / 31.7 / 31.8; before 49 / 49 / 43), within 3 dB per string at mf–ff; pp stays 2–6 dB cleaner than the recording. Attack times, the harmonic bands, levels and rings don't change; the pitch's measured wobble rises by about a cent (2–3 against 1–2 recorded). The same values on all four instruments.
- **The noise's spectrum is 6–8 dB too strong at 1–2 kHz** (−41 to −43 against −47 to −50) and 2–5 dB weak at 4–8 kHz, with the other two bands within 3 dB. The cutoff hardly changes that: gated to the short slips, the noise is broadband whatever its input bandwidth (250 Hz to 20 kHz tried). At the bridge (`compare --bridge`) the model's noise is −38 / −43 / −47 / −48, so part of the excess is the string and part the estimated 1.3 kHz bridge hill (STATUS.md item 16); the recording's dip at 1–2 kHz, below its 2–4 kHz, looks like a real body's response.
- **Rejected:**
  - Noise in the stick phase as well (the hair's grip fluctuating): the right low-heavy shape below 2 kHz, but the pp attacks take 45 ms instead of 15.
  - Lowpassing the gated noise (a smoother force): the pitch wobbles by 10–25 cents.
- **Seed sweeps** (failed checks without → with the noise; the noise now follows `PerformerSettings::seed`): cello 0 → 1 of 1152, violin 4 → 6, viola 10 → 8, bass 0 → 9 of 1296, all within the 1% limit. Nearly all are quiet attacks just over 0.15 s (the bass's lowest notes at pp: 0.19–0.49 s). On the violin, which fixed-seed checks fail is a draw: at levels 0.03–0.065 one or two of its quiet attacks fail on the default seed, and the sweep has 6 at both. The steady range tests (`steady()` in `tests/performer.rs`) turn the noise off with the wander; the seed sweeps check both.
- **The force band is calibrated without it:** `calibrate` bows bare strings, so its map stays deterministic.
- **Cost:** none measurable (cello 4.2% on the stage, 12 players 28.0%).
- **Listening files:** `out/ab-bow-noise/` (`-before` without the noise, `-after` with it: cello phrase, legato, staccato and sul, an 8-player cello section, and the violin, viola and bass phrases; levels within 0.4 dB), and `compare`'s pairs in `out/compare/`. By ear it sounds good (September 2026). The tuning window has the level and bandwidth under Bow.

### Thermal friction (September 2026)

STATUS.md item 40 and the "Soft, dark pp" notes point at the friction model: the curve keeps sharpening the Helmholtz corner at low force. Woodhouse's thermal model is built as an option (`bow::ThermalFriction`, off by default; renders without it are byte-identical).

- **Model** (Woodhouse 2003, from Smith & Woodhouse 2000; docs/Literature.md): friction depends only on the contact temperature, μ(T) digitized from Woodhouse's Fig. 3 (1.2 cold, 0.35 above about 50 K). The heat `F·Δv` goes into a 1 µm rosin layer (its heat capacity), out with the sheared rosin, and by conduction into the string (the whole history) and the bow (renewed at the bow speed). Conduction is `A·√(Kρc)·√(s + c)` per surface, `c = 0` for the string and `3v_b/(8a)` for the bow (their steady-sliding flux, Eq. 20), as 8 one-pole modes per surface, within a few percent of their discrete Green's function below a few kHz. The contact area grows with the force (Amontons). The balance is solved implicitly each sample with the last sample's heat (conduction is about 30× stiffer than the layer's heat capacity at 96 kHz). At a given temperature the junction is Coulomb's: one solution, no Friedlander construction; the hysteresis comes from the temperature. Each contact point of a wide bow has its own temperature. Table I's values (a rosin-coated perspex rod on a cello string).
- **Checks:** at 0.05 m/s the contact runs at 17–36 K (Woodhouse: 17–31). Against the measured cello string (`measured --thermal`, 12 mm hair), with nothing fitted: Helmholtz area 647 / 727 / 485 cells at 0.05 / 0.1 / 0.2 m/s (measured 701 / 700 / 392; the curve 602 / 579 / 334), lower limit at β = 0.1 0.17 / 0.31 / 0.67 N (measured 0.21 / 0.31 / 0.76; the curve 0.24 / 0.44 / 1.03), H/not-H agreement 84 / 90 / 92% (86 / 85 / 90%). At the lowest forces the string stops oscillating instead of slipping several times (Woodhouse's Fig. 9 shows the same).
- **It fails at fast bows.** The contact heats with the bow speed: at 0.4 m/s it sits at 45–83 K, where μ(T) is flat, and the string slips two or three times a period. Woodhouse simulated only 0.05 m/s. A fixed stretch of μ(T)'s temperature axis only moves the working window (about 4× of bow speed each; scale 3 plays 0.1–0.4 m/s and fails the measured string at 0.05). **Fix, not from the literature:** the axis stretches as `(v_b / 0.05 m/s)^0.5` (`speed_exponent`), rosin softening at higher temperatures the faster it is sheared, which Woodhouse's conclusion allows for. The measured string's agreement stays 83 / 90 / 90% (lower limits 0.17 / 0.28 / 0.65 N), and the cello's force band comes back: 958 Helmholtz cells (the curve 1028), band positions 0.4–0.9 prompt Helmholtz in 95–99%. All four instruments use it (`presets::cello::THERMAL`).
- **Force bands:** calibrated per instrument with `calibrate --thermal` (`InstrumentSpec::thermal_band`); `force_limits_at` switches to them while thermal friction is on. Violin 1319 cells (curve 1346), viola 1195 (1114), bass 929 of 6720, with its own band at the extension's gates. Band positions 0.4–0.8 give prompt Helmholtz motion in 88–98% (violin), 97–100% (viola) and 86–88% (bass; the E string 67–71%).
- **Against the recorded notes** (`compare --thermal`, heard output, medians, the curve → thermal, recorded in brackets): partials 4–7 at pp −9 → −13 dB (−19), mf −9 → −12 (−12), ff −9 → −10 (−10); 8–15 at pp −24 → −28 (−32), ff −21 → −25 (−18). The spectrum now brightens a little with the dynamics (4–7: 3 dB, recorded 9). But ff is darker than recorded, the HNR falls to 24–27 dB (recorded 27–32; the bow noise was fitted with the curve), the pitch wobbles by 4–5 cents (recorded 1–2), and the pitch falls with force as Woodhouse predicts: open strings up to 20 cents flat, and the D string's notes a median 25 cents flat at pp–mf, some A-string notes 34–49.
- **By ear:** no audible difference in `compare`'s pairs (September 2026). So it stays off by default. It may still matter for attacks, where Galluzzo found it closer to measured transients.
- **Cost:** 30–50% more per string (the solo cello phrase 2.7 → 4.0% of real time, the bass 2.9 → 4.3%).
- **Where to find it:** `--thermal [SCALE]` on `bow`, `schelleng`, `measured`, `calibrate`, `play`, `compare` and `guettler`; the plugin's tuning window has a "Thermal friction" checkbox (Bow). A/B phrases in `out/ab-thermal/curve/` and `thermal/`.

### Attacks against measured data (September 2026)

STATUS.md item 44: the Iowa notes show how one player started them, not what the string can do. The mdw attack data can show that. `strings-render guettler` (data: `scripts/fetch-reference-data.sh guettler-waveforms`) compares the model with the measured Guettler diagrams of Lampis, Mayer & Chatziioannou (JASA Express Lett. 2024; docs/Literature.md). A robot bows four cello G2 strings (A Prelude, B Helicore, C Dominant, D Kaplan; sample 2 of each, as in the paper) on the monochord at β = 5.5/70. Each stroke starts from rest, with a constant bow force (0.1–4.2 N) and a constant acceleration (0.15–3.2 m/s²). There are six sessions per string, about 5900 strokes each. Every measured stroke is played again on the model at its own force and acceleration: the string from the paper's Table 1 (tension, pitch, EI), the reference string's damping, torsion scaled to its impedance, the cello's bow (hair with its width, bow noise, 2× oversampling). Both bridge forces go through the same classifier.

- **The classifier is the authors' own**, ported from their MATLAB code (Zenodo 10946413; only the `.m` files were read, by range requests, out of the 22 GB archive). The first slip is the first peak of the smoothed slope of the bridge force. Galluzzo and Woodhouse's histogram method then takes over: scaled by the bow speed and with the Helmholtz ramp taken out, Helmholtz motion is a staircase that falls 2Z/β per period. Each step is a peak in a histogram, and a smaller or larger step is multiple slips or raucous motion. The transient runs from the first slip to the last non-Helmholtz step before at least three Helmholtz steps, and 30 means it never settles. On one Dominant session it gives the same transient as the authors' results file (`matx.mat`, from the Acta Acustica data) for 92% of strokes, and the same success or failure for 99.6%.
- **Findings with the old friction (μs 0.8).** The model's successful region (transient under 20 periods) is Guettler's wedge, as measured. But its right edge, the fastest acceleration a force can start cleanly, needs about twice the measured force: F = 2.0·a + 0.5 (B) and 1.6·a + 0.4 (C), against 0.97·a + 1.0 and 1.06·a + 0.38 measured (A 1.63·a + 0.92, D 0.91·a + 1.28). So 21–29% of the strokes succeed against 40–43% measured on B and C. Per stroke, success and failure agree in 78–88%. Guettler's limits scale with a friction term of μs and μd. At μs 0.8 and μd 0.3 his equation B gives F ≈ 2.6·a on these strings, which is the model's slope. The data's `stat` files have the measured static coefficient just before the first slip at 1.2–1.4.
- **Neither the hair, the width, the noise nor the oversampling moves it much.** Right edges on B / C: rigid bow 2.5 / 1.7 (and far fewer successes: 6 / 9%), width 0 1.8 / 1.35, no bow noise 1.9 / 1.4, 1× 2.2 / 1.65, v0 0.05 1.8 / 1.4, v0 0.2 2.2 / 1.7. **Thermal friction is worse:** its region is a narrow band, F ≈ 0.7·a, that fails at slow accelerations, and only 14–18% of strokes succeed (agreement 49%).
- **μs sets it.** Right edges over the four strings: 1.28–1.65 at μs 0.9, 1.12–1.48 at 1.0, 1.11–1.34 at 1.1. Higher up the left edge moves right (slow, heavy starts go raucous), and at 1.3 per-stroke agreement falls to 45–53%. Agreement over all four strings is 84 / 83 / 83 / 84% at μs 0.9, 73–83% at 1.0 and 88 / 79 / 78 / 87% at 0.8. The share of successful strokes at 0.9 is 21 / 28 / 37 / 30% (measured 22 / 40 / 43 / 30%).
- **The steady state agrees too** (`measured`, with the cello's hair and width). At μs 0.9: H/not-H agreement 88 / 88 / 91% (0.8: 86 / 85 / 90%), Helmholtz area 604 / 617 / 368 cells (0.8: 602 / 579 / 334; measured 701 / 700 / 392), and lower limit at β = 0.1 0.19 / 0.37 / 0.88 N (0.8: 0.24 / 0.44 / 1.03; measured 0.21 / 0.31 / 0.76). μs 1.0 fits the steady state a little better still (0.17 / 0.32 / 0.76 N, 87 / 88 / 93%), but the attacks worse.
- **As built: the cello's μs is 0.9** (`presets::cello::INSTRUMENT`; `FrictionParams::default()`, and so the Phase 1 violin strings and the physics tests, stay at 0.8). The recalibrated band (`calibrate --sample-rate 96000`) has 1018 Helmholtz cells (1028 before), and band positions 0.2–0.9 give prompt Helmholtz motion in 93–99%. For the G string at β = 0.1 and 0.1 m/s it now runs 0.95–2.46 N (1.17–3.12 before).
- **Quiet attacks can be faster.** The cello's `pp_attack` was 1.6, set against Guettler's double slips at low force. Now it is 0.6. Seed sweep failures (of 1152): 5 at 1.6 with the new friction, 2 at 1.0, **1 at 0.6** (G2 at ff, 0.22 s; 1 before the change), 6 at 0.3 and 13 at 0 (pp C2 and G2 in multiple slips again). In `compare` the model's median attack falls from 20 / 15 / 15 ms to 10 / 10 / 15 ms at pp / mf / ff. The HNR rises by 1–2 dB (32.3 / 34.0 / 32.9 dB; recorded 26.7 / 31.7 / 31.8), and the harmonic bands don't change.
- **The bass follows the cello (μs 0.9).** It is built like the cello and plays with its hair, so it takes the same friction; there is no attack data for it. Its recalibrated band lies 15–20% lower (the E string's lower limit at β = 0.1: 1.742·β^−0.80 → 1.393·β^−0.82 of Z·v), band positions 0.5–0.8 give prompt Helmholtz motion in 79–100% (71–100% before), and the band at the extension's gates was found in 15 columns (20 before). Unlike the cello's, its quiet attacks must be *slower*: at `pp_attack` 1.6 the seed sweep fails 15 of 1296 checks (9 before the change), all but three pp attacks on C1–E1 settling in 0.2–0.6 s; 1.0 and 0.6 fail more, 2.2 none, 3.0 two ff attacks. So `pp_attack` is 2.2. In `compare` (medians, recording / model) the HNR rises 0.5–1.2 dB (pp 33.2 → 33.7, mf 33.3 → 34.3, ff 30.9 → 32.1; recorded 28.3 / 30.0 / 31.4), the attacks 5 ms at pp and mf, and the harmonic bands, ring and level don't move. Listening files: `out/ab-bass-friction/` (`play bass-*`, before and after, levels within 0.4 dB) and `compare`'s pairs in `out/compare-bass/` (after) and `out/ab-bass-friction/compare-before/`. By ear the change sounds good (September 2026).
- **Not done:** the violin and viola keep μs 0.8. There is no measured attack data for them, and changing theirs means recalibrating each band and its seed sweep.
- **Listening files:** `out/ab-attack-friction/before/` and `after/` (cello phrase, legato, staccato, ostinato and scale; levels within 0.5 dB); `compare`'s pairs in `out/compare/` (after) and `out/compare-before/`. By ear the change sounds good (September 2026); the attack's timings and bite stay as they were.

### The other instruments' recorded notes (September 2026)

STATUS.md item 61. `compare` now measures the model against any of the four instruments' Iowa sets (`--instrument violin | viola | bass`; data from `scripts/fetch-reference-data.sh iowa-violin | iowa-viola | iowa-bass`). All four sets are recorded the same way: one player, one mic, chromatic runs of long single notes per string, dynamic (pp, mf, ff) and range.

- **Per set:** the file prefix, Iowa's "sul" letters and which preset string each is played on, and the high-pass both signals get (15 Hz for the bass, whose open C1 is 32.7 Hz; 40 Hz for the others). The violin's and cello's files name the dynamic before the string, the viola's and bass's after it. The bass's set has both the extended C string (sulC, C1–Eb1) and the E string stopped at the gates (sulE, E1 up); both are the model's lowest string, which plays E1 as a stopped note.
- **The model now uses each instrument's own gesture values** (`PerformerSettings::for_instrument`, in `compare` as in `play`). Before this, a viola or bass note was compared after being bowed with the cello's β, speed, bow distance and force band.
- **The viola set's 16/44.1 kHz files hold 96 kHz audio**: the take named `A4B4` reads 202 Hz at 44.1 kHz but 440.4 Hz read at 96 kHz (docs/Violin Reference Recordings.md §1; the site's own individual stereo AIFFs are the same). `compare --file-rate 96000` reads the samples at 96 kHz and runs the model and the measurements at that rate.
- **Numbers** (medians per dynamic, recording / model; `out/cmp-{violin,viola,bass}.csv`):

| | pairs pp/mf/ff | attack ms | partials 4–7 dB | 8–15 dB | HNR dB |
|---|---|---|---|---|---|
| violin | 101 / 97 / 89 | 195/30, 115/20, 110/10 | −12/−10, −11/−13, −10/−12 | −27/−27, −25/−32, −23/−26 | 22.3/28.2, 26.0/29.6, 29.6/30.5 |
| viola (96 kHz) | 100 / 96 / 100 | 545/30, 510/10, 390/10 | −15/−7, −11/−9, −11/−10 | −33/−22, −25/−25, −24/−21 | 26.7/28.4, 32.5/29.7, 30.6/32.4 |
| bass | 90 / 101 / 93 | 300/40, 220/25, 115/25 | −16/−8, −16/−9, −15/−11 | −24/−27, −24/−28, −23/−25 | 28.3/33.2, 30.0/33.3, 31.4/30.9 |

- **What it says:** ring-off, pitch and HNR are within a few dB on all three, and the model's partials 4–7 are still brighter than the recordings' at pp (item 40's pattern; the viola is the opposite above partial 8 at pp, −33 against −22). The recorded attacks are 4–30× the model's at every dynamic: these players swell into the note (as the cello's do), where the model's time is the physical onset. The recordings' own levels differ by instrument (pp medians: violin −48.5, viola −46.2, bass −40.6, cello −35.9 dBFS), so level means nothing across sets — only within one.
- **Listening pairs:** `out/compare-violin/`, `out/compare-viola/` (at 96 kHz), `out/compare-bass/` — recording then model per note, level-matched, as for the cello. Not listened to yet.

### The body fitted to recordings (September 2026)

The bodies' low modes had frequencies from the literature, but their levels, the hills and the dense modes' envelope were estimates. `compare` had measured the spectrum only in five bands of partials, which can't show a body's shape. Two changes:

- **The spectral envelope** (`compare`, every run; `strings-render/src/envelope.rs`). For each pair of notes, every partial at least 10 dB clear of the noise halfway to its neighbours, in both signals, gives a point: frequency, recording − model in dB (each relative to its note's harmonic power). All notes' points are fitted as `d = H(f) + c_note` in sixth-octave bins, by alternating medians. The notes span the instrument's range, so partial number and frequency decouple: `H` is what a filter after the model would have to add. `compare` prints `H` per dynamic in third octaves and writes every bin to `<out>/envelope.csv`. It also prints the **envelope distance**: per note, the RMS over sixth octaves of recording − model after the note's own offset, with the median over notes shown as it is and after taking `H` off (the second is what a perfect smooth correction would leave). With `--bridge`, `H` is the body, radiation and microphone together.
- **`strings-render fit-body`** fits a body to `envelope.csv`. The body's response, the median dB over a third of an octave, is fitted to `H` smoothed the same way, by random search. The finer structure is where this body's modes and the recorded instrument's differ, which a smooth change can't mend. What moves: the dense modes' level, rolloff and up to four hills (at least 0.3 octaves wide); with `--listed`, the listed modes' levels (within 4× either way; frequencies, damping and signs stay); with `--dense-from`, where the dense modes start, at their density; with `--seeds N`, the dense modes' seed, the best of N. If the dense modes stop below the top of the fit (10 kHz), the bank is extended at its own spacing. The generator draws each mode's values in order, so the modes already there don't move. `--keep-below` holds the body as it is below a frequency. The overall level is free, so each output gain was corrected afterwards by the level `compare` measured.

**Found:** on the cello the body made the spectrum worse. On the bridge force (`--bridge`) the envelope distance was 6.3 dB, with the body 7.7. The body was 8–13 dB too strong around 0.8–1.3 kHz (the estimated 1.3 kHz bridge hill, three times the base level) and 4–8 dB weak above 5 kHz. The other instruments showed the same kind of error, and larger:
- The violin's and viola's dense modes started at 500 and 400 Hz, above A0. Between the few narrow listed modes, the body passed their lowest notes' fundamentals 15–30 dB weaker than the recordings (violin 300–400 Hz, viola 200–300 Hz).
- The bass's 700 Hz hill was 12–17 dB too strong. With the dense modes ending at 4 kHz and rolling off from 1.5 kHz, its top was 10–20 dB too weak (partials 16 up: −47 dB against −37). Brown's "radiation falls steeply above 1 kHz" doesn't hold for this recording.

**As built** (`presets::*::BODY`):

| | fit | response misfit (dB) | envelope distance, all dynamics | output gain |
|---|---|---|---|---|
| cello | hills, level, rolloff; `--keep-below 250`; bank extended 6 → 9.9 kHz | 4.4 → 1.5 | 7.7 → 6.5 | 0.065 → 0.085 |
| violin | `--listed --dense-from 300 --seeds 12` | 5.9 → 2.0 | 7.4 → 6.5 | 0.5 → 1.02 |
| viola (96 kHz) | `--listed --dense-from 200 --seeds 12` | 5.9 → 1.3 | 8.0 → 6.3 | 0.4 → 0.19 |
| bass | `--listed --seeds 12`; bank extended 4 → 9.5 kHz | 8.1 → 1.6 | 10.3 → 6.7 | 0.06 → 0.017 |

- **The cello's low end is unchanged:** the fit kept the body below 250 Hz within 1 dB ("The body's low end"). The cello was fitted with an earlier version of `fit-body`, which held the level at the kept bins instead of leaving it free, and without `--listed`.
- **The listed modes' levels hit the 4× limit** in several places: violin B1− (1.0 → 0.25), viola A0 (0.8 → 0.2) and B1+ (−1.2 → −4.3), and bass A0 and T1 (to a quarter) and A2 (−0.6 → −2.4). The dense modes now carry most of the low end on those instruments. Only the modes' frequencies were ever from data.
- **Side effects in `compare`:**
  - The cello's noise between partials moved toward the recording: −49 against −50 dB recorded at 1–2 kHz (was −45), −57 against −55 at 4–8 kHz (was −61). This is part of STATUS item 42.
  - The violin's HNR fell to 22.2 / 25.6 / 26.8 dB at pp / mf / ff (recorded 22.3 / 26.0 / 29.6; was 28–31).
  - The bass's partials 16 up are at −40 / −39 / −38 (recorded −37 / −37 / −34), and its fundamental band at −1 (recorded −1; was −3).
  - The viola is now darker than its recording at mf–ff in partials 4–7 (−16 against −11).
- **What is left:** 5.6–5.9 dB of the 6.3–6.7 dB distance stays even after taking the remaining `H` off. That is note-to-note detail: this body's modes against the recorded instrument's, and the string's spectrum per note (STATUS item 40). Single bins stand out: the cello at 320 Hz (+7 dB in the recording, +16 at pp–mf), the violin at 400 Hz (+8), the viola at 200 Hz (+8), and the bass at 63 Hz (+12) and 320 Hz (+7).
- **Not yet heard.** The changes are large (the bass and viola moved about 11 and 6.5 dB in level before the gain correction, the bass's top by 10 dB). They are to be judged by playing the plugin.

## 8. Alternatives to explore later

| Technique | What it buys | Why not now |
|---|---|---|
| **FDTD stiff string** (Bilbao; Willemsen's real-time work) | Accurate stiffness and loss from physical constants, two polarizations, fingerboard collisions | Costs O(N) per sample. Changing pitch continuously needs dynamic grids, which are an active research area and prone to artifacts. Candidate for an "HQ solo" mode |
| **Modal synthesis** | Exact mode frequencies and damping per mode, easy to couple to other resonators, good for bodies and sympathetic resonance | Low strings need 150+ modes. Bow coupling requires summing all modes each sample |
| **Torsional waves** | A second waveguide per string, coupled at the bow. Known to affect the stick/slip trigger and attack quality (Woodhouse) | Implemented in Phase 1b (3.6); it doesn't help until the damping is realistic |
| **Thermal friction model** (Woodhouse) | Friction depends on the rosin's temperature, giving better attacks and hysteresis | Built as an option in Phase 4 ("Thermal friction"); off by default |
| **LuGre / elasto-plastic friction** | Micro-slip, dynamic hysteresis, smooth transitions | Multi-state, so there is no closed-form solve and it needs iteration; high CPU cost for sections |
| **3D bow-hair ribbon** | A realistic contact patch, hair compliance, the torsional interaction of the hair | Beyond real-time today. A lumped hair compliance is in since Phase 2 (`BowHair`), and a finite width as up to four contact points since Phase 4 ("The bow's width") |
| **Two transverse polarizations** | Realistic decay, beating, fingerboard contact | Doubles the cost. Mostly matters for pizzicato and decays |
| **Port-Hamiltonian formulation** | Passivity guaranteed by construction; robust under fast parameter changes | A heavier framework. Adopt only if stability problems appear with the DWG |
| **Lambert-W bow solver** | Closed-form solve for the *exponential* friction curve | The hyperbolic curve gives a quadratic, which is simpler. Revisit only if we switch curves |
| **Bridge-coupled strings** | Sympathetic resonance and energy exchange through a shared bridge admittance | Planned for Phase 4 in simplified form |
| **Measured body IRs** | Realistic timbre | Data sourcing and licensing |

---

## 9. Caveats and risks

- **The realism ceiling is in the details.** Bowed physical models tend to sound "synthy". The difference is made by attacks, the body, bow noise, legato transitions and humanized control, not by the core equations. Most of Phases 2–4 is about these details.
- **Controllability.** Keyboard, mod wheel and expression carry far less information than a real bow arm. The performer layer (4.2, 4.3) is the product, and it will need many rounds of tuning by ear.
- **Delay modulation artifacts.** Changing the nut-side delay quickly (legato, portamento) can click or zip. It may need a finger modeled as a lossy scattering junction with its impedance ramped in, instead of a plain change of delay length.
- **Tuning drift.** The phase delay of the loop filters changes with their parameters. Tuning compensation must be recomputed whenever the filters change.
- **Stick/slip hysteresis bugs** are easy to write and hard to hear in isolation. Test them explicitly (section 6).
- **Aliasing** from stick/slip corners at high bow force. The strings run at 2× (see "2× oversampling"); aliasing itself hasn't been measured.
- **High-frequency loss shapes playability.** With too little loss in the bridge filter, the sharp Helmholtz corner fragments the slip phase into extra slips, even well inside the Schelleng range. Loss pole 0.5 (at 48 kHz) gives a clean Helmholtz band without oversampling; brightness must come back through the body, not by thinning the loss.
- **A rigidly sticking bow traps energy.** With the bow stopped on the string, the nut-side segment only decays at the string's own rate (about −19 dB after 200 ms on the A string). Bow-hair compliance doesn't change that much (Phase 2 notes); a stop is clean when the force eases with the bow speed, so the Helmholtz motion follows the bow down.
- **Body data.** Sourcing measured body IRs (cello first) under a usable license is unsolved. The biquad bank tuned from published mode frequencies is the fallback.
- **CLAP-only host coverage.** Hosts without CLAP (for example Logic, which is AU only, and Pro Tools, which is AAX) can't load it. nih-plug can also export VST3, but its VST3 bindings are GPLv3, which is a licensing decision. CLAP-only keeps the licensing simple (nih-plug itself is ISC).
- **nih-plug maintenance.** Check the project's current activity before committing to it. `clack` is the fallback.
- **The research doc isn't fully reliable.** See below.

---

## Research notes / corrections

Errors and gaps in `docs/Real-Time Physical Modeling Techniques for Audio Synthesis of Bowed Strings.md`:

1. **Schelleng limits are swapped.** `F_max ∝ 1/β` and `F_min ∝ 1/β²` (and `F_min` depends on the bridge resistance `R`). Corrected formulas are in 4.2.
2. **Commuted synthesis doesn't apply to the bow excitation.** It requires LTI components. The body filter goes after the string.
3. **The Friedlander stick/slip ambiguity is missing.** It is essential; see 3.2.
4. **Lambert W is overstated.** The hyperbolic friction curve gives a closed-form quadratic.
5. **Notation clash:** `σ0` and `σ1` are used both for string damping and for LuGre bristle stiffness and damping.
6. Leftover citation markers (`[cite: 22]`) suggest AI-generated text. Some listed projects (PartialString, GayageumSynth) are unverified.

## Open questions

- Whether CC64 or CC68 toggles legato.
- Whether to rename the `strings-*` crates to match the product name, LibreStrings (decided September 2026; CLAP ID `io.github.petterthowsen.librestrings`).
