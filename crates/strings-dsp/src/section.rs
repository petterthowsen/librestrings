//! A section: up to [`MAX_PLAYERS`] players of one instrument playing the
//! same part (PLAN.md §5, docs/SECTIONS.md A2).
//!
//! Every player is a full [`Performer`], cloned from the first so the strings
//! are fitted once. Each player differs from the section's settings by its
//! own seeded draws ([`Humanization`]): tuning, how late it comes in,
//! vibrato, dynamics, bow position and pressure, timing, the body, and the
//! bow's wander. Player 0 has none of it, so a section of one is exactly the
//! solo performer.
//!
//! All players are built up front; [`Section::set_players`] only switches
//! them on and off, fading over [`FADE`]. A player switched on comes in with
//! the next note. Real-time safe after construction: notes wait for their
//! player in a fixed-size queue.

use crate::body::BodyTuning;
use crate::instrument::InstrumentSpec;
use crate::performer::{
    BowLift, Fingering, MAX_DETUNE, Performer, PerformerSettings, PerformerTuning, Polyphony,
};

/// Most players in a section.
pub const MAX_PLAYERS: usize = 12;

/// String oversampling of every player but the first, which plays at the
/// settings' own (2× by default, so a section of one is the solo cello). At
/// 1× high notes step by 5–20 cents alone (PLAN.md), but in a section of 8
/// the difference wasn't audible (docs/SECTIONS.md A1), and it halves the
/// cost.
pub const PLAYER_OVERSAMPLING: usize = 1;

/// Players switched on or off fade in or out over this long (s).
pub const FADE: f32 = 0.05;

/// Notes a player can have waiting for its delay.
const QUEUE: usize = 64;

/// Detune drift is updated every this many samples.
const DRIFT_INTERVAL: usize = 64;

/// How far the players spread around the section's settings. Each player
/// draws a value in ±1 per quantity from its seed, scaled by these; changing
/// them keeps the draws. Zero everywhere makes every player play alike.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Humanization {
    /// Each player's own tuning (cents, either way), and a slow drift around
    /// it (cents) with segments about `detune_time` (s) long.
    pub detune: f32,
    pub detune_drift: f32,
    pub detune_time: f32,
    /// A player comes in up to this late (s), and each note up to `jitter`
    /// either way around that, never early.
    pub delay: f32,
    pub jitter: f32,
    /// Vibrato rate and depth (fractions, either way).
    pub vibrato_rate: f32,
    pub vibrato_depth: f32,
    /// Dynamics (0–1 control, either way).
    pub dynamics: f32,
    /// Bow position (fraction). Only toward the bridge: the performer's β
    /// range is already at the edge of the flat zone (STATUS.md item 8).
    pub beta: f32,
    /// Pressure (band position, either way).
    pub pressure: f32,
    /// Attack, legato and portamento times (fraction, either way).
    pub timing: f32,
    /// Body: frequencies of the listed modes and their damping (fractions,
    /// either way). The dense modes get a seed of their own.
    pub body_frequency: f32,
    pub body_damping: f32,
}

impl Default for Humanization {
    /// First guesses, to be judged by ear.
    fn default() -> Self {
        Self {
            detune: 5.0,
            detune_drift: 2.0,
            detune_time: 3.0,
            delay: 0.025,
            jitter: 0.008,
            vibrato_rate: 0.1,
            vibrato_depth: 0.25,
            dynamics: 0.05,
            beta: 0.12,
            pressure: 0.08,
            timing: 0.2,
            body_frequency: 0.03,
            body_damping: 0.15,
        }
    }
}

impl Humanization {
    /// Every player alike.
    pub const NONE: Self = Self {
        detune: 0.0,
        detune_drift: 0.0,
        detune_time: 3.0,
        delay: 0.0,
        jitter: 0.0,
        vibrato_rate: 0.0,
        vibrato_depth: 0.0,
        dynamics: 0.0,
        beta: 0.0,
        pressure: 0.0,
        timing: 0.0,
        body_frequency: 0.0,
        body_damping: 0.0,
    };
}

/// A player's draws, each in ±1 (`delay` in 0–1).
#[derive(Clone, Copy, Debug, Default)]
struct Draws {
    detune: f32,
    delay: f32,
    vibrato_rate: f32,
    vibrato_depth: f32,
    dynamics: f32,
    beta: f32,
    pressure: f32,
    timing: f32,
}

#[derive(Clone, Copy, Debug)]
enum Event {
    NoteOn(u8, f32),
    NoteOff(u8),
}

/// Notes waiting for their time (in samples), oldest first.
#[derive(Clone, Copy)]
struct Queue {
    events: [(u64, Event); QUEUE],
    start: usize,
    len: usize,
}

impl Queue {
    const EMPTY: Self = Self {
        events: [(0, Event::NoteOff(0)); QUEUE],
        start: 0,
        len: 0,
    };

    /// `false` if full.
    fn push(&mut self, time: u64, event: Event) -> bool {
        if self.len == QUEUE {
            return false;
        }
        self.events[(self.start + self.len) % QUEUE] = (time, event);
        self.len += 1;
        true
    }

    fn pop_due(&mut self, now: u64) -> Option<Event> {
        let (time, event) = self.events[self.start];
        if self.len == 0 || time > now {
            return None;
        }
        self.start = (self.start + 1) % QUEUE;
        self.len -= 1;
        Some(event)
    }

    /// Time of the newest event waiting.
    fn last(&self) -> Option<u64> {
        (self.len > 0).then(|| self.events[(self.start + self.len - 1) % QUEUE].0)
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

struct Player {
    performer: Performer,
    draws: Draws,
    /// Fixed per player (the body's variations); `rng` runs on (jitter, drift).
    seed: u32,
    rng: u32,
    queue: Queue,
    /// Switched on by the section's size.
    active: bool,
    /// Off and faded out: not processed until switched on again.
    asleep: bool,
    gain: f32,
    /// Detune drift in ±1: raised-cosine segments between random points.
    drift_from: f32,
    drift_to: f32,
    drift_pos: f32,
    drift_length: f32,
}

pub struct Section {
    players: Vec<Player>,
    count: usize,
    settings: PerformerSettings,
    body: BodyTuning,
    humanization: Humanization,
    fs: f32,
    /// Samples since construction.
    clock: u64,
    /// Gain per active player: power stays about the same at every size.
    player_gain: f32,
    fade_step: f32,
    drift_countdown: usize,
    /// The section's controls, before each player's offset.
    dynamics: f32,
}

impl Section {
    /// Slow (builds one or two instruments, about 30 ms each); don't call it
    /// on the audio thread. Starts with one player.
    pub fn new(
        spec: &InstrumentSpec,
        settings: PerformerSettings,
        humanization: Humanization,
        sample_rate: f32,
    ) -> Self {
        let first = Performer::new(spec, settings, sample_rate);
        let others = if settings.oversampling == PLAYER_OVERSAMPLING {
            first.clone()
        } else {
            let settings = PerformerSettings {
                oversampling: PLAYER_OVERSAMPLING,
                ..settings
            };
            Performer::new(spec, settings, sample_rate)
        };
        let players = (0..MAX_PLAYERS)
            .map(|i| {
                let seed = player_seed(settings.seed, i);
                let mut rng = seed;
                let draws = if i == 0 {
                    Draws::default()
                } else {
                    Draws {
                        detune: signed(&mut rng),
                        delay: next_random(&mut rng),
                        vibrato_rate: signed(&mut rng),
                        vibrato_depth: signed(&mut rng),
                        dynamics: signed(&mut rng),
                        beta: next_random(&mut rng),
                        pressure: signed(&mut rng),
                        timing: signed(&mut rng),
                    }
                };
                let performer = if i == 0 {
                    first.clone()
                } else {
                    let mut p = others.clone();
                    p.reseed(rng ^ 0x9e37_79b9);
                    p
                };
                Player {
                    performer,
                    draws,
                    seed,
                    rng,
                    queue: Queue::EMPTY,
                    active: i == 0,
                    asleep: i != 0,
                    gain: if i == 0 { 1.0 } else { 0.0 },
                    drift_from: 0.0,
                    drift_to: 0.0,
                    drift_pos: 1.0,
                    drift_length: 1.0,
                }
            })
            .collect();
        let mut section = Self {
            players,
            count: 1,
            settings,
            body: BodyTuning::from(&spec.body),
            humanization,
            fs: sample_rate,
            clock: 0,
            player_gain: 1.0,
            fade_step: 1.0 / (FADE * sample_rate),
            drift_countdown: 0,
            dynamics: 0.5,
        };
        section.set_humanization(humanization);
        section
    }

    /// Active players.
    pub fn players(&self) -> usize {
        self.count
    }

    /// Switches players on or off (1–[`MAX_PLAYERS`]). Real-time safe. Players
    /// switched off finish their note as if it were let go and fade out;
    /// players switched on fade in and come in with the next note.
    pub fn set_players(&mut self, count: usize) {
        let count = count.clamp(1, MAX_PLAYERS);
        self.count = count;
        self.player_gain = 1.0 / (count as f32).sqrt();
        for (i, p) in self.players.iter_mut().enumerate() {
            let active = i < count;
            if active && !p.active && p.asleep {
                p.asleep = false;
            } else if !active && p.active {
                p.queue.clear();
                p.performer.release_all();
            }
            p.active = active;
        }
    }

    /// Player `i` (0 is the one without humanization), for telemetry and the
    /// instrument's real-time setters.
    pub fn player(&self, i: usize) -> &Performer {
        &self.players[i].performer
    }

    pub fn player_mut(&mut self, i: usize) -> &mut Performer {
        &mut self.players[i].performer
    }

    pub fn humanization(&self) -> &Humanization {
        &self.humanization
    }

    /// Changes the spread while playing; each player keeps its draws.
    pub fn set_humanization(&mut self, h: Humanization) {
        self.humanization = h;
        self.set_settings(self.settings);
        let body = self.body;
        self.set_body(&body);
        self.set_dynamics(self.dynamics);
    }

    pub fn settings(&self) -> &PerformerSettings {
        &self.settings
    }

    /// The section's settings; each player plays them with its own offsets.
    pub fn set_settings(&mut self, settings: PerformerSettings) {
        self.settings = settings;
        let h = self.humanization;
        for p in &mut self.players {
            p.performer
                .set_settings(vary_settings(&settings, &p.draws, &h));
        }
    }

    /// Retunes every player's body: the tuning, varied per player (player 0
    /// plays it as it is). Real-time safe.
    pub fn set_body(&mut self, body: &BodyTuning) {
        self.body = *body;
        let h = self.humanization;
        for (i, p) in self.players.iter_mut().enumerate() {
            let tuning = if i == 0 {
                *body
            } else {
                vary_body(body, p.seed, &h)
            };
            p.performer.instrument_mut().set_body(&tuning);
        }
    }

    pub fn set_dynamics(&mut self, value: f32) {
        self.dynamics = value;
        let spread = self.humanization.dynamics;
        for p in &mut self.players {
            p.performer.set_dynamics(value + spread * p.draws.dynamics);
        }
    }

    pub fn set_vibrato(&mut self, value: f32) {
        self.each(|p| p.set_vibrato(value));
    }

    pub fn set_pressure(&mut self, value: f32) {
        self.each(|p| p.set_pressure(value));
    }

    pub fn set_bow_lift(&mut self, bow_lift: BowLift) {
        self.each(|p| p.set_bow_lift(bow_lift));
    }

    pub fn set_polyphony(&mut self, polyphony: Polyphony) {
        self.each(|p| p.set_polyphony(polyphony));
    }

    pub fn set_fingering(&mut self, fingering: Fingering) {
        self.each(|p| p.set_fingering(fingering));
    }

    pub fn set_string(&mut self, string: Option<usize>) {
        self.each(|p| p.set_string(string));
    }

    fn each(&mut self, mut f: impl FnMut(&mut Performer)) {
        for p in &mut self.players {
            f(&mut p.performer);
        }
    }

    /// `velocity` is the MIDI velocity scaled to 0–1. Each active player
    /// plays it after its own delay.
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        self.schedule(Event::NoteOn(note, velocity));
    }

    pub fn note_off(&mut self, note: u8) {
        self.schedule(Event::NoteOff(note));
    }

    fn schedule(&mut self, event: Event) {
        let h = self.humanization;
        let fs = self.fs;
        for (i, p) in self.players.iter_mut().enumerate() {
            if !p.active {
                continue;
            }
            let delay = if i == 0 {
                0.0
            } else {
                (h.delay * p.draws.delay + h.jitter * signed(&mut p.rng)).max(0.0)
            };
            // Never before a note already waiting: the order of a legato
            // line's notes is what makes it legato.
            let time = (self.clock + (delay * fs) as u64).max(p.queue.last().unwrap_or(0));
            if time <= self.clock && p.queue.len == 0 {
                play(&mut p.performer, event);
            } else if !p.queue.push(time, event) {
                // Full (a flood of notes): play it now.
                play(&mut p.performer, event);
            }
        }
    }

    /// Releases every held note, as MIDI "all notes off"; notes still
    /// waiting are dropped.
    pub fn release_all(&mut self) {
        for p in &mut self.players {
            p.queue.clear();
            p.performer.release_all();
        }
    }

    /// Silences everything at once.
    pub fn reset(&mut self) {
        for p in &mut self.players {
            p.queue.clear();
            p.performer.reset();
            p.gain = if p.active { self.player_gain } else { 0.0 };
            p.asleep = !p.active;
        }
    }

    /// Advances one sample: every player's output (already scaled, 0 for a
    /// player that is off) in `out`, and their sum.
    pub fn process(&mut self, out: &mut [f32; MAX_PLAYERS]) -> f32 {
        let drift = self.drift_countdown == 0;
        if drift {
            self.drift_countdown = DRIFT_INTERVAL;
        }
        self.drift_countdown -= 1;
        let h = self.humanization;
        let dt = DRIFT_INTERVAL as f32 / self.fs;
        let mut sum = 0.0;
        for (i, (p, y)) in self.players.iter_mut().zip(out.iter_mut()).enumerate() {
            if p.asleep {
                *y = 0.0;
                continue;
            }
            while let Some(event) = p.queue.pop_due(self.clock) {
                play(&mut p.performer, event);
            }
            if drift && i > 0 {
                p.drift(dt, h.detune_time);
                let cents = h.detune * p.draws.detune + h.detune_drift * p.drift_value();
                p.performer.set_detune(cents.clamp(-MAX_DETUNE, MAX_DETUNE));
            }
            let target = if p.active { self.player_gain } else { 0.0 };
            p.gain = if p.gain < target {
                (p.gain + self.fade_step).min(target)
            } else {
                (p.gain - self.fade_step).max(target)
            };
            *y = p.gain * p.performer.process();
            sum += *y;
            if !p.active && p.gain == 0.0 {
                p.performer.reset();
                p.asleep = true;
            }
        }
        self.clock += 1;
        sum
    }
}

impl Player {
    fn drift(&mut self, dt: f32, mean: f32) {
        self.drift_pos += dt / self.drift_length;
        if self.drift_pos >= 1.0 {
            self.drift_from = self.drift_to;
            self.drift_to = signed(&mut self.rng);
            self.drift_length = (mean * (0.6 + 0.8 * next_random(&mut self.rng))).max(1e-3);
            self.drift_pos = 0.0;
        }
    }

    fn drift_value(&self) -> f32 {
        let s = 0.5 - 0.5 * (std::f32::consts::PI * self.drift_pos.min(1.0)).cos();
        self.drift_from + (self.drift_to - self.drift_from) * s
    }
}

fn play(performer: &mut Performer, event: Event) {
    match event {
        Event::NoteOn(note, velocity) => performer.note_on(note, velocity),
        Event::NoteOff(note) => performer.note_off(note),
    }
}

/// The section's settings as one player plays them.
fn vary_settings(s: &PerformerSettings, d: &Draws, h: &Humanization) -> PerformerSettings {
    let time = 1.0 + h.timing * d.timing;
    let t = s.tuning;
    let beta = 1.0 - h.beta * d.beta;
    PerformerSettings {
        beta: (s.beta.0 * beta, s.beta.1 * beta),
        pressure: s.pressure + h.pressure * d.pressure,
        vibrato_rate: s.vibrato_rate * (1.0 + h.vibrato_rate * d.vibrato_rate),
        vibrato_depth: s.vibrato_depth * (1.0 + h.vibrato_depth * d.vibrato_depth),
        tuning: PerformerTuning {
            attack: (t.attack.0 * time, t.attack.1 * time),
            grip_attack: (t.grip_attack.0 * time, t.grip_attack.1 * time),
            place: t.place * time,
            shift: t.shift * time,
            portamento: t.portamento * time,
            vibrato_delay: t.vibrato_delay * time,
            ..t
        },
        ..*s
    }
}

/// The body as one player's instrument has it: the listed modes moved and
/// damped a little, and the dense modes drawn from another seed.
fn vary_body(body: &BodyTuning, seed: u32, h: &Humanization) -> BodyTuning {
    let mut rng = seed ^ 0x5bd1_e995;
    let mut b = *body;
    for m in &mut b.modes[..b.mode_count] {
        m.frequency *= 1.0 + h.body_frequency * signed(&mut rng);
        m.damping *= 1.0 + h.body_damping * signed(&mut rng);
    }
    b.dense.seed = (body.dense.seed ^ seed).max(1);
    b
}

/// A player's seed from the section's seed and its index.
fn player_seed(seed: u32, i: usize) -> u32 {
    // Murmur3's finalizer, so neighbouring seeds draw unrelated values.
    let mut x = seed ^ (i as u32).wrapping_mul(0x9e37_79b9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85eb_ca6b);
    x ^= x >> 13;
    x = x.wrapping_mul(0xc2b2_ae35);
    x ^= x >> 16;
    x.max(1)
}

/// Uniform in [0, 1) (xorshift32).
fn next_random(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 8) as f32 / (1u32 << 24) as f32
}

/// Uniform in [−1, 1).
fn signed(state: &mut u32) -> f32 {
    2.0 * next_random(state) - 1.0
}
