//! The solo cello as a CLAP plugin (PLAN.md Phase 3).
//!
//! The plugin is a thin layer over [`Performer`]: MIDI notes, CC1/CC11/CC21
//! and keyswitches in; the performer's mono output on every output channel.
//! The editor shows the performer's state and has an on-screen keyboard and
//! faders, so it can be played without a MIDI controller.
//!
//! Real-time rules as in `strings-dsp`: the performer is built in
//! `initialize` (about 30 ms), never in `process`, which doesn't allocate, lock
//! or wait (checked in debug builds by nih-plug's `assert_process_allocs`).

use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use nih_plug::prelude::*;
#[cfg(test)]
use strings_dsp::Articulation;
use strings_dsp::presets::cello;
use strings_dsp::{InstrumentSpec, Performer, PerformerSettings};

mod editor;
pub mod params;
pub mod shared;

use params::{ArticulationParam, StringsParams};
use shared::{GuiEvent, Shared, Telemetry};

/// The only instrument so far.
pub const INSTRUMENT: &InstrumentSpec = &cello::INSTRUMENT;

/// Default controller numbers (PLAN.md 4.1).
const CC_DYNAMICS: u8 = 1;
const CC_EXPRESSION: u8 = 11;
const CC_VIBRATO: u8 = 21;
const CC_ALL_SOUND_OFF: u8 = 120;
const CC_ALL_NOTES_OFF: u8 = 123;

/// MIDI note of a frequency, rounded.
pub fn midi_note(frequency: f32) -> u8 {
    (69.0 + 12.0 * (frequency / 440.0).log2()).round() as u8
}

/// The first keyswitch: the first C below the instrument's lowest note
/// (cello: C1). The white keys from there select the articulations.
pub fn keyswitch_base(spec: &InstrumentSpec) -> u8 {
    let lowest = midi_note(spec.strings[0].frequency);
    (lowest - 1) / 12 * 12
}

/// The articulation a keyswitch note selects, if it is one.
pub fn keyswitch(spec: &InstrumentSpec, note: u8) -> Option<ArticulationParam> {
    let base = keyswitch_base(spec);
    match note.checked_sub(base)? {
        0 => Some(ArticulationParam::Sustain),
        2 => Some(ArticulationParam::Staccato),
        4 => Some(ArticulationParam::Spiccato),
        _ => None,
    }
}

pub struct Strings {
    params: Arc<StringsParams>,
    shared: Arc<Shared>,
    engine: Option<Engine>,
}

impl Default for Strings {
    fn default() -> Self {
        Self {
            params: Arc::new(StringsParams::default()),
            shared: Arc::new(Shared::default()),
            engine: None,
        }
    }
}

/// Parameter values last passed to the performer, to detect changes.
#[derive(Clone, Copy)]
struct Applied {
    /// Dynamics, expression, vibrato, pressure.
    controls: [f32; 4],
    articulation: Option<ArticulationParam>,
}

impl Applied {
    const NONE: Self = Self {
        controls: [f32::NAN; 4],
        articulation: None,
    };
}

/// The performer and what the plugin measures around it.
struct Engine {
    performer: Performer,
    sample_rate: f32,
    applied: Applied,
    /// The controls as the performer has them: dynamics, expression, vibrato, pressure.
    controls: [f32; 4],

    // Accumulated over a block.
    level: [f32; 4],
    peak: f32,
    slips: SlipCounter,
    /// Bow velocity (m/s) and force (N) at the last sample.
    bow: (f32, f32),

    // Kept across blocks.
    slips_per_period: f32,
    load: f32,
    load_peak: f32,
    output_peak: f32,
    resets: u32,
}

impl Engine {
    fn new(sample_rate: f32) -> Self {
        let settings = PerformerSettings::default();
        Self {
            performer: Performer::new(INSTRUMENT, settings, sample_rate),
            sample_rate,
            applied: Applied::NONE,
            controls: [0.5, 1.0, 0.0, settings.pressure],
            level: [0.0; 4],
            peak: 0.0,
            slips: SlipCounter::default(),
            bow: (0.0, 0.0),
            slips_per_period: 0.0,
            load: 0.0,
            load_peak: 0.0,
            output_peak: 0.0,
            resets: 0,
        }
    }

    /// Passes changed parameters to the performer. A parameter only acts when
    /// it changes, so a CC moved since keeps its value until then.
    fn apply_params(&mut self, params: &StringsParams) {
        let values = [
            params.dynamics.value(),
            params.expression.value(),
            params.vibrato.value(),
            params.pressure.value(),
        ];
        for (i, value) in values.into_iter().enumerate() {
            if value != self.applied.controls[i] {
                self.applied.controls[i] = value;
                self.set_control(i, value);
            }
        }
        let articulation = params.articulation.value();
        if self.applied.articulation != Some(articulation) {
            self.applied.articulation = Some(articulation);
            self.performer.set_articulation(articulation.into());
        }
    }

    fn set_control(&mut self, i: usize, value: f32) {
        self.controls[i] = value;
        match i {
            0 => self.performer.set_dynamics(value),
            1 => self.performer.set_expression(value),
            2 => self.performer.set_vibrato(value),
            _ => self.performer.set_pressure(value),
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        match keyswitch(INSTRUMENT, note) {
            Some(articulation) => self.performer.set_articulation(articulation.into()),
            None => self.performer.note_on(note, velocity),
        }
    }

    fn note_off(&mut self, note: u8) {
        if keyswitch(INSTRUMENT, note).is_none() {
            self.performer.note_off(note);
        }
    }

    fn gui_event(&mut self, event: GuiEvent) {
        match event {
            GuiEvent::NoteOn { note, velocity } => self.note_on(note, velocity),
            GuiEvent::NoteOff { note } => self.note_off(note),
            GuiEvent::Articulation(a) => self.performer.set_articulation(a.into()),
        }
    }

    fn midi_event(&mut self, event: NoteEvent<()>) {
        match event {
            NoteEvent::NoteOn { note, velocity, .. } => self.note_on(note, velocity),
            NoteEvent::NoteOff { note, .. } => self.note_off(note),
            NoteEvent::MidiCC { cc, value, .. } => match cc {
                CC_DYNAMICS => self.set_control(0, value),
                CC_EXPRESSION => self.set_control(1, value),
                CC_VIBRATO => self.set_control(2, value),
                CC_ALL_NOTES_OFF => self.performer.release_all(),
                CC_ALL_SOUND_OFF => self.performer.reset(),
                _ => {}
            },
            _ => {}
        }
    }

    /// One sample, before the output gain.
    fn tick(&mut self) -> f32 {
        let frame = self.performer.process_frame();
        if !frame.output.is_finite() {
            // PLAN.md 6: a silent recovery in release builds.
            debug_assert!(false, "non-finite output");
            self.performer.reset();
            self.resets += 1;
            self.slips.restart();
            return 0.0;
        }
        for (level, f) in self.level.iter_mut().zip(self.performer.string_frames()) {
            *level += f.bridge_force * f.bridge_force;
        }
        self.slips.tick(frame.frame.state.is_slipping());
        self.bow = (frame.bow_velocity, frame.bow_force);
        frame.output
    }

    /// Publishes the block's measurements and resets the accumulators.
    fn publish(&mut self, t: &Telemetry, samples: usize, elapsed: Duration) {
        let block = samples as f32 / self.sample_rate;
        let p = &self.performer;
        let bowed = p.bowed_string();
        let instrument = p.instrument();

        let load = elapsed.as_secs_f32() / block;
        self.load += (load - self.load) * (1.0 - (-block / 0.3).exp());
        self.load_peak = load.max(self.load_peak * (-block / 1.5).exp());
        self.output_peak = self.peak.max(self.output_peak * (-block / 0.3).exp());

        let sounding = p.note().is_some() && p.contact(bowed) > 0.5;
        if !sounding {
            self.slips.restart();
            self.slips_per_period = 0.0;
        } else if let Some(slips) = self
            .slips
            .measure(instrument.string(bowed).frequency(), self.sample_rate)
        {
            self.slips_per_period = slips;
        }

        t.ready.store(true, Relaxed);
        t.sample_rate.store(self.sample_rate, Relaxed);
        t.block_size.store(samples as u32, Relaxed);
        t.load.store(self.load, Relaxed);
        t.load_peak.store(self.load_peak, Relaxed);
        t.output_peak.store(self.output_peak, Relaxed);
        t.resets.store(self.resets, Relaxed);
        t.note.store(p.note().map_or(-1, i32::from), Relaxed);
        t.string.store(bowed as u32, Relaxed);
        for (i, s) in t.strings.iter().enumerate() {
            let string = instrument.string(i);
            s.frequency.store(string.frequency(), Relaxed);
            s.beta.store(string.beta(), Relaxed);
            s.contact.store(p.contact(i), Relaxed);
            s.level
                .store((self.level[i] / samples as f32).sqrt(), Relaxed);
        }
        t.bow_velocity.store(self.bow.0, Relaxed);
        t.bow_force.store(self.bow.1, Relaxed);
        t.slips_per_period.store(self.slips_per_period, Relaxed);
        let controls = [&t.dynamics, &t.expression, &t.vibrato, &t.pressure];
        for (atomic, value) in controls.into_iter().zip(self.controls) {
            atomic.store(value, Relaxed);
        }
        let articulation = ArticulationParam::from(p.articulation());
        t.articulation.store(articulation as u32, Relaxed);

        self.level = [0.0; 4];
        self.peak = 0.0;
    }
}

/// Counts slip onsets on the bowed string: one per period is Helmholtz motion.
///
/// Measured from the span between the first and last onset in a window, which
/// is exact for periodic motion; whole onsets per block would be off by up to
/// one slip per window.
#[derive(Default)]
struct SlipCounter {
    was_slipping: bool,
    /// Samples since the window began.
    clock: u32,
    onsets: u32,
    first: u32,
    last: u32,
}

impl SlipCounter {
    /// Periods in a window.
    const WINDOW: f32 = 8.0;

    fn tick(&mut self, slipping: bool) {
        if slipping && !self.was_slipping {
            if self.onsets == 0 {
                self.first = self.clock;
            }
            self.last = self.clock;
            self.onsets += 1;
        }
        self.was_slipping = slipping;
        self.clock += 1;
    }

    /// Slips per period of `frequency` once the window is full, which then
    /// starts a new one. No slips at all (a sticking or lifted bow) is 0.
    fn measure(&mut self, frequency: f32, sample_rate: f32) -> Option<f32> {
        let period = sample_rate / frequency;
        if (self.clock as f32) < Self::WINDOW * period {
            return None;
        }
        let slips = if self.onsets >= 2 {
            (self.onsets - 1) as f32 * period / (self.last - self.first) as f32
        } else {
            0.0
        };
        self.restart();
        Some(slips)
    }

    fn restart(&mut self) {
        *self = Self {
            was_slipping: self.was_slipping,
            ..Self::default()
        };
    }
}

impl Plugin for Strings {
    const NAME: &'static str = "Strings";
    const VENDOR: &'static str = "Strings";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(1),
            ..AudioIOLayout::const_default()
        },
    ];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.shared.clone())
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        let fs = buffer_config.sample_rate;
        // `initialize` may be called again with nothing changed; building the
        // instrument takes about 30 ms, so keep it when the rate is the same.
        if self.engine.as_ref().is_none_or(|e| e.sample_rate != fs) {
            self.engine = Some(Engine::new(fs));
        }
        if let Some(engine) = &mut self.engine {
            engine.applied = Applied::NONE;
        }
        true
    }

    fn reset(&mut self) {
        if let Some(engine) = &mut self.engine {
            engine.performer.reset();
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let start = Instant::now();
        let Some(engine) = &mut self.engine else {
            for channel in buffer.as_slice() {
                channel.fill(0.0);
            }
            return ProcessStatus::Normal;
        };
        engine.apply_params(&self.params);
        while let Some(event) = self.shared.gui_events.pop() {
            engine.gui_event(event);
        }

        let samples = buffer.samples();
        let mut next_event = context.next_event();
        for (i, channels) in buffer.iter_samples().enumerate() {
            while let Some(event) = next_event {
                if event.timing() as usize > i {
                    break;
                }
                engine.midi_event(event);
                next_event = context.next_event();
            }
            let out = engine.tick() * self.params.volume.smoothed.next();
            engine.peak = engine.peak.max(out.abs());
            for sample in channels {
                *sample = out;
            }
        }
        // Events timed past the block (shouldn't happen) still count.
        while let Some(event) = next_event {
            engine.midi_event(event);
            next_event = context.next_event();
        }

        engine.publish(&self.shared.telemetry, samples, start.elapsed());
        // An instrument rings on after its notes end, and the editor animates it.
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for Strings {
    // Placeholder until the product has a name (PLAN.md "Open questions").
    const CLAP_ID: &'static str = "dev.strings.cello";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Physically modeled bowed cello");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
        ClapFeature::Mono,
    ];
}

nih_export_clap!(Strings);

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 48_000.0;

    fn cc(cc: u8, value: f32) -> NoteEvent<()> {
        NoteEvent::MidiCC {
            timing: 0,
            channel: 0,
            cc,
            value,
        }
    }

    fn note_on(note: u8) -> NoteEvent<()> {
        NoteEvent::NoteOn {
            timing: 0,
            voice_id: None,
            channel: 0,
            note,
            velocity: 0.8,
        }
    }

    /// Runs `seconds` in blocks of 256, publishing after each.
    fn run(engine: &mut Engine, t: &Telemetry, seconds: f32) {
        for _ in 0..(seconds * FS / 256.0) as usize {
            for _ in 0..256 {
                engine.tick();
            }
            engine.publish(t, 256, Duration::ZERO);
        }
    }

    #[test]
    fn keyswitches_are_the_white_keys_from_c1() {
        assert_eq!(keyswitch_base(INSTRUMENT), 24);
        assert_eq!(keyswitch(INSTRUMENT, 24), Some(ArticulationParam::Sustain));
        assert_eq!(keyswitch(INSTRUMENT, 26), Some(ArticulationParam::Staccato));
        assert_eq!(keyswitch(INSTRUMENT, 28), Some(ArticulationParam::Spiccato));
        assert_eq!(keyswitch(INSTRUMENT, 25), None);
        assert_eq!(keyswitch(INSTRUMENT, 36), None);
    }

    #[test]
    fn slip_counter_is_exact_for_periodic_motion() {
        // Not a whole number of samples per period, and windows that don't
        // start on a slip.
        for (period, slips_per_period) in [(274.3f32, 1.0f32), (91.7, 1.0), (274.3, 2.0)] {
            let mut counter = SlipCounter::default();
            let spacing = period / slips_per_period;
            let mut measured = Vec::new();
            for n in 0..(40.0 * period) as usize {
                let phase = (n as f32 + 17.0) % spacing;
                counter.tick(phase < 5.0);
                if n % 256 == 255
                    && let Some(m) = counter.measure(FS / period, FS)
                {
                    measured.push(m);
                }
            }
            assert!(!measured.is_empty());
            for m in measured {
                assert!(
                    (m - slips_per_period).abs() < 0.01,
                    "{m} for {slips_per_period}"
                );
            }
        }
    }

    #[test]
    fn a_note_from_midi_reaches_helmholtz_motion_and_releases() {
        let params = StringsParams::default();
        let t = Telemetry::default();
        let mut engine = Engine::new(FS);
        engine.apply_params(&params);
        engine.midi_event(note_on(50));
        run(&mut engine, &t, 0.6);
        assert_eq!(t.note(), Some(50));
        let slips = t.slips_per_period.load(Relaxed);
        assert!((slips - 1.0).abs() < 0.02, "{slips} slips per period");
        let bowed = t.string.load(Relaxed) as usize;
        assert!(t.strings[bowed].contact.load(Relaxed) > 0.99);
        assert!(t.strings[bowed].level.load(Relaxed) > 0.0);

        engine.midi_event(cc(CC_ALL_NOTES_OFF, 0.0));
        run(&mut engine, &t, 0.5);
        assert_eq!(t.note(), None);
        assert_eq!(t.slips_per_period.load(Relaxed), 0.0);
    }

    #[test]
    fn keyswitch_changes_articulation_without_playing() {
        let params = StringsParams::default();
        let t = Telemetry::default();
        let mut engine = Engine::new(FS);
        engine.apply_params(&params);
        engine.midi_event(note_on(26));
        run(&mut engine, &t, 0.05);
        assert_eq!(t.note(), None);
        assert_eq!(t.articulation(), ArticulationParam::Staccato);
        // The unchanged parameter doesn't take it back.
        engine.apply_params(&params);
        assert_eq!(engine.performer.articulation(), Articulation::Staccato);
    }

    #[test]
    fn a_cc_holds_until_the_parameter_changes() {
        let params = StringsParams::default();
        let mut engine = Engine::new(FS);
        engine.apply_params(&params);
        assert_eq!(engine.controls[0], 0.5);
        engine.midi_event(cc(CC_DYNAMICS, 0.9));
        engine.apply_params(&params);
        assert_eq!(engine.controls[0], 0.9);
        // Only a change of the parameter (automation or the fader) takes over.
        engine.applied.controls[0] = 0.4;
        engine.apply_params(&params);
        assert_eq!(engine.controls[0], 0.5);
    }
}

/// CPU cost of the plugin's engine, measured flat out:
/// `cargo test --release -p strings-plugin cpu_cost -- --ignored --nocapture`.
#[cfg(test)]
mod cpu {
    use super::*;

    #[test]
    #[ignore]
    fn cpu_cost() {
        let fs = 48_000.0;
        let params = StringsParams::default();
        let t = Telemetry::default();
        let mut engine = Engine::new(fs);
        engine.apply_params(&params);
        engine.set_control(2, 0.6);
        let seconds = 20.0;
        let blocks = (seconds * fs / 256.0) as usize;
        let start = Instant::now();
        for b in 0..blocks {
            // A new note every half second, alternating two strings.
            if b % 94 == 0 {
                let note = if b % 188 == 0 { 50 } else { 57 };
                engine.note_on(note, 0.8);
            }
            let block = Instant::now();
            for _ in 0..256 {
                engine.tick();
            }
            engine.publish(&t, 256, block.elapsed());
        }
        let load = start.elapsed().as_secs_f32() / seconds;
        println!("engine: {:.2}% of real time at 48 kHz", 100.0 * load);
    }
}
