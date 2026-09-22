# Physically Modeled Strings — Plan

A CLAP instrument plugin, written in Rust, that synthesizes bowed string instruments by physical modeling.

- **Long-term scope:** violin, viola, cello and double bass, each playable as a **solo** instrument or as a **section** of up to about 12 players (the limit depends on CPU cost). Sections are placed on a stereo stage and humanized.
- **First milestone:** solo violin only.

Background research is in [docs/](docs/). This plan corrects several errors in that research; see [Research notes](#research-notes--corrections).

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
      instrument.rs        # N strings + string selection + fingering + body
      performer.rs         # gesture layer: articulations, bow strokes, vibrato
      section.rs           # (later) N players + humanization + stage placement
      presets/             # per-instrument physical data (violin, viola, cello, bass)
  strings-render/          # CLI: gesture script / MIDI file -> WAV (+ CSV of internal signals)
  strings-plugin/          # nih-plug wrapper, CLAP export, param + MIDI mapping
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
- **Bridge reflection:** `-g · lowpass`. Start with a one-pole filter. This sets decay time and brightness.
- **Finger/nut reflection:** `-1` for open strings. A stopped note is simply a shorter nut-side delay.
- **Fractional delay:** 3rd-order Lagrange on the nut-side delay, because its length is modulated by vibrato, legato and portamento. Thiran allpass sounds cleaner for fixed lengths but produces transients when its length is modulated, so we keep it only as an option for the bridge side.
- **Output:** force at the bridge, `F = Z · (v_in − v_out)`, which then goes through the body filter.
- **Stiffness/dispersion (Phase 4):** a cascade of 1st-order allpasses in the loop, with tuning re-compensated. It matters much more for bass and cello than for violin.

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
- Tuned to the known violin signature modes:
  - A0 at about 275 Hz
  - CBR
  - B1− and B1+ at about 450–550 Hz
  - the "bridge hill" at about 2–3 kHz
  - plus a broadband tail
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
| Violin | G3 D4 A4 E5 | ~328 mm | Milestone 1 |
| Viola | C3 G3 D4 A4 | ~370–420 mm | Size varies a lot between instruments |
| Cello | C2 G2 D3 A3 | ~690 mm | Stiffness matters more |
| Double bass | E1 A1 D2 G2 | ~1040–1060 mm | Stiffness and dispersion are clearly audible. Longer loops need more delay memory |

Per string we store tension, linear density (which gives `Z`), and loss parameters. Per instrument we store body modes, bow friction parameters, and typical ranges of `β`, `v_b` and `F_b`.

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
| Keyswitches | White keys, starting at the first C below the instrument's lowest note (violin: C3 D3 E3 F3, with C4 = middle C) | Articulation selection. If more keyswitches are needed than fit below the range, start one octave lower |

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

- No allocation, locks or I/O in `process()`. Enforce it with [`assert_no_alloc`](https://crates.io/crates/assert_no_alloc) in debug builds.
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
| **2. Solo violin (offline)** | 4 strings, string selection, fingering, legato, vibrato, biquad body, performer layer with the 4 articulations, bow-hair damping so a bow stopped on the string silences it quickly | Scripted phrases (scales, legato lines, staccato runs) sound like a violin, not a synth |
| **3. CLAP plugin** | nih-plug wrapper, CLAP-only export, CC1/CC11/vibrato mapping, keyswitches, parameters, real-time safety | Playable in Bitwig and Reaper; no allocations in the audio thread; CPU cost measured |
| **4. Realism pass** | Stiffness allpass, thermal friction, finger damping at note changes, bow noise, oversampling decision, bouncing-bow spiccato, sympathetic string coupling | A/B against recordings; clear improvement on attacks and legato transitions |
| **5. More instruments** | Viola, cello, double bass presets and bodies | Each instrument is convincing across its range |
| **6. Sections** | N-player engine, humanization, stage placement, SIMD across players | 12-player violin section within the CPU budget; sounds like a section, not a chorus effect |
| **7. Extended techniques** | Tremolo, trills, pizzicato, double stops, harmonics, mutes, MPE | — |

**CPU budgets to confirm in Phase 3** (per instance, one core at 48 kHz):
- Solo: < 3%
- 12-player section: < 25%

---

### Phase 0–1 notes

- Run the tests with `cargo test`. Renderer: `cargo run --release -p strings-render -- <pluck|bow|schelleng> --help`. Output goes to `out/` (gitignored).
- Free-string tuning is within ±1 cent for all four violin strings, stopped up to an octave, at 44.1, 48 and 96 kHz.
- Moderate bowing gives one slip per period with a slip fraction close to β, and the bridge force has the sawtooth shape of Helmholtz motion.
- `strings-render schelleng` prints the simulated regime map next to Schelleng's prediction. About 80% agree; the disagreement is the lower-limit offset described in 4.2.
- String loss is split between the bridge and nut reflections in proportion to segment length, so a segment trapped by a sticking bow still decays.

## 8. Alternatives to explore later

| Technique | What it buys | Why not now |
|---|---|---|
| **FDTD stiff string** (Bilbao; Willemsen's real-time work) | Accurate stiffness and loss from physical constants, two polarizations, fingerboard collisions | Costs O(N) per sample. Changing pitch continuously needs dynamic grids, which are an active research area and prone to artifacts. Candidate for an "HQ solo" mode |
| **Modal synthesis** | Exact mode frequencies and damping per mode, easy to couple to other resonators, good for bodies and sympathetic resonance | Low strings need 150+ modes. Bow coupling requires summing all modes each sample |
| **Torsional waves** | A second waveguide per string, coupled at the bow. Known to affect the stick/slip trigger and attack quality (Woodhouse) | Doubles the string cost. Needs tuning data. A good early Phase 4+ experiment |
| **Thermal friction model** (Woodhouse) | Friction depends on the rosin's temperature, giving better attacks and hysteresis | Planned for Phase 4. Needs its own solver work |
| **LuGre / elasto-plastic friction** | Micro-slip, dynamic hysteresis, smooth transitions | Multi-state, so there is no closed-form solve and it needs iteration; high CPU cost for sections |
| **Finite bow width / 3D bow-hair ribbon** | A realistic contact patch, hair compliance, the torsional interaction of the hair | Beyond real-time today. A finite width (a few contact points) is a cheaper approximation worth trying |
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
- **A rigidly sticking bow traps energy.** With the bow stopped on the string, the nut-side segment only decays at the string's own rate (about −19 dB after 200 ms on the A string). Real staccato stops are faster because the bow hair is compliant and lossy; this needs modeling in Phase 2.
- **Body data.** Sourcing measured violin body IRs under a usable license is unsolved. The biquad bank tuned from published mode frequencies is the fallback.
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
- A GUI: nih-plug supports egui, iced and vizia. Is one needed before Phase 5?
- The product name (it replaces the `strings-*` crate placeholders).
