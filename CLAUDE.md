# CLAUDE.md

A physically modeled bowed-string synthesizer in Rust. The eventual target is a CLAP plugin; for now it is a DSP library plus an offline renderer. [PLAN.md](PLAN.md) is the roadmap and design reference; read the relevant section before changing DSP.

## Commands

```sh
cargo test                                   # unit + physics tests (test profile is optimized)
cargo clippy --all-targets                   # must be clean
cargo fmt
cargo run --release -p strings-render -- play scale -o out/scale.wav   # solo cello: scale | legato | staccato | phrase | file.score
cargo run --release -p strings-render -- bow --string A -o out/a.wav
cargo run --release -p strings-render -- schelleng --string A   # playability map (--instrument cello for cello strings)
cargo run --release -p strings-render -- calibrate             # cello force band (ForceLimits) from simulated maps
cargo run --release -p strings-render -- measured              # vs measured cello string (needs scripts/fetch-reference-data.sh)
```

Renders go in `out/` (gitignored).

## Layout

- `crates/strings-dsp`: the model. `string.rs` holds the waveguide (with stiffness, torsion and bow hair), `bow.rs` the friction junction and Schelleng limits, `loss.rs` the loop-loss models, `delay.rs` and `filters.rs` the building blocks, `body.rs` the body resonators, `instrument.rs` four strings and a body with the calibrated force band, `performer.rs` the gesture layer (notes and controllers to bow and finger), `presets.rs` the instrument data, and `analysis.rs` the offline measurements.
- `crates/strings-render`: the CLI (clap, hound). `score.rs` is the text score format for `play`; `scores/` has examples.
- `docs/`: research notes, plus `Literature.md` (papers the model takes numbers from, with links). The research notes are **not fully reliable**; PLAN.md's "Research notes / corrections" lists known errors, such as swapped Schelleng formulas and commuted synthesis misapplied to bowing.

## Rules

- **Real-time safety:** everything in `strings-dsp` except `analysis` must never allocate, lock or do I/O in `process`/`solve`. Allocate in constructors.
- **SI units everywhere:** m/s, N, kg/s, seconds, Hz. Parameters must not depend on the sample rate; convert using `fs` (see how `Loss::OnePole::lowpass` is defined at 48 kHz).
- **DSP changes must keep the physics tests passing** (`crates/strings-dsp/tests/physics.rs`: tuning ±1 cent, decay, Helmholtz motion, Schelleng extremes). For changes to the bow or string, also run the `schelleng` map and compare it before and after.
- The bow solver's stick/slip hysteresis (Friedlander) is deliberate. Don't "simplify" it into a stateless solve.
- Keep plugin or framework dependencies out of `strings-dsp`; it must build and test with no audio I/O.
- When a phase or finding changes, update PLAN.md (the phase table and the "Phase 0–1 notes"-style sections).
- [STATUS.md](STATUS.md) lists the open issues. Remove an entry when you fix it, and add one when a finding leaves something unresolved.

## Known model behavior (don't "fix" without reason)

- The simulated lower force limit sits about 5–10× above Schelleng's theoretical F_min; the upper limit matches F_max. Anchor force defaults on F_max (the renderer and tests use 0.3 × F_max).
- The bridge loss lowpass at 0.5 (at 48 kHz) is needed for a clean Helmholtz band at 48 kHz; brighter settings fragment the slip phase.
- A rigid bow stopped on the string damps it slowly, and bow hair doesn't change that much. Notes stop cleanly because the performer eases the force with the bow speed (PLAN.md "Phase 2 notes"). Keep that coupling when changing strokes.
- Bow hair (`BowHair`) is off by default, so violin and reference results stay unchanged; the cello preset turns it on. Its parameters are fitted to the measured map.
- The cello strings play flat at β ≈ 0.124–0.156 (a torsion effect), so the performer's β range stays below 0.115. Bowed pitch also drifts with force; the performer intonates stopped notes by ear (slip-period feedback), which is deliberate, not a tuning bug.
- After changing the strings, bow or friction of the cello, re-run `calibrate` and paste its limits into `presets::cello::FORCE_LIMITS`.
- On the measured cello string, bending stiffness and torsion (Phase 1b) *shrink* the Helmholtz region, through real extra slips, even with the measured damping. This is not a bug (PLAN.md "Phase 1b results"). Violin presets have neither and use the one-pole loss.
- The measured loss (`Loss::Measured`) fits one filter per semitone in the constructor (about 7 ms per cello string). Don't construct strings on the audio thread.
