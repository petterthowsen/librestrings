# Physically Modeled Strings — Plan

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
      performer.rs         # gesture layer: string choice, fingering, articulations, bow strokes, vibrato
      section.rs           # (later) N players + humanization + stage placement
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

**Aliasing.** The stick/slip transitions are corners, which alias at high `F_b`. Because a DWG is cheap, the option is to run the **whole string at 2× oversampling** rather than only the junction. Measure before deciding.

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
- **Implementation (done):** `TorsionSpec` on `StringSpec`, with two round-trip delay lines, one on each side of the bow, both reflecting with −1. The loss per period is the same for every mode (no lowpass). The torsional lines scale with the stopped pitch. The bridge-side line is clamped to 2 samples, which matters only below β ≈ 0.02.
- **Cost:** roughly one extra short delay line and a filter per string. That is cheap next to the transverse loop, but it matters for 12-player sections.
- **Checks:** Helmholtz motion still passes the physics tests, and the measured comparison shows the effect, especially the lower force limit and small β.

---

## 4. Performance layer (gestures → physics)

A physical model is only as playable as its control mapping. This layer turns MIDI into bow speed, bow force, bow position and finger position over time.

### 4.1 Controls (initial)

| Control | Default CC | Maps to |
|---|---|---|
| **Dynamics** | CC1 (mod wheel) | A path through (`v_b`, `F_b`, `β`) space. See 4.2 |
| **Expression** | CC11 | Output gain (post-body), for phrasing and fades |
| **Vibrato depth** | CC21 | Depth of finger-position modulation. 0 means none. Rate is a parameter. CC21 is the usual vibrato CC in orchestral sample libraries; General MIDI assigns vibrato to CC1, but here CC1 is dynamics |
| Velocity | — | Attack intensity for short notes; legato transition speed |
| Keyswitches | White keys, starting at the first C below the instrument's lowest note (cello: C1 D1 E1 F1; violin: C3 D3 E3 F3; C4 = middle C) | Articulation selection. If more keyswitches are needed than fit below the range, start one octave lower |

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
- A "bow pressure" parameter biases `p`: toward 0 gives a flautando, surface sound; toward 1 gives a crunchy, pressed sound.
- As players do, the bow also moves slightly toward the bridge at high dynamics.
- `R` has to be derived from the bridge loss filter. Calibrate empirically in the renderer.
- **Measured in Phase 1:** the simulated upper edge follows `F_max` closely, but the lower edge sits about 5–10× above the formula's `F_min` (same 1/β² slope). The dynamics mapping should anchor on `F_max` or on a calibrated lower edge, not on the raw `F_min`.
- **Against a real string** (see "Measured comparison" under Phase 0–1 notes): the measured lower edge follows roughly 1/β (fitted exponents −0.9 to −1.3), not 1/β². This is in line with Schoonderwaldt et al. (2008) and Mansour et al. (2017). A calibrated lower edge should be fitted to measurements, not derived from the formula.
- **As built (Phase 2):** `instrument::ForceLimits` stores both edges per string as `F = c·Z·v_b·β^α`, fitted by `strings-render calibrate` to the model's own simulated maps (the performer needs forces where the *model* plays Helmholtz). The measured lower edge is lower still; see "Phase 2 notes". The performer uses p = 0.65, tilted to 0.8 at pp and 0.5 at ff.

### 4.3 Articulations (initial set)

| Articulation | How it is triggered | Physics |
|---|---|---|
| **Legato** | Overlapping notes (mono per instrument) | No new bow stroke. Finger moves: the nut-side delay glides to the new length over ~10–40 ms. Velocity sets speed; low velocity gives a slower, portamento-like slide. If the new note is on another string, it is a string crossing: bow force ramps from the old string to the new one |
| **Détaché / sustain** | Non-overlapping note, default | New bow stroke, bow direction alternates. `v_b` rises over ~30–80 ms; `F_b` follows the dynamics mapping |
| **Staccato** (on the string) | Keyswitch | Force is pre-loaded before bow speed bursts (a martelé-style bite). The bow then **stops on the string**, and the stuck bow damps the string, so the note ends quickly and physically |
| **Spiccato** | Keyswitch | The bow is thrown: a short bell-shaped `F_b` pulse (~20–60 ms) with a speed burst. The bow **leaves the string**, so the string rings on with its natural decay. Phase 4: replace the scripted pulse with a bouncing bow (a mass on a spring) |

**Choosing short articulations.** A real-time plugin can't know a note's length at note-on, so short articulations come from keyswitches, with an optional "velocity above X means short" mode. Legato is detected automatically from overlap while in sustain mode.

All CC and keyswitch assignments are defaults; they become user-adjustable once there is a GUI.

### 4.4 Vibrato

- Modulates finger position, which modulates the nut-side delay.
- Depth ±0.1–0.35 semitones, set by the CC. Rate 5–7 Hz, as a parameter.
- Humanized: slow random drift in rate and depth, and a delayed onset on long notes.

### 4.5 Deferred

Tremolo, trills, double stops, pizzicato (pluck exciter on the same string engine), harmonics, col legno, con sordino, automatic bow changes when the bow runs out.

---

## 5. Sections (later phase)

A section is **N independent players**. Each player is a full `Instrument` plus `Performer`, with its own humanization, placed on a stage.

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
| **3. CLAP plugin** ⚠️ | nih-plug wrapper, CLAP-only export, CC1/CC11/vibrato mapping, keyswitches, parameters, real-time safety | Playable in Bitwig and Reaper; no allocations in the audio thread; CPU cost measured. **Built:** plugin, editor and standalone app; engine at 2.1% of real time. **Not yet tried in Bitwig or Reaper.** See "Phase 3 notes" |
| **4. Realism pass** | Thermal friction, finger damping at note changes, bow noise, oversampling decision, bouncing-bow spiccato, sympathetic string coupling | A/B against recordings; clear improvement on attacks and legato transitions (the mdw attack data gives a measured target for attacks) |
| **5. More instruments** | Violin, viola and double bass presets and bodies | Each instrument is convincing across its range |
| **6. Sections** | N-player engine, humanization, stage placement, SIMD across players | 12-player section within the CPU budget; sounds like a section, not a chorus effect |
| **7. Extended techniques** | Tremolo, trills, pizzicato, double stops, harmonics, mutes, MPE | — |

**CPU budgets to confirm in Phase 3** (per instance, one core at 48 kHz):
- Solo: < 3%
- 12-player section: < 25%

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
  - frequency-dependent torsional loss (a lowpass in the torsional loop), with the torsional Q from the literature rather than fitted;
  - finite bow width and bow-hair compliance (§8);
  - the friction model (thermal friction, Phase 4), which Woodhouse links to the minimum bow force.
- `strings-render measured` takes these overrides:
  - `--t60` / `--loss-lowpass`: switch to the one-pole loss;
  - `--damping-exponent`;
  - `--bending-stiffness`, `--no-torsion`, `--torsion-impedance`, `--torsion-ratio` and `--torsion-q`.

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
- **Controls:** CC1 dynamics, CC11 expression, CC21 vibrato, CC123 releases all notes gracefully (`Performer::release_all`), CC120 silences at once. Dynamics, expression, vibrato, pressure, articulation and volume are also host parameters. **Whichever changed last wins:** a parameter only acts when its value changes, so a CC keeps its value until the parameter moves.
- **Keyswitches:** the white keys from the first C below the lowest note (cello: C1 sustain, D1 staccato, E1 spiccato).
- **CPU:** 2.1% of real time on one core at 48 kHz for the engine (performer plus telemetry, 256-sample blocks, notes changing every 0.5 s; `cargo test --release -p strings-plugin cpu_cost -- --ignored --nocapture`). The standalone's load meter showed about 4% while idle-playing a note; probably CPU frequency scaling under light real-time load. To confirm in a DAW.

**Editor** (`crates/strings-plugin/src/editor/`, layout from ROADMAP.md)
- Status row: sample rate, block size, DSP load (smoothed and peak, yellow above the 3% budget), output peak, NaN resets; toggles for the computer keyboard and a debug view.
- Instrument and ensemble selection (only cello and solo are enabled), articulation buttons that follow keyswitches.
- The instrument view is a placeholder drawing. The strings are drawn in parts (nut to finger, finger to bridge, afterlength) from the performer's state: the finger where the vibrating length starts, the bow at β moving along its length with the bow velocity, and Helmholtz motion (a corner on a parabolic envelope) on the vibrating part, slowed down for display with an amplitude from the string's bridge force. It is stylized, not the simulated string shape.
- Readout: note, string, position, pitch, β, bow speed and force, and the motion from slips per period (Helmholtz at one per period, timed from slip onsets; exact for periodic motion). The debug view adds the calibrated force band and the bow's position in it, and per-string pitch, contact and level.
- Bottom: a piano from C1 to C6 (mouse: lower on a key is louder, dragging plays legato; keyswitches colored, out-of-range keys grey), the computer keyboard in the tracker layout (Q = C, 2 = C♯ … P = E, by physical key position; Z/X transpose by octave) with a velocity setting, and faders for the parameters. A marker on a fader shows the performer's value when a CC has moved it away from the parameter.
- Editor and audio thread share only atomics (telemetry, published once per block) and a lock-free queue (notes from the editor).

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
- **Aliasing** from stick/slip corners at high bow force. Decide on oversampling by measurement.
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
- Whether to use the whole-string 2× oversampling mode by default.
- The product name (it replaces the `strings-*` crate placeholders).
