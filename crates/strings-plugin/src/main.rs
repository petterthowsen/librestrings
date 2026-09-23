//! Standalone app for playing without a DAW: `cargo run --release -p
//! strings-plugin --features standalone`. Run with `--help` for the audio
//! and MIDI backend options (JACK by default, falling back to ALSA).

use nih_plug::prelude::*;
use strings_plugin::Strings;

fn main() {
    nih_export_standalone::<Strings>();
}
