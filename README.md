# Physically Modeled Strings

A bowed-string synthesizer in Rust, built on physical modeling rather than samples. The sound comes from simulating a string and the stick-slip friction of a bow on it, sample by sample.

**Status: early development.** A solo cello (four strings, body, and a performer that turns notes and controllers into bowing) plays as a CLAP plugin with its own editor, as a standalone app, and offline from the command line. It hasn't been tested in a DAW yet. The goal is a CLAP instrument covering violin, viola, cello and double bass, played either solo or as sections of up to about 12 players. See [PLAN.md](PLAN.md) for the roadmap and [STATUS.md](STATUS.md) for open issues.

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

The plugin's editor and the standalone app need system libraries for the window (X11/XCB, OpenGL) and audio (ALSA, JACK). On Debian or Ubuntu:

```sh
sudo apt install libasound2-dev libgl-dev libjack-jackd2-dev \
  libx11-xcb-dev libxcb1-dev libxcb-dri2-0-dev libxcb-icccm4-dev \
  libxcb-shape0-dev libxcb-xfixes0-dev libxcursor-dev libxkbcommon-dev
```

`strings-dsp` and `strings-render` build without them.

```sh
cargo build --release
cargo test
```

## Plugin

Build the CLAP bundle, then copy it to your CLAP folder (on Linux, `~/.clap`):

```sh
cargo xtask bundle strings-plugin --release
cp target/bundled/Strings.clap ~/.clap/
```

It is a mono instrument (the same signal on both outputs) that takes MIDI:

| Input | Controls |
|---|---|
| Notes | Monophonic. Overlapping notes play legato |
| CC1 | Dynamics: bow speed and position, and with them loudness |
| CC11 | Expression: output level |
| CC21 | Vibrato depth |
| C1, D1, E1 | Keyswitches: sustain, staccato, spiccato |

Dynamics, expression, vibrato, pressure, articulation and volume are also plugin parameters that the host can automate. When a CC and a parameter both set a control, the one that changed last wins.

The editor shows the instrument, with the bow, finger and vibrating string following what the performer does, plus bow speed, bow force and whether the string is in clean Helmholtz motion. Below it are an on-screen keyboard and faders, so you can play without a MIDI controller:

- **Mouse:** click a key to play it. The lower you click on a key, the louder the note. Dragging across keys plays legato.
- **Computer keyboard:** `Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P` play C to E, in the tracker layout (the letter row plays the white keys, the number row the black keys). `Z` and `X` shift the octave. It works while the editor has keyboard focus; some hosts keep keys for their own shortcuts.
- **Debug** (top right) shows the force band, per-string data and the controls as the performer has them.

### Standalone app

To play without a DAW, run the plugin as a standalone app:

```sh
cargo run --release -p strings-plugin --features standalone -- --backend alsa
```

- `--backend` picks the audio: `alsa`, `jack`, or `dummy` (silent, for trying the editor). The default, `auto`, tries JACK, then ALSA, then falls back to `dummy`.
- With ALSA, pass `--output-device <name>` and `--midi-input <name>` to choose devices. Give an empty name (`--midi-input ""`) to list what's available.
- With JACK, the app connects to the running JACK server. Under PipeWire, start it through `pw-jack` (from the `pipewire-jack` package). `--connect-jack-midi-input <port>` connects a MIDI controller.
- `--sample-rate` (default 48000) and `--period-size` (default 512) set the ALSA buffer. A smaller period means lower latency.

Run with `--help` for all options.

## Offline renderer

`strings-render` drives the model offline and writes 32-bit float mono WAV files. `play` renders the whole cello through its body. The single-string commands below (`bow`, `pluck`, `schelleng`) use the violin strings and write the raw force on the bridge, peak-normalized to −1 dBFS; with no body, they sound thinner and buzzier than a real violin.

```sh
cargo run --release -p strings-render -- <command> [options]
# or, after building:
./target/release/strings-render <command> [options]
```

Add `--help` to any command for all options.

### `play`: the solo cello from a score

```sh
strings-render play phrase -o out/phrase.wav     # also: scale, legato, staccato
strings-render play my.score -o out/my.wav       # a score file
```

The score format is described in `crates/strings-render/src/score.rs`; `crates/strings-render/scores/` has examples. `--bridge-out <file>` also writes the bridge force before the body; `--pressure` and `--string-bias` change how it is played.

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
  strings-plugin/   The CLAP plugin and standalone app (nih-plug, egui editor).
xtask/              Bundles the plugin (`cargo xtask bundle`).
docs/               Background research and reference material.
PLAN.md             Design, roadmap, caveats and alternatives.
```

The crate names are placeholders until the project has a name.

## Contributing

It's early, and the design is still moving; [PLAN.md](PLAN.md) shows what's next. Before sending a change to the DSP, run `cargo test` and check `strings-render schelleng`. The physics tests guard tuning, stability and Helmholtz motion.

## License

To be decided.
