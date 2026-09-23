//! The stage: a section's players placed in a room and picked up by a stereo
//! microphone pair (PLAN.md §5, docs/SECTIONS.md A3–A4).
//!
//! Coordinates are metres on one stage shared by every instance: `x` to the
//! right as the audience sees it, `y` upstage from the front edge of the stage,
//! `z` up from the floor. The mics stand on the centre line, `mic_distance` in
//! front of the stage. Two instances with the same room and mics are on the
//! same stage, so their sections sit where their positions say.
//!
//! The mics are a near-coincident pair (as ORTF): two cardioids 17 cm apart,
//! angled 55° to either side. The difference in time and level between them
//! places a player; there is no panner. Each player reaches each mic through a
//! delay (its distance), 1/r and air absorption. The first reflections (four
//! walls, floor and ceiling of a shoebox room, as image sources) come from the
//! section's centre, from the sum of its players. The late reverb is left to
//! the user's reverb.
//!
//! Only relative delays are kept: every path is shortened by the distance
//! from the mics to the front of the stage, the same for every instance, so
//! instances stay in time with each other and no latency is added.
//!
//! Real-time safe: changing the room, the mics, the placement or the number
//! of players only recomputes targets, and delays and gains glide to them.

use crate::delay::DelayLine;
use crate::section::MAX_PLAYERS;

/// Speed of sound (m/s).
const SOUND_SPEED: f32 = 343.0;
/// Height of the instruments and of the mics (m).
const SOURCE_HEIGHT: f32 = 1.0;
const MIC_HEIGHT: f32 = 2.5;
/// Half the spacing of the mic pair (m) and each mic's angle off the centre
/// line (rad).
const MIC_HALF_SPACING: f32 = 0.085;
const MIC_ANGLE: f32 = 55.0 * std::f32::consts::PI / 180.0;
/// Air absorption at 10 kHz (dB/m), about 20 °C and 50% humidity.
const AIR_DB_PER_M: f32 = 0.015 * 10.0;
/// Delays and gains glide to new targets with this time constant (s).
const GLIDE: f32 = 0.05;
/// Players and mics stay this far inside the walls (m).
const WALL_MARGIN: f32 = 0.5;
/// Largest random offset of a player from its place in the grid (m).
const SEAT_JITTER: f32 = 0.15;
/// The six first-order reflections.
const IMAGES: usize = 6;

/// A shoebox room around the stage (m). The stage's back wall is at `y =
/// back`; the room runs `length` from there toward the audience.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    pub width: f32,
    pub length: f32,
    pub height: f32,
    pub back: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RoomPreset {
    Studio,
    #[default]
    ChamberHall,
    ConcertHall,
    ScoringStage,
}

impl RoomPreset {
    pub const ALL: [Self; 4] = [
        Self::Studio,
        Self::ChamberHall,
        Self::ConcertHall,
        Self::ScoringStage,
    ];

    pub fn room(self) -> Room {
        let (width, length, height, back) = match self {
            Self::Studio => (12.0, 16.0, 6.0, 7.0),
            Self::ChamberHall => (16.0, 26.0, 11.0, 8.0),
            Self::ConcertHall => (24.0, 42.0, 17.0, 11.0),
            Self::ScoringStage => (22.0, 30.0, 12.0, 12.0),
        };
        Room {
            width,
            length,
            height,
            back,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Studio => "Studio",
            Self::ChamberHall => "Chamber hall",
            Self::ConcertHall => "Concert hall",
            Self::ScoringStage => "Scoring stage",
        }
    }
}

/// How much the walls absorb: their reflection gain and how dark they make
/// a reflection (a one-pole lowpass).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Absorption {
    Low,
    #[default]
    Medium,
    High,
}

impl Absorption {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    /// Pressure reflection gain `sqrt(1 − α)` and lowpass cutoff (Hz).
    fn wall(self) -> (f32, f32) {
        match self {
            Self::Low => (0.95, 10_000.0),
            Self::Medium => (0.84, 6_000.0),
            Self::High => (0.63, 3_000.0),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

/// What every instance on the stage shares: the room and the mics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StageSettings {
    pub room: RoomPreset,
    pub absorption: Absorption,
    /// Distance of the mic pair in front of the stage (m): close to far.
    pub mic_distance: f32,
    /// Level of the early reflections: 0 is off, 1 as the room gives them.
    pub reflections: f32,
}

impl Default for StageSettings {
    fn default() -> Self {
        Self {
            room: RoomPreset::default(),
            absorption: Absorption::default(),
            mic_distance: 4.0,
            reflections: 1.0,
        }
    }
}

/// Where a section sits: its centre and the area its players fill (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub depth: f32,
}

impl Placement {
    /// The cellos on the audience's right, as most orchestras seat them.
    pub const CELLOS: Self = Self {
        x: 3.5,
        y: 3.0,
        width: 4.0,
        depth: 3.0,
    };
}

impl Default for Placement {
    fn default() -> Self {
        Self::CELLOS
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Point {
    x: f32,
    y: f32,
    z: f32,
}

impl Point {
    fn distance(self, o: Point) -> f32 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2) + (self.z - o.z).powi(2)).sqrt()
    }
}

/// A path from a source to one mic: a delay read, a gain and a lowpass, the
/// delay and gain gliding to their targets.
#[derive(Clone, Copy, Debug, Default)]
struct Tap {
    delay: f32,
    gain: f32,
    target_delay: f32,
    target_gain: f32,
    /// One-pole lowpass: coefficient and state.
    pole: f32,
    state: f32,
}

impl Tap {
    fn set(&mut self, delay: f32, gain: f32, pole: f32) {
        self.target_delay = delay;
        self.target_gain = gain;
        self.pole = pole;
    }

    fn jump(&mut self) {
        self.delay = self.target_delay;
        self.gain = self.target_gain;
    }

    fn read(&mut self, line: &DelayLine, glide: f32) -> f32 {
        self.delay = self.target_delay + glide * (self.delay - self.target_delay);
        self.gain = self.target_gain + glide * (self.gain - self.target_gain);
        let x = line.read(self.delay);
        self.state = (1.0 - self.pole) * x + self.pole * self.state;
        self.gain * self.state
    }
}

pub struct Stage {
    settings: StageSettings,
    placement: Placement,
    players: usize,
    fs: f32,
    /// Seed for the players' offsets from their seats.
    seed: u32,
    positions: [Point; MAX_PLAYERS],
    lines: [DelayLine; MAX_PLAYERS],
    /// Samples since each player's last sound: once its whole line is
    /// silent, its taps are skipped (players that are off, or rests).
    quiet: [usize; MAX_PLAYERS],
    /// Length of the players' lines (samples).
    line_length: usize,
    /// Direct paths: player, then left and right mic.
    direct: [[Tap; 2]; MAX_PLAYERS],
    /// The section's sum, for the reflections.
    sum: DelayLine,
    reflections: [[Tap; 2]; IMAGES],
    glide: f32,
}

impl Stage {
    /// Allocates the delay lines for the largest room at `sample_rate`; don't
    /// call it on the audio thread.
    pub fn new(settings: StageSettings, placement: Placement, seed: u32, sample_rate: f32) -> Self {
        // The longest direct path is the largest room's diagonal; a first
        // reflection is at most twice that.
        let diagonal = RoomPreset::ALL
            .iter()
            .map(|p| {
                let r = p.room();
                (r.width.powi(2) + r.length.powi(2) + r.height.powi(2)).sqrt()
            })
            .fold(0.0, f32::max);
        let samples = |metres: f32| (metres / SOUND_SPEED * sample_rate).ceil() as usize + 8;
        let mut stage = Self {
            settings,
            placement,
            players: 1,
            fs: sample_rate,
            seed: seed.max(1),
            positions: [Point::default(); MAX_PLAYERS],
            lines: std::array::from_fn(|_| DelayLine::new(samples(diagonal))),
            quiet: [usize::MAX; MAX_PLAYERS],
            line_length: samples(diagonal) + 4,
            direct: [[Tap::default(); 2]; MAX_PLAYERS],
            sum: DelayLine::new(samples(2.0 * diagonal)),
            reflections: [[Tap::default(); 2]; IMAGES],
            glide: (-1.0 / (GLIDE * sample_rate)).exp(),
        };
        // Every seat gets a place, so players fading out keep theirs.
        stage.seat(MAX_PLAYERS);
        stage.seat(1);
        stage.update();
        for tap in stage
            .direct
            .iter_mut()
            .chain(&mut stage.reflections)
            .flatten()
        {
            tap.jump();
        }
        stage
    }

    pub fn settings(&self) -> &StageSettings {
        &self.settings
    }

    pub fn set_settings(&mut self, settings: StageSettings) {
        self.settings = settings;
        self.seat(self.players);
        self.update();
    }

    pub fn placement(&self) -> &Placement {
        &self.placement
    }

    pub fn set_placement(&mut self, placement: Placement) {
        self.placement = placement;
        self.seat(self.players);
        self.update();
    }

    /// The section's size (see [`crate::Section::set_players`]). Players
    /// beyond it keep their seats while they fade out.
    pub fn set_players(&mut self, players: usize) {
        self.players = players.clamp(1, MAX_PLAYERS);
        self.seat(self.players);
        self.update();
    }

    /// Where player `i` sits (x, y in m).
    pub fn position(&self, i: usize) -> (f32, f32) {
        (self.positions[i].x, self.positions[i].y)
    }

    pub fn reset(&mut self) {
        for line in &mut self.lines {
            line.clear();
        }
        self.quiet = [usize::MAX; MAX_PLAYERS];
        self.sum.clear();
        for tap in self
            .direct
            .iter_mut()
            .chain(&mut self.reflections)
            .flatten()
        {
            tap.state = 0.0;
            tap.jump();
        }
    }

    /// One sample of every player's output (as [`crate::Section::process`]
    /// gives them) to the left and right mic.
    pub fn process(&mut self, players: &[f32; MAX_PLAYERS]) -> [f32; 2] {
        let glide = self.glide;
        let mut out = [0.0; 2];
        let mut sum = 0.0;
        let players = self
            .lines
            .iter_mut()
            .zip(&mut self.direct)
            .zip(&mut self.quiet)
            .zip(players);
        for (((line, taps), quiet), &x) in players {
            *quiet = if x == 0.0 { quiet.saturating_add(1) } else { 0 };
            if *quiet > self.line_length {
                // Silent: nothing to glide from when it plays again.
                for tap in taps.iter_mut() {
                    tap.jump();
                    tap.state = 0.0;
                }
                continue;
            }
            line.push(x);
            sum += x;
            for (y, tap) in out.iter_mut().zip(taps.iter_mut()) {
                *y += tap.read(line, glide);
            }
        }
        self.sum.push(sum);
        for taps in &mut self.reflections {
            for (y, tap) in out.iter_mut().zip(taps.iter_mut()) {
                *y += tap.read(&self.sum, glide);
            }
        }
        out
    }

    /// The room, with the placement kept inside it.
    fn room(&self) -> Room {
        self.settings.room.room()
    }

    /// Seats the first `n` players: rows across the section's area, front to
    /// back, each player a little off its seat. One player sits at the centre.
    fn seat(&mut self, n: usize) {
        let room = self.room();
        let p = self.placement;
        let (w, d) = (p.width.max(0.0), p.depth.max(0.0));
        let cols = ((n as f32 * w / d.max(0.1)).sqrt().ceil() as usize).clamp(1, n.max(1));
        let rows = n.div_ceil(cols);
        let half_w = 0.5 * room.width - WALL_MARGIN;
        for i in 0..n {
            let (row, col) = (i / cols, i % cols);
            let in_row = cols.min(n - row * cols);
            let mut x = p.x + w * ((col as f32 + 0.5) / in_row as f32 - 0.5);
            let mut y = p.y + d * ((row as f32 + 0.5) / rows as f32 - 0.5);
            if n > 1 {
                let mut rng = (self.seed ^ (i as u32 + 1).wrapping_mul(0x9e37_79b9)).max(1);
                x += SEAT_JITTER * signed(&mut rng);
                y += SEAT_JITTER * signed(&mut rng);
            }
            self.positions[i] = Point {
                x: x.clamp(-half_w, half_w),
                y: y.clamp(
                    room.back - room.length + WALL_MARGIN,
                    room.back - WALL_MARGIN,
                ),
                z: SOURCE_HEIGHT,
            };
        }
    }

    /// Recomputes every path's target delay, gain and lowpass.
    fn update(&mut self) {
        let room = self.room();
        let s = self.settings;
        let fs = self.fs;
        let front = room.back - room.length + WALL_MARGIN;
        let mic_y = (-s.mic_distance).clamp(front, -WALL_MARGIN);
        let mics = [-1.0, 1.0].map(|side| Point {
            x: side * MIC_HALF_SPACING,
            y: mic_y,
            z: MIC_HEIGHT,
        });
        let axes = [-1.0f32, 1.0].map(|side| (side * MIC_ANGLE.sin(), MIC_ANGLE.cos()));
        // Every path is shortened by the distance to the front of the stage
        // on the centre line, and its level is relative to a player 3 m
        // upstage there: moving the mics changes the balance, not the level.
        let centre = Point {
            x: 0.0,
            y: mic_y,
            z: MIC_HEIGHT,
        };
        let height = MIC_HEIGHT - SOURCE_HEIGHT;
        let reference = (mic_y.powi(2) + height.powi(2) + MIC_HALF_SPACING.powi(2)).sqrt();
        let unit = centre.distance(Point {
            x: 0.0,
            y: 3.0,
            z: SOURCE_HEIGHT,
        });
        // A path of `distance` from `from` to mic `m`, with extra `gain` and
        // wall lowpass `wall_cutoff` (Hz).
        let path = |from: Point, m: usize, gain: f32, wall_cutoff: f32| {
            let mic = mics[m];
            let distance = from.distance(mic).max(0.3);
            let (ax, ay) = axes[m];
            let cos = (ax * (from.x - mic.x) + ay * (from.y - mic.y)) / distance;
            let cardioid = 0.5 * (1.0 + cos);
            let delay = (distance - reference).max(0.0) / SOUND_SPEED * fs;
            let air = air_cutoff(distance);
            // Two one-poles in series, as one: their losses add.
            let cutoff = 1.0 / (air.powi(-2) + wall_cutoff.powi(-2)).sqrt();
            let level = gain * cardioid * (unit / distance).min(4.0);
            (delay, level, one_pole(cutoff, fs))
        };

        for (i, taps) in self.direct.iter_mut().enumerate() {
            for (m, tap) in taps.iter_mut().enumerate() {
                let (delay, gain, pole) = path(self.positions[i], m, 1.0, f32::INFINITY);
                tap.set(delay, gain, pole);
            }
        }

        let p = self.placement;
        let source = Point {
            x: p.x.clamp(
                -0.5 * room.width + WALL_MARGIN,
                0.5 * room.width - WALL_MARGIN,
            ),
            y: p.y.clamp(front, room.back - WALL_MARGIN),
            z: SOURCE_HEIGHT,
        };
        let (wall_gain, wall_cutoff) = s.absorption.wall();
        let images = [
            Point {
                x: -room.width - source.x,
                ..source
            },
            Point {
                x: room.width - source.x,
                ..source
            },
            Point {
                y: 2.0 * room.back - source.y,
                ..source
            },
            Point {
                y: 2.0 * (room.back - room.length) - source.y,
                ..source
            },
            Point {
                z: -source.z,
                ..source
            },
            Point {
                z: 2.0 * room.height - source.z,
                ..source
            },
        ];
        let level = wall_gain * s.reflections.max(0.0);
        for (image, taps) in images.iter().zip(&mut self.reflections) {
            for (m, tap) in taps.iter_mut().enumerate() {
                let (delay, gain, pole) = path(*image, m, level, wall_cutoff);
                tap.set(delay, gain, pole);
            }
        }
    }
}

/// Cutoff (Hz) of a one-pole lowpass that loses as much at 10 kHz as
/// `distance` metres of air.
fn air_cutoff(distance: f32) -> f32 {
    let loss = 10f32.powf(AIR_DB_PER_M * distance / 10.0) - 1.0;
    if loss <= 0.0 {
        f32::INFINITY
    } else {
        10_000.0 / loss.sqrt()
    }
}

/// Coefficient of a one-pole lowpass at `cutoff` (Hz); 0 passes everything.
fn one_pole(cutoff: f32, fs: f32) -> f32 {
    if cutoff >= 0.45 * fs {
        0.0
    } else {
        (-std::f32::consts::TAU * cutoff / fs).exp()
    }
}

/// Uniform in [−1, 1) (xorshift32).
fn signed(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    2.0 * ((*state >> 8) as f32 / (1u32 << 24) as f32) - 1.0
}
