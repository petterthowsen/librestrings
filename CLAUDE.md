# CLAUDE.md

A physically modeled bowed-string synthesizer in Rust. The eventual target is a CLAP plugin; for now it is a DSP library plus an offline renderer. [PLAN.md](PLAN.md) is the roadmap and design reference; read the relevant section before changing DSP.

## Commands

```sh
cargo test                                   # unit + physics tests (test profile is optimized)
cargo clippy --all-targets                   # must be clean
cargo fmt
cargo run --release -p strings-render -- bow --string A -o out/a.wav
cargo run --release -p strings-render -- schelleng --string A   # playability map
```

Renders go in `out/` (gitignored).

## Layout

- `crates/strings-dsp`: the model. `string.rs` holds the waveguide, `bow.rs` the friction junction and Schelleng limits, `delay.rs` and `filters.rs` the building blocks, `presets.rs` the instrument data, and `analysis.rs` the offline measurements.
- `crates/strings-render`: the CLI (clap, hound).
- `docs/`: research notes. **Not fully reliable**; PLAN.md's "Research notes / corrections" lists known errors, such as swapped Schelleng formulas and commuted synthesis misapplied to bowing.

## Rules

- **Real-time safety:** everything in `strings-dsp` except `analysis` must never allocate, lock or do I/O in `process`/`solve`. Allocate in constructors.
- **SI units everywhere:** m/s, N, kg/s, seconds, Hz. Parameters must not depend on the sample rate; convert using `fs` (see how `loss_lowpass` is defined at 48 kHz).
- **DSP changes must keep the physics tests passing** (`crates/strings-dsp/tests/physics.rs`: tuning ±1 cent, decay, Helmholtz motion, Schelleng extremes). For changes to the bow or string, also run the `schelleng` map and compare it before and after.
- The bow solver's stick/slip hysteresis (Friedlander) is deliberate. Don't "simplify" it into a stateless solve.
- Keep plugin or framework dependencies out of `strings-dsp`; it must build and test with no audio I/O.
- When a phase or finding changes, update PLAN.md (the phase table and the "Phase 0–1 notes"-style sections).

## Known model behavior (don't "fix" without reason)

- The simulated lower force limit sits about 5–10× above Schelleng's theoretical F_min; the upper limit matches F_max. Anchor force defaults on F_max (the renderer and tests use 0.3 × F_max).
- The bridge loss lowpass at 0.5 (at 48 kHz) is needed for a clean Helmholtz band at 48 kHz; brighter settings fragment the slip phase.
- A bow stopped on the string damps it slowly (no bow-hair damping yet; planned for Phase 2).
