# Physically Modeled Strings

A bowed-string synthesizer in Rust, built on physical modeling rather than samples. The sound comes from simulating a string and the stick-slip friction of a bow on it, sample by sample.

**Status: early development.** A single bowed violin string works and can be rendered offline from the command line. There is no plugin yet. The goal is a CLAP instrument covering violin, viola, cello and double bass, played either solo or as sections of up to about 12 players. See [PLAN.md](PLAN.md) for the roadmap.

## How it works

- **String:** a digital waveguide. Velocity waves travel along delay lines between the bridge, the bow and the nut (or stopping finger). Loss and damping are lumped into the reflections; fractional delays keep the pitch within a cent.
- **Bow:** a friction junction at the bow point using a hyperbolic friction curve. It is solved in closed form every sample, so the cost per sample is fixed. The solver remembers whether the string was sticking or slipping, which it needs to choose correctly where more than one solution exists.
- **Playability:** the model reproduces the regimes of a real bowed string, as mapped by Schelleng's diagram:
  - clean **Helmholtz motion** at moderate bow force
  - a multi-slip "surface" sound when the force is too light
  - a raucous crunch when it is too heavy

  The controls are real physical quantities: bow force (N), bow speed (m/s) and bow position (a fraction of the string length from the bridge).

Section 3 of [PLAN.md](PLAN.md) has the equations and design decisions. [docs/](docs/) holds background research; the plan lists corrections to it.

## Building

You need a recent stable Rust toolchain (edition 2024).

```sh
cargo build --release
cargo test
```

## Offline renderer

`strings-render` drives the model offline and writes 32-bit float mono WAV files, peak-normalized to −1 dBFS. The output is the raw force on the bridge; there is no instrument body yet, so it sounds thinner and buzzier than a real violin.

```sh
cargo run --release -p strings-render -- <command> [options]
# or, after building:
./target/release/strings-render <command> [options]
```

Add `--help` to any command for all options.

### `bow`: bow a single note

The note has an attack, a sustain and an ending. `--end lift` lifts the bow and lets the string ring; `--end stop` stops the bow on the string.

```sh
# Open A string with default bowing
strings-render bow --string A -o out/a.wav

# D string stopped a fifth up, bowed fast and close to the bridge (sul ponticello)
strings-render bow --string D --semitones 7 --speed 0.3 --beta 0.05 -o out/d_pont.wav

# Too much force: the tone breaks into a raucous crunch
strings-render bow --string A --force 0.9 -o out/a_pressed.wav

# Also write per-sample internal signals as CSV
strings-render bow --string G --csv out/g.csv -o out/g.wav
```

| Option | Default | Meaning |
|---|---|---|
| `--string` | `A` | Violin string: `G`, `D`, `A` or `E` |
| `--semitones` | `0` | Stopped note, in semitones above the open string |
| `--force` | 0.3 × F_max | Bow force in newtons. The default sits inside the playable range |
| `--speed` | `0.1` | Bow speed in m/s |
| `--beta` | `0.1` | Bow position as a fraction of the string length, from the bridge (0.04–0.25 is sensible) |
| `--seconds` | `2` | Sustain length |
| `--attack`, `--release` | `0.08`, `0.15` | Ramp times in seconds |
| `--end` | `lift` | `lift` or `stop` |
| `--sample-rate` | `48000` | Any rate. Physical parameters are rate-independent |
| `--loss-lowpass` | preset | Overrides the string's high-frequency loss (0–1, higher is darker) |
| `--csv` | – | Writes `time, bridge_force, bow_point_velocity, friction_force, slipping` per sample |
| `-o`, `--out` | required | Output WAV path |

The renderer prints the force it used, along with Schelleng's theoretical force limits for that bow speed and position.

### `pluck`: a free string with no bow

Useful for checking tuning and decay by ear.

```sh
strings-render pluck --string E --semitones 12 -o out/e_pluck.wav
```

### `schelleng`: map where the bow produces a clean tone

This sweeps bow force against bow position and prints two maps side by side: what the simulation actually does, and what Schelleng's formulas predict.

```sh
strings-render schelleng --string G --speed 0.1
```

The symbols are `H` Helmholtz motion (a clean tone), `M` multi-slip (surface sound), `R` raucous and `.` no oscillation. This is the main tool for checking that a change to the bow or string still plays well.

## Project layout

```
crates/
  strings-dsp/      The model: waveguide string, bow friction, filters, presets.
                    Real-time safe (no allocation while processing), except the
                    offline `analysis` module used by tests and the renderer.
  strings-render/   Command-line offline renderer.
docs/               Background research and reference material.
PLAN.md             Design, roadmap, caveats and alternatives.
```

The crate names are placeholders until the project has a name.

## Contributing

It's early, and the design is still moving; [PLAN.md](PLAN.md) shows what's next. Before sending a change to the DSP, run `cargo test` and check `strings-render schelleng`. The physics tests guard tuning, stability and Helmholtz motion.

## License

To be decided.
