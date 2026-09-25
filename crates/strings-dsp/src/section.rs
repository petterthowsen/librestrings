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
//!
//! With [`Polyphony::Divisi`] the notes of a chord are divided among the
//! players, one note each, instead of every player playing them all (see
//! [`Section::flush`]).

use crate::body::BodyTuning;
use crate::instrument::InstrumentSpec;
use crate::performer::{
    BowLift, Fingering, MAX_DETUNE, Performer, PerformerFrame, PerformerSettings, PerformerTuning,
    Polyphony,
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

/// Most notes in a chord (divisi divides them among the players, so a chord
/// needs no more than [`MAX_PLAYERS`]).
const MAX_CHORD: usize = MAX_PLAYERS;

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
    /// Tuned by ear in Bitwig (September 2026), from first guesses.
    fn default() -> Self {
        Self {
            detune: 0.0,
            detune_drift: 7.0,
            detune_time: 2.0,
            delay: 0.01,
            jitter: 0.008,
            vibrato_rate: 0.17,
            vibrato_depth: 0.25,
            dynamics: 0.2,
            beta: 0.3,
            pressure: 0.3,
            timing: 0.1,
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
    Sustain(bool),
    /// A bow keyswitch: direction and velocity.
    Bow(f32, f32),
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

    /// Drops the events `f` matches, keeping the rest in their order.
    fn remove(&mut self, mut f: impl FnMut(&Event) -> bool) {
        let mut kept = 0;
        for k in 0..self.len {
            let (time, event) = self.events[(self.start + k) % QUEUE];
            if !f(&event) {
                self.events[(self.start + kept) % QUEUE] = (time, event);
                kept += 1;
            }
        }
        self.len = kept;
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

/// The notes the section plays as one chord (divisi), in the order they came.
#[derive(Clone, Copy)]
struct Chord {
    notes: [u8; MAX_CHORD],
    len: usize,
}

impl Chord {
    const EMPTY: Self = Self {
        notes: [0; MAX_CHORD],
        len: 0,
    };

    fn position(&self, note: u8) -> Option<usize> {
        self.notes[..self.len].iter().position(|&n| n == note)
    }

    /// `false` if the chord is full.
    fn add(&mut self, note: u8) -> bool {
        if self.len == MAX_CHORD {
            return false;
        }
        self.notes[self.len] = note;
        self.len += 1;
        true
    }

    fn remove(&mut self, note: u8) {
        if let Some(i) = self.position(note) {
            self.notes.copy_within(i + 1..self.len, i);
            self.len -= 1;
        }
    }

    fn clear(&mut self) {
        self.len = 0;
    }
}

/// The notes that came in while the sample was being handled, waiting for the
/// rest of the chord to arrive so it can be divided once (divisi).
#[derive(Clone, Copy)]
struct Pending {
    notes: [u8; MAX_CHORD],
    velocities: [f32; MAX_CHORD],
    len: usize,
}

impl Pending {
    const EMPTY: Self = Self {
        notes: [0; MAX_CHORD],
        velocities: [0.0; MAX_CHORD],
        len: 0,
    };

    /// `false` if full.
    fn push(&mut self, note: u8, velocity: f32) -> bool {
        if self.len == MAX_CHORD {
            return false;
        }
        self.notes[self.len] = note;
        self.velocities[self.len] = velocity;
        self.len += 1;
        true
    }

    fn contains(&self, note: u8) -> bool {
        self.notes[..self.len].contains(&note)
    }

    /// The velocity this note came in with, 0 if it is not here.
    fn velocity(&self, note: u8) -> f32 {
        self.notes[..self.len]
            .iter()
            .rposition(|&n| n == note)
            .map_or(0.0, |i| self.velocities[i])
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
    /// The chord note this player is divided over (divisi; `None` when the
    /// players play the whole chord, or when it plays nothing).
    voice: Option<u8>,
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
    /// Divisi: the notes of a chord are divided among the players
    /// ([`Polyphony::Divisi`]).
    divisi: bool,
    /// The players are seated on the chord's notes, one each (a section of
    /// more than one player): evenly divided ([`Section::divide`]) when there
    /// are no more notes than players, and with the new notes taking the
    /// closest-playing player ([`Section::steal`]) when there are.
    dividing: bool,
    chord: Chord,
    pending: Pending,
    fs: f32,
    /// Samples since construction.
    clock: u64,
    /// Gain per active player: power stays about the same at every size.
    player_gain: f32,
    fade_step: f32,
    drift_countdown: usize,
    /// The section's controls, before each player's offset.
    dynamics: f32,
    /// Player 0's last frame, for telemetry.
    frame: PerformerFrame,
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
                    voice: None,
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
            divisi: false,
            dividing: false,
            chord: Chord::EMPTY,
            pending: Pending::EMPTY,
            fs: sample_rate,
            clock: 0,
            player_gain: 1.0,
            fade_step: 1.0 / (FADE * sample_rate),
            drift_countdown: 0,
            dynamics: 0.5,
            frame: PerformerFrame::default(),
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
            if active != p.active {
                // A player switched on comes in with the next note, not into
                // the middle of the chord the others are divided over.
                p.voice = None;
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

    /// Player 0's frame from the last sample (unscaled by the section's gain).
    pub fn frame(&self) -> &PerformerFrame {
        &self.frame
    }

    /// The notes the section is sounding, one bit per MIDI note in `mask`
    /// (`mask[note / 64]`, bit `note % 64`): every player's note, and a double
    /// stop's second. A player waiting out its delay has not started, and one
    /// that faded out sounds nothing.
    pub fn sounding_notes(&self, mask: &mut [u64; 2]) {
        for p in &self.players {
            if p.asleep {
                continue;
            }
            let notes = [p.performer.note(), p.performer.second().map(|(n, _)| n)];
            for note in notes.into_iter().flatten() {
                mask[note as usize / 64] |= 1 << (note % 64);
            }
        }
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

    /// Takes effect from the next note. With [`Polyphony::Divisi`] the notes
    /// of a chord are divided among the players ([`Section::flush`]); the
    /// notes already sounding stay where they are.
    pub fn set_polyphony(&mut self, polyphony: Polyphony) {
        let divisi = polyphony == Polyphony::Divisi;
        if divisi != self.divisi {
            self.divisi = divisi;
            self.forget_chord();
        }
        // A player of a divisi section still plays the two notes it must when
        // the section can't divide (one player, or more notes than players).
        let played = if divisi {
            Polyphony::DoubleStops
        } else {
            polyphony
        };
        self.each(|p| p.set_polyphony(played));
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
    /// plays it after its own delay; with [`Polyphony::Divisi`] the notes of
    /// a chord wait for the sample they came in on and are then divided
    /// among the players ([`Section::flush`]).
    pub fn note_on(&mut self, note: u8, velocity: f32) {
        if !self.divisi {
            self.schedule(Event::NoteOn(note, velocity));
            return;
        }
        if !self.pending.push(note, velocity) {
            // More notes at once than a chord holds: divide the ones so far.
            self.flush();
            self.pending.push(note, velocity);
        }
    }

    pub fn note_off(&mut self, note: u8) {
        let note_off = Event::NoteOff(note);
        if !self.divisi {
            self.schedule(note_off);
            return;
        }
        // The notes of the chord come first, whatever order they came in.
        self.flush();
        self.chord.remove(note);
        if !self.dividing {
            self.schedule(note_off);
            return;
        }
        // Only the players divided over this note play it.
        let h = self.humanization;
        let fs = self.fs;
        let clock = self.clock;
        for (i, p) in self.players.iter_mut().enumerate() {
            if p.voice == Some(note) {
                p.voice = None;
                queue(clock, fs, &h, i, p, note_off);
            }
        }
    }

    /// The sustain pedal ([`Performer::set_sustain`]), in order with the
    /// notes around it.
    pub fn set_sustain(&mut self, on: bool) {
        self.flush();
        self.schedule(Event::Sustain(on));
    }

    /// A bow keyswitch ([`Performer::set_bow_direction`]), in order with the
    /// notes around it.
    pub fn set_bow_direction(&mut self, direction: f32, velocity: f32) {
        self.flush();
        self.schedule(Event::Bow(direction, velocity));
    }

    fn schedule(&mut self, event: Event) {
        let h = self.humanization;
        let fs = self.fs;
        let clock = self.clock;
        for (i, p) in self.players.iter_mut().enumerate() {
            queue(clock, fs, &h, i, p, event);
        }
    }

    /// Divides the notes that came in while the sample was being handled
    /// (PLAN.md §5 divisi): the players are seated on the chord's notes, one
    /// note each, spread as evenly over the section as its desks are. A
    /// player keeps the note it is playing while the chord holds it, so only
    /// the players a note that just came in needs move: they end the note
    /// they were playing and start the new one. A player whose note is let go
    /// rests until the next chord (a player never joins a note already
    /// sounding). With one player there is nobody to divide and every player
    /// plays the whole chord, as it does without divisi; with more notes than
    /// players every player plays one note, the new notes taking the player
    /// whose note is closest (see [`Section::steal`]).
    fn flush(&mut self) {
        if self.pending.len == 0 {
            return;
        }
        let h = self.humanization;
        let fs = self.fs;
        let clock = self.clock;
        let pending = self.pending;
        self.pending.clear();
        for &note in &pending.notes[..pending.len] {
            self.chord.add(note);
        }
        let active = self.players.iter().filter(|p| p.active).count();
        if active > 1 && self.chord.len <= active {
            let target = self.divide(&pending);
            self.seat(&target, &pending);
        } else if active > 1 {
            let target = self.steal(&pending);
            self.seat(&target, &pending);
        } else {
            // Nobody to divide: play the chord as a section without divisi
            // does, every player taking what it can of it.
            if self.dividing {
                for (i, p) in self.players.iter_mut().enumerate() {
                    if let Some(old) = p.voice.take() {
                        cancel(&mut p.queue, old);
                        queue(clock, fs, &h, i, p, Event::NoteOff(old));
                    }
                }
                self.dividing = false;
            }
            for n in 0..pending.len {
                let event = Event::NoteOn(pending.notes[n], pending.velocities[n]);
                for (i, p) in self.players.iter_mut().enumerate() {
                    queue(clock, fs, &h, i, p, event);
                }
            }
        }
    }

    /// The note each active player plays from the chord, with no more notes
    /// than players: the players keep the notes they have, and no note keeps
    /// more than its share of the section (`players / notes`, the earlier
    /// notes one more where it doesn't divide). The players that leaves free
    /// take the notes that just came in, the lower-numbered players first:
    /// the front desks take the first note of the chord. With more notes than
    /// players, [`Section::steal`] takes over.
    fn divide(&self, pending: &Pending) -> [Option<u8>; MAX_PLAYERS] {
        let m = self.chord.len;
        let n = self.players.iter().filter(|p| p.active).count();
        let mut target = [None; MAX_PLAYERS];
        let mut have: [usize; MAX_CHORD] = [0; MAX_CHORD];
        for (i, p) in self.players.iter().enumerate() {
            if !p.active {
                continue;
            }
            if let Some(voice) = p.voice
                && let Some(j) = self.chord.position(voice)
            {
                target[i] = Some(voice);
                have[j] += 1;
            }
        }
        let mut want: [usize; MAX_CHORD] = [0; MAX_CHORD];
        for (j, w) in want[..m].iter_mut().enumerate() {
            *w = n / m + usize::from(j < n % m);
        }
        // The back of a note's players moves to a new note first.
        for j in 0..m {
            let mut over = have[j].saturating_sub(want[j]);
            for i in (0..MAX_PLAYERS).rev() {
                if over == 0 {
                    break;
                }
                if target[i] == Some(self.chord.notes[j]) {
                    target[i] = None;
                    have[j] -= 1;
                    over -= 1;
                }
            }
        }
        for (seat, p) in target.iter_mut().zip(self.players.iter()) {
            if seat.is_some() || !p.active {
                continue;
            }
            let free = (0..m).find(|&j| pending.contains(self.chord.notes[j]) && have[j] < want[j]);
            if let Some(j) = free {
                *seat = Some(self.chord.notes[j]);
                have[j] += 1;
            }
        }
        target
    }

    /// Moves the players onto `target`, starting the notes that just came in
    /// and ending the notes they leave: the seating both [`Section::divide`]
    /// and [`Section::steal`] work out.
    fn seat(&mut self, target: &[Option<u8>; MAX_PLAYERS], pending: &Pending) {
        let h = self.humanization;
        let fs = self.fs;
        let clock = self.clock;
        let velocity = |note: u8| pending.velocity(note);
        for (i, (&want, p)) in target.iter().zip(self.players.iter_mut()).enumerate() {
            if !p.active {
                continue;
            }
            match (p.voice, want) {
                (Some(old), Some(new)) if old == new => {
                    // The same note pressed again: a new stroke.
                    if pending.contains(new) {
                        queue(clock, fs, &h, i, p, Event::NoteOn(new, velocity(new)));
                    }
                }
                (Some(old), Some(new)) => {
                    // The player leaves its note for the new one. A note
                    // still waiting for its delay is dropped instead of
                    // played for the few samples before the change.
                    cancel(&mut p.queue, old);
                    queue(clock, fs, &h, i, p, Event::NoteOff(old));
                    queue(clock, fs, &h, i, p, Event::NoteOn(new, velocity(new)));
                    p.voice = Some(new);
                }
                (Some(old), None) => {
                    cancel(&mut p.queue, old);
                    queue(clock, fs, &h, i, p, Event::NoteOff(old));
                    p.voice = None;
                }
                (None, Some(new)) => {
                    queue(clock, fs, &h, i, p, Event::NoteOn(new, velocity(new)));
                    p.voice = Some(new);
                }
                (None, None) => {}
            }
        }
        self.dividing = true;
    }

    /// More notes pressed than players: every active player plays one note
    /// (never a double stop), and a note that comes in with no player free
    /// takes the one whose note is closest to it in pitch, ending that note.
    /// A note left without a player is dropped: a player never joins a note
    /// already sounding.
    fn steal(&self, pending: &Pending) -> [Option<u8>; MAX_PLAYERS] {
        let mut target = [None; MAX_PLAYERS];
        let mut taken = [false; MAX_CHORD];
        // The players keep the notes they have, one player per note: the
        // front desk keeps it and the rest are free for the new notes.
        for (i, p) in self.players.iter().enumerate() {
            if !p.active {
                continue;
            }
            if let Some(voice) = p.voice
                && let Some(j) = self.chord.position(voice)
                && !taken[j]
            {
                taken[j] = true;
                target[i] = Some(voice);
            }
        }
        for &note in &pending.notes[..pending.len] {
            let Some(j) = self.chord.position(note) else {
                continue;
            };
            if taken[j] {
                continue;
            }
            let free = target
                .iter()
                .zip(self.players.iter())
                .position(|(seat, p)| seat.is_none() && p.active);
            let seat = free.or_else(|| {
                self.players
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.active)
                    .filter_map(|(i, p)| Some((i, target[i].or(p.voice)?.abs_diff(note))))
                    .min_by_key(|&(_, distance)| distance)
                    .map(|(i, _)| i)
            });
            if let Some(i) = seat {
                target[i] = Some(note);
                taken[j] = true;
            }
        }
        target
    }

    /// Forgets the chord the players are divided over, without touching them.
    fn forget_chord(&mut self) {
        self.chord.clear();
        self.pending.clear();
        self.dividing = false;
        for p in &mut self.players {
            p.voice = None;
        }
    }

    /// Releases every held note, as MIDI "all notes off"; notes still
    /// waiting are dropped.
    pub fn release_all(&mut self) {
        self.forget_chord();
        for p in &mut self.players {
            p.queue.clear();
            p.performer.release_all();
        }
    }

    /// Silences everything at once.
    pub fn reset(&mut self) {
        self.forget_chord();
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
        // The notes that came in on this sample are divided together.
        self.flush();
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
            let output = if i == 0 {
                self.frame = p.performer.process_frame();
                self.frame.output
            } else {
                p.performer.process()
            };
            *y = p.gain * output;
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
        Event::Sustain(on) => performer.set_sustain(on),
        Event::Bow(direction, velocity) => performer.set_bow_direction(direction, velocity),
    }
}

/// Delivers `event` to player `i` now, or queues it for its delay: player 0
/// at once, every other player its own humanizing delay after `clock`, and
/// never before an event already waiting for it (the order of a legato
/// line's notes is what makes it legato).
fn queue(clock: u64, fs: f32, h: &Humanization, i: usize, p: &mut Player, event: Event) {
    if !p.active {
        return;
    }
    let delay = if i == 0 {
        0.0
    } else {
        (h.delay * p.draws.delay + h.jitter * signed(&mut p.rng)).max(0.0)
    };
    let time = (clock + (delay * fs) as u64).max(p.queue.last().unwrap_or(0));
    if time <= clock && p.queue.len == 0 {
        play(&mut p.performer, event);
    } else if !p.queue.push(time, event) {
        // Full (a flood of notes): play it now.
        play(&mut p.performer, event);
    }
}

/// Drops a note still waiting for its delay: the player plays another one
/// instead, so the note it was going to start never begins.
fn cancel(queue: &mut Queue, note: u8) {
    queue.remove(|event| matches!(event, Event::NoteOn(n, _) if *n == note));
}

/// The section's settings as one player plays them.
fn vary_settings(s: &PerformerSettings, d: &Draws, h: &Humanization) -> PerformerSettings {
    let time = 1.0 + h.timing * d.timing;
    let t = s.tuning;
    let beta = 1.0 - h.beta * d.beta;
    PerformerSettings {
        beta: (s.beta.0 * beta, s.beta.1 * beta),
        tasto: s.tasto * beta,
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
