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
| Double bass | E1 A1 D2 G2 | ~1040–1060 mm | Stiffness and dispersion are clearly audible. Longer loops need more delay memory |

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
- `R` has to be derived from the bridge loss filter. Calibrate empirically in the renderer.
- **Measured in Phase 1:** the simulated upper edge follows `F_max` closely, but the lower edge sits about 5–10× above the formula's `F_min` (same 1/β² slope). The dynamics mapping should anchor on `F_max` or on a calibrated lower edge, not on the raw `F_min`.
- **Against a real string** (see "Measured comparison" under Phase 0–1 notes): the measured lower edge follows roughly 1/β (fitted exponents −0.9 to −1.3), not 1/β². This is in line with Schoonderwaldt et al. (2008) and Mansour et al. (2017). A calibrated lower edge should be fitted to measurements, not derived from the formula.
- **As built (Phase 2):** `instrument::ForceLimits` stores both edges per string as `F = c·Z·v_b·β^α`, fitted by `strings-render calibrate` to the model's own simulated maps (the performer needs forces where the *model* plays Helmholtz, and promptly: a cell counts only if it is Helmholtz within 0.15 s; see "Force band from prompt settling"). The measured lower edge is lower still; see "Phase 2 notes". The performer uses p = 0.65, tilted to 0.8 at pp and 0.5 at ff, with a bite at the start of each stroke and a slow wander (see "Phase 3 notes: tuning and first listening").

### 4.3 Articulations: one way of playing

As in SWAM, there are no articulation modes. How a note is played follows from whether it overlaps the last one, its velocity, its length and the bow lift (off string or on string, a parameter and keyswitches).

| Played as | Physics |
|---|---|
| **Detached note** (no overlap) | A new bow stroke; the direction alternates. Velocity sets the attack: off the string, the bow lands (12 ms) and accelerates over 35–120 ms with a bite of 0.2 × velocity in the band; on the string, the force grips for 15 ms before the bow moves, then it accelerates over 8–100 ms with a bite of 0.25 × velocity (hard presses are a martelé). The stroke lasts as long as the key, at least 40 ms |
| **End of a note, off string** | The bow lifts as it slows (150 ms), and the string rings on. A note shorter than that is thrown off as quickly as it was played, still moving: spiccato-like. Phase 4: a bouncing bow (a mass on a spring) |
| **End of a note, on string** | The bow **stops on the string** (40 ms) and stays there until the next note; the stopped bow damps the string, so short notes are staccato |
| **Legato** (overlapping notes) | No new bow stroke. The landing note's velocity sets the transition: pressed hard, a finger drops or lifts within the hand's position (6 ms) and a shift of the hand slides quickly (20 ms); below velocity 0.6 the finger slides, up to 250 ms at velocity 0 (portamento). Only between stopped notes; to or from an open string the finger is placed. A note on another string is a crossing: the bow force ramps from the old string to the new one |
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
| **1b. Measured string physics** ⚠️ | Bending stiffness, torsional waves (3.6) and measured frequency-dependent damping, checked against the measured cello G string with `strings-render measured` | Simulated Helmholtz region close to the measured one at all three bow speeds (area within about ±30%, Helmholtz present at small β); physics tests still pass. **Not met:** all three are implemented and verified. The damping gets closest, at about half the measured area; stiffness and torsion still shrink it. See "Phase 1b results" |
| **2. Solo cello (offline)** ⚠️ | Cello presets for C2 G2 D3 A3 (G from the measured string; the others from published string data), 4 strings, string selection, fingering, legato, vibrato, biquad body, performer layer with the 4 articulations and its force mapping calibrated on the measured limits (4.2), bow-hair damping so a bow stopped on the string silences it quickly | Scripted phrases (scales, legato lines, staccato runs) sound like a cello, not a synth. **Built and tested objectively; not yet judged by ear.** See "Phase 2 notes" |
| **3. CLAP plugin** ⚠️ | nih-plug wrapper, CLAP-only export, CC1/CC11/vibrato mapping, keyswitches, parameters, real-time safety | Playable in Bitwig and Reaper; no allocations in the audio thread; CPU cost measured. **Built:** plugin, editor and standalone app; engine at 4.8% of real time (strings at 2×); a tuning window for the model's numbers. **Plays in Bitwig; not yet tried in Reaper.** See "Phase 3 notes" |
| **4. Realism pass** | Thermal friction, finger damping at note changes (a first, frequency-independent version is in: "Phase 3 notes: tuning and first listening"), bow noise, oversampling decision, bouncing-bow spiccato, sympathetic string coupling | A/B against recordings; clear improvement on attacks and legato transitions (the mdw attack data gives a measured target for attacks). **Started:** `strings-render compare` against the Iowa cello notes; see "Phase 4: comparison with recorded notes" |
| **5. More instruments** | Violin, viola and double bass presets and bodies | Each instrument is convincing across its range |
| **6. Sections** ⚠️ | N-player engine, humanization, stage placement, SIMD across players | 12-player section within the CPU budget; sounds like a section, not a chorus effect. **Built** (docs/SECTIONS.md Phase A): up to 12 humanized players on a stage with early reflections, in the plugin; 12 players at 23% of real time without SIMD. Not yet heard in a host |
| **7. Extended techniques** | Tremolo, trills, pizzicato, double stops, harmonics, mutes, MPE | — |

**CPU budgets** (per instance, one core at 48 kHz):
- Solo: < 5% (raised from 3% for 2× oversampling; the engine measures 4.8%)
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
- **Keyswitches:** the white keys from the first C below the lowest note (cello: C1 off string, D1 on string; they chose sustain, staccato and spiccato until "Phase 3 notes: playing like SWAM").
- **CPU:** 2.1% of real time on one core at 48 kHz for the engine (performer plus telemetry, 256-sample blocks, notes changing every 0.5 s; `cargo test --release -p strings-plugin cpu_cost -- --ignored --nocapture`). The standalone's load meter showed about 4% while idle-playing a note; probably CPU frequency scaling under light real-time load. To confirm in a DAW.

**Editor** (`crates/strings-plugin/src/editor/`, layout from ROADMAP.md)
- Status row: sample rate, block size, DSP load (smoothed and peak, yellow above the 3% budget), output peak, NaN resets; toggles for the computer keyboard and a debug view.
- Instrument and ensemble selection (only cello and solo are enabled), bow-lift buttons that follow keyswitches, polyphony and fingering.
- The instrument view is a placeholder drawing. The strings are drawn in parts (nut to finger, finger to bridge, afterlength) from the performer's state: the finger where the vibrating length starts, the bow at β moving along its length with the bow velocity, and Helmholtz motion (a corner on a parabolic envelope) on the vibrating part, slowed down for display with an amplitude from the string's bridge force. It is stylized, not the simulated string shape.
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
- **Finger damping:** a stopped string loses `finger_loss` = 0.015 Np per reflection at the finger (t60 about 1.6 s at D4, 2.8 s at E3), so a spiccato or a released note no longer rings like an open string. When the note is over (key up and the bow off, or the bow gone to another string) the finger eases off: another 0.08 Np, fading in over 50 ms (t60 about 0.25 s at D4). Open strings ring on. 0.02 Np moved E4's bowed pitch at pp out of tolerance. Both values are estimates: the fingertip's loss is really frequency dependent (Phase 4). `BowedString::set_termination_loss` scales the nut-side reflection; the Schelleng maps are unchanged with it at zero.

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
- The model can't play a real flautando: players also move toward the fingerboard, but β stays below 0.115 (the flat zone). Flautando here is only the lower force.

**Hand position and legato slides.** A legato line used to glide on every step on one string: 50 ms at velocity 90 for a whole tone, where a cellist drops a finger. String crossings were clean (the new string's finger is placed before the bow arrives). The performer now keeps a hand position, the lowest finger position it covers, shared by all strings, with `hand_span` = 4 semitones (first to fourth finger with an extension). A legato note within it changes in the finger-placement time (6 ms); only a shift slides, and only between two stopped notes (to or from an open string, the finger is just placed). The landing velocity now adds a portamento on top (see "One way of playing" below). Shifting up puts the new note under the first finger, shifting down under the last. Tested: a whole tone within the hand settles in under 12 ms, a shift at low velocity takes over 60 ms.

**Fingering modes** (`Fingering`, replacing `string_bias`). Each mode plays notes up to a number of semitones above a lower string's open pitch on that lower string: 0 near the nut (the lowest positions, open strings), 7.5 in mid position (just past a fifth, so every open string but C is played stopped on the string below), 12.5 near the bridge (up to an octave up the lower string). Ties go to the lower string. Legato still stays on its string within `legato_stick`.

**Double stops** (`Polyphony::DoubleStops`).
- While a note sounds, a new one joins it on the adjacent string: the lower note on the lower string, stopped notes within `hand_span` of each other (an open string pairs with anything). The sounding note keeps its string if any pair allows it; otherwise the cheapest pair by the fingering's cost.
- With two sounding, the new note leads on from the nearer one (moving its finger, as a voice) and pairs with the farther one; if that doesn't fit, it pairs with the nearer one; if nothing fits, it plays legato from the nearer note alone.
- Releasing either note leaves the other on its string, unless the other key follows within `chord` (30 ms): then the stroke ends on both, as a chord released on a keyboard should (on the string, the bow stops on both). Notes pressed together join in the grip or the attack.
- Both strings get the same bow speed and position; each gets the force of its own band. Vibrato moves both fingers together; intonation by ear listens to the older note, and the other string uses the correction learned on it.
- The editor shows both notes and a contact mark on each string the bow touches. `strings-render play doublestops` renders an example.

**One way of playing** (`BowLift`, replacing the sustain, staccato and spiccato articulations; PLAN.md 4.3). A detached note is a new stroke whose velocity sets the attack; its end depends on the bow lift; overlapping notes play legato with the landing velocity setting the transition. The articulations follow from the gesture: staccato is short notes on the string, martelé the same pressed hard, spiccato-like short notes off the string, détaché long notes off it, portamento soft legato.
- The old staccato played a fixed stroke (100–160 ms) whatever the key length, and spiccato a scripted sin² touch; both are gone. Now a stroke lasts as long as the key, at least `min_stroke` (40 ms), so a tap still plays.
- On the string the stroke starts from a grip (15 ms of force before the bow moves) and accelerates over `grip_attack` = 100 ms at velocity 0 to 8 ms at 1; the old staccato always took 8–20 ms. At the end the bow stops (40 ms) and rests on the string until the next note, however long (it used to lift after 0.4 s). Switching to off string lifts a resting bow.
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
- **Attacks and rings.** The recorded pp and mf notes swell in over 300–450 ms (the player's slow start at the heel), ff notes in 30 ms; the model reaches its level in 10–40 ms at every dynamic, because the bite front-loads the stroke. After the stroke, the recording rings 0.25–0.55 s to −30 dB and the model 0.13–0.3 s: the model's finger eases off when the note ends (finger damping, "Phase 3 notes: tuning and first listening"), while this player keeps it down. Neither is a clear fault on its own: both depend on how the note is played.

Not yet done: listening to the pairs, the Guettler attack comparison (the mdw data), and recordings with vibrato and connected playing.

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

## 8. Alternatives to explore later

| Technique | What it buys | Why not now |
|---|---|---|
| **FDTD stiff string** (Bilbao; Willemsen's real-time work) | Accurate stiffness and loss from physical constants, two polarizations, fingerboard collisions | Costs O(N) per sample. Changing pitch continuously needs dynamic grids, which are an active research area and prone to artifacts. Candidate for an "HQ solo" mode |
| **Modal synthesis** | Exact mode frequencies and damping per mode, easy to couple to other resonators, good for bodies and sympathetic resonance | Low strings need 150+ modes. Bow coupling requires summing all modes each sample |
| **Torsional waves** | A second waveguide per string, coupled at the bow. Known to affect the stick/slip trigger and attack quality (Woodhouse) | Implemented in Phase 1b (3.6); it doesn't help until the damping is realistic |
| **Thermal friction model** (Woodhouse) | Friction depends on the rosin's temperature, giving better attacks and hysteresis | Planned for Phase 4. Needs its own solver work |
| **LuGre / elasto-plastic friction** | Micro-slip, dynamic hysteresis, smooth transitions | Multi-state, so there is no closed-form solve and it needs iteration; high CPU cost for sections |
| **Finite bow width / 3D bow-hair ribbon** | A realistic contact patch, hair compliance, the torsional interaction of the hair | Beyond real-time today. A lumped hair compliance is in since Phase 2 (`BowHair`); a finite width (a few contact points) is a cheaper next step worth trying |
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
