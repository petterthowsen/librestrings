//! The solo cello as a CLAP plugin (PLAN.md Phase 3).
//!
//! The plugin is a thin layer over [`Performer`]: MIDI notes, CC11 (dynamics),
//! CC1 (vibrato) and keyswitches in; the performer's mono output on every
//! output channel.
//! The editor shows the performer's state and has an on-screen keyboard and
//! faders, so it can be played without a MIDI controller.
//!
//! Real-time rules as in `strings-dsp`: the performer is built in
//! `initialize` (about 30 ms), never in `process`, which doesn't allocate, lock
//! or wait (checked in debug builds by nih-plug's `assert_process_allocs`).
//! The tuning window's changes arrive the same way as its notes: through
//! lock-free queues, with anything slow done on the editor's side.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use nih_plug::prelude::*;
#[cfg(test)]
use strings_dsp::BowLift;
use strings_dsp::presets::cello;
use strings_dsp::{InstrumentSpec, Performer, PerformerSettings};

mod editor;
pub mod params;
pub mod shared;
pub mod tuning;

use params::{BowLiftParam, FingeringParam, PolyphonyParam, StringsParams};
use shared::{GuiEvent, Shared, Telemetry};
use tuning::LiveTuning;

/// Numbers each engine, so the editor notices a new one.
static ENGINES: AtomicU32 = AtomicU32::new(0);

/// The only instrument so far.
pub const INSTRUMENT: &InstrumentSpec = &cello::INSTRUMENT;

/// Default controller numbers (PLAN.md 4.1), as in SWAM: the expression
/// pedal plays the dynamics and the mod wheel the vibrato.
const CC_VIBRATO: u8 = 1;
const CC_DYNAMICS: u8 = 11;
const CC_ALL_SOUND_OFF: u8 = 120;
const CC_ALL_NOTES_OFF: u8 = 123;

/// MIDI note of a frequency, rounded.
pub fn midi_note(frequency: f32) -> u8 {
    (69.0 + 12.0 * (frequency / 440.0).log2()).round() as u8
}

/// The first keyswitch: the first C below the instrument's lowest note
/// (cello: C1). The white keys from there set the bow lift.
pub fn keyswitch_base(spec: &InstrumentSpec) -> u8 {
    let lowest = midi_note(spec.strings[0].frequency);
    (lowest - 1) / 12 * 12
}

/// The bow lift a keyswitch note selects, if it is one.
pub fn keyswitch(spec: &InstrumentSpec, note: u8) -> Option<BowLiftParam> {
    let base = keyswitch_base(spec);
    match note.checked_sub(base)? {
        0 => Some(BowLiftParam::OffString),
        2 => Some(BowLiftParam::OnString),
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

/// The continuous controls, in order: dynamics, vibrato, pressure.
const CONTROLS: usize = 3;

/// Parameter values last passed to the performer, to detect changes.
#[derive(Clone, Copy)]
struct Applied {
    controls: [f32; CONTROLS],
    bow_lift: Option<BowLiftParam>,
    polyphony: Option<PolyphonyParam>,
    fingering: Option<FingeringParam>,
}

impl Applied {
    const NONE: Self = Self {
        controls: [f32::NAN; CONTROLS],
        bow_lift: None,
        polyphony: None,
        fingering: None,
    };
}

/// The performer and what the plugin measures around it.
struct Engine {
    performer: Performer,
    sample_rate: f32,
    id: u32,
    strings_generation: u32,
    applied: Applied,
    /// The controls as the performer has them: dynamics, vibrato, pressure.
    controls: [f32; CONTROLS],

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
            id: ENGINES.fetch_add(1, Relaxed) + 1,
            strings_generation: 0,
            applied: Applied::NONE,
            controls: [0.5, 0.0, 0.5],
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
            params.vibrato.value(),
            params.pressure.value(),
        ];
        for (i, value) in values.into_iter().enumerate() {
            if value != self.applied.controls[i] {
                self.applied.controls[i] = value;
                self.set_control(i, value);
            }
        }
        let bow_lift = params.bow_lift.value();
        if self.applied.bow_lift != Some(bow_lift) {
            self.applied.bow_lift = Some(bow_lift);
            self.performer.set_bow_lift(bow_lift.into());
        }
        // Only the parameters set these.
        let polyphony = params.polyphony.value();
        if self.applied.polyphony != Some(polyphony) {
            self.applied.polyphony = Some(polyphony);
            self.performer.set_polyphony(polyphony.into());
        }
        let fingering = params.fingering.value();
        if self.applied.fingering != Some(fingering) {
            self.applied.fingering = Some(fingering);
            self.performer.set_fingering(fingering.into());
        }
    }

    /// Takes the tuning window's changes (real-time safe).
    fn apply_tuning(&mut self, shared: &Shared) {
        let mut live = None;
        while let Some(l) = shared.live_tuning.pop() {
            live = Some(l);
        }
        if let Some(live) = live {
            self.set_live_tuning(&live);
        }
        while let Some(mut update) = shared.string_updates.pop() {
            self.performer
                .instrument_mut()
                .apply_strings(&update.specs, &mut update.designs);
            self.strings_generation = update.generation;
            // The old filters are freed by the editor. If it isn't draining the
            // queue, leak them rather than free them here.
            if let Err(update) = shared.string_returns.push(update) {
                std::mem::forget(update);
            }
        }
    }

    fn set_live_tuning(&mut self, live: &LiveTuning) {
        self.performer.set_settings(live.performer);
        let instrument = self.performer.instrument_mut();
        instrument.set_friction(live.friction);
        instrument.set_hair(live.hair);
        instrument.set_body(&live.body);
    }

    fn set_control(&mut self, i: usize, value: f32) {
        self.controls[i] = value;
        match i {
            0 => self.performer.set_dynamics(value),
            1 => self.performer.set_vibrato(value),
            _ => self.performer.set_pressure(value),
        }
    }

    fn note_on(&mut self, note: u8, velocity: f32) {
        match keyswitch(INSTRUMENT, note) {
            Some(bow_lift) => self.performer.set_bow_lift(bow_lift.into()),
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
            GuiEvent::BowLift(b) => self.performer.set_bow_lift(b.into()),
        }
    }

    fn midi_event(&mut self, event: NoteEvent<()>) {
        match event {
            NoteEvent::NoteOn { note, velocity, .. } => self.note_on(note, velocity),
            NoteEvent::NoteOff { note, .. } => self.note_off(note),
            NoteEvent::MidiCC { cc, value, .. } => match cc {
                CC_DYNAMICS => self.set_control(0, value),
                CC_VIBRATO => self.set_control(1, value),
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
        t.engine.store(self.id, Relaxed);
        t.strings_generation.store(self.strings_generation, Relaxed);
        t.sample_rate.store(self.sample_rate, Relaxed);
        t.block_size.store(samples as u32, Relaxed);
        t.load.store(self.load, Relaxed);
        t.load_peak.store(self.load_peak, Relaxed);
        t.output_peak.store(self.output_peak, Relaxed);
        t.resets.store(self.resets, Relaxed);
        t.note.store(p.note().map_or(-1, i32::from), Relaxed);
        t.string.store(bowed as u32, Relaxed);
        t.second_note
            .store(p.second().map_or(-1, |(n, _)| i32::from(n)), Relaxed);
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
        let controls = [&t.dynamics, &t.vibrato, &t.pressure];
        for (atomic, value) in controls.into_iter().zip(self.controls) {
            atomic.store(value, Relaxed);
        }
        let bow_lift = BowLiftParam::from(p.bow_lift());
        t.bow_lift.store(bow_lift as u32, Relaxed);

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
    const NAME: &'static str = "LibreStrings";
    const VENDOR: &'static str = "LibreStrings";
    const URL: &'static str = "https://github.com/petterthowsen/librestrings";
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
        engine.apply_tuning(&self.shared);
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
    // Hosts save projects against this ID: never change it. One ID for the
    // whole plugin; the instrument is a choice inside it, not part of the ID.
    const CLAP_ID: &'static str = "io.github.petterthowsen.librestrings";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Physically modeled bowed cello");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> =
        Some("https://github.com/petterthowsen/librestrings/issues");
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
        assert_eq!(keyswitch(INSTRUMENT, 24), Some(BowLiftParam::OffString));
        assert_eq!(keyswitch(INSTRUMENT, 26), Some(BowLiftParam::OnString));
        assert_eq!(keyswitch(INSTRUMENT, 28), None);
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
    fn keyswitch_changes_the_bow_lift_without_playing() {
        let params = StringsParams::default();
        let t = Telemetry::default();
        let mut engine = Engine::new(FS);
        engine.apply_params(&params);
        engine.midi_event(note_on(26));
        run(&mut engine, &t, 0.05);
        assert_eq!(t.note(), None);
        assert_eq!(t.bow_lift(), BowLiftParam::OnString);
        // The unchanged parameter doesn't take it back.
        engine.apply_params(&params);
        assert_eq!(engine.performer.bow_lift(), BowLift::OnString);
    }

    /// The tuning window's changes reach the performer; refitted strings are
    /// swapped in and their old filters come back to be freed.
    #[test]
    fn tuning_reaches_the_engine() {
        let params = StringsParams::default();
        let shared = Shared::default();
        let mut engine = Engine::new(FS);
        engine.apply_params(&params);
        engine.set_control(2, 0.4);

        let mut tuning = tuning::Tuning::new(INSTRUMENT);
        tuning.live.performer.tuning.attack_bite = 0.33;
        tuning.live.performer.pressure = 0.7;
        tuning.live.friction.mu_s = 0.9;
        assert!(shared.live_tuning.push(tuning.live).is_ok());
        tuning.strings.damping.floor = 2e-3;
        let specs = tuning.strings.apply_to(&INSTRUMENT.strings);
        let update = tuning::StringsUpdate {
            generation: 7,
            specs,
            designs: strings_dsp::Instrument::design_strings(&specs, FS),
        };
        assert!(shared.string_updates.push(Box::new(update)).is_ok());

        engine.apply_tuning(&shared);
        engine.publish(&shared.telemetry, 256, Duration::ZERO);
        let settings = engine.performer.settings();
        assert_eq!(settings.tuning.attack_bite, 0.33);
        // The band position of normal pressure is the tuning's; the control
        // stays the parameter's.
        assert_eq!(settings.pressure, 0.7);
        assert_eq!(engine.performer.pressure(), 0.4);
        let instrument = engine.performer.instrument();
        assert_eq!(instrument.spec().friction.mu_s, 0.9);
        assert_eq!(instrument.spec().strings[2].loss, specs[2].loss);
        assert_eq!(shared.telemetry.strings_generation.load(Relaxed), 7);
        assert_eq!(shared.string_returns.len(), 1);
    }

    /// As in SWAM: the expression pedal plays the dynamics, the mod wheel the
    /// vibrato. There is no separate expression gain.
    #[test]
    fn expression_pedal_plays_dynamics_and_mod_wheel_vibrato() {
        let params = StringsParams::default();
        let t = Telemetry::default();
        let mut engine = Engine::new(FS);
        engine.apply_params(&params);
        engine.midi_event(cc(11, 0.8));
        engine.midi_event(cc(1, 0.3));
        engine.midi_event(cc(21, 0.9));
        engine.publish(&t, 256, Duration::ZERO);
        assert_eq!(engine.controls, [0.8, 0.3, 0.5]);
        assert_eq!(t.dynamics.load(Relaxed), 0.8);
        assert_eq!(t.vibrato.load(Relaxed), 0.3);
    }

    #[test]
    fn a_double_stop_is_published() {
        let params = StringsParams::default();
        let t = Telemetry::default();
        let mut engine = Engine::new(FS);
        engine.apply_params(&params);
        engine
            .performer
            .set_polyphony(strings_dsp::Polyphony::DoubleStops);
        engine.midi_event(note_on(50));
        engine.midi_event(note_on(57));
        run(&mut engine, &t, 0.3);
        assert_eq!((t.note(), t.second_note()), (Some(50), Some(57)));
        assert!(t.strings[2].contact.load(Relaxed) > 0.99);
        assert!(t.strings[3].contact.load(Relaxed) > 0.99);
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
        engine.set_control(1, 0.6);
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
