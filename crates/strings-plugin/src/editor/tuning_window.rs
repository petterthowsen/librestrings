//! The tuning window: the model's numbers, editable while playing.
//!
//! Each number is a [`Knob`] reading and writing a field of [`Tuning`]. Most
//! changes reach the audio thread within a block. The strings' damping,
//! stiffness and torsion need their filters fitted again, which happens on a
//! thread of its own once the mouse is released. "Copy changes" puts the
//! changed values on the clipboard, to be pasted into the presets.
//!
//! Changes aren't saved with the plugin's state: they are for finding values
//! by ear, which then go into the code.

use std::fmt::Write as _;
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use nih_plug_egui::egui::{self, Color32, RichText, Stroke, pos2, vec2};
use strings_dsp::{Body, BodyTuning, BowedString, Instrument, InstrumentSpec, Loss, MAX_PLAYERS};

use crate::params::InstrumentParam;
use crate::shared::{Shared, Telemetry};
use crate::tuning::{LiveTuning, StringsTuning, StringsUpdate, Tuning};

/// Marks changed values, as the faders mark a CC's value.
const CHANGED: Color32 = Color32::from_rgb(240, 170, 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Attack,
    Sustain,
    Release,
    Legato,
    OnString,
    Vibrato,
    Intonation,
    Bow,
    Body,
    Section,
    Strings,
}

impl Group {
    const ALL: [Self; 11] = [
        Self::Attack,
        Self::Sustain,
        Self::Release,
        Self::Legato,
        Self::OnString,
        Self::Vibrato,
        Self::Intonation,
        Self::Bow,
        Self::Body,
        Self::Section,
        Self::Strings,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Attack => "Attack",
            Self::Sustain => "Sustain",
            Self::Release => "Release and left hand",
            Self::Legato => "Legato",
            Self::OnString => "Bow on the string",
            Self::Vibrato => "Vibrato",
            Self::Intonation => "Intonation",
            Self::Bow => "Bow",
            Self::Body => "Body",
            Self::Section => "Section: how the players differ (not player 0)",
            Self::Strings => "Strings (refitted on release, about 30 ms)",
        }
    }
}

type Get = Box<dyn Fn(&Tuning) -> f32 + Send + Sync>;
type Set = Box<dyn Fn(&mut Tuning, f32) + Send + Sync>;

/// A field of [`Tuning`] as `(path, getter, setter)`.
macro_rules! field {
    ($($path:tt)+) => {
        (
            stringify!($($path)+),
            Box::new(|t: &Tuning| t.$($path)+) as Get,
            Box::new(|t: &mut Tuning, v: f32| t.$($path)+ = v) as Set,
        )
    };
}

struct Knob {
    group: Group,
    label: String,
    /// The field, as written in the copied text.
    path: String,
    unit: &'static str,
    range: RangeInclusive<f32>,
    log: bool,
    integer: bool,
    help: &'static str,
    get: Get,
    set: Set,
}

impl Knob {
    fn new(group: Group, label: impl Into<String>, (path, get, set): (&str, Get, Set)) -> Self {
        Self {
            group,
            label: label.into(),
            path: path.replace(' ', ""),
            unit: "",
            range: 0.0..=1.0,
            log: false,
            integer: false,
            help: "",
            get,
            set,
        }
    }

    fn range(mut self, min: f32, max: f32) -> Self {
        self.range = min..=max;
        self
    }

    /// A logarithmic slider from `min` to `max`.
    fn log(mut self, min: f32, max: f32) -> Self {
        self.range = min..=max;
        self.log = true;
        self
    }

    fn unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    fn integer(mut self) -> Self {
        self.integer = true;
        self
    }

    fn help(mut self, help: &'static str) -> Self {
        self.help = help;
        self
    }
}

/// Every editable number, in display order.
fn knobs(spec: &InstrumentSpec, defaults: &Tuning) -> Vec<Knob> {
    use Group::*;
    let mut k = vec![
        Knob::new(
            Attack,
            "Acceleration, soft",
            field!(live.performer.tuning.attack.0),
        )
        .log(0.01, 0.4)
        .unit(" s")
        .help("Time the bow takes to reach full speed in a détaché stroke, at velocity 0."),
        Knob::new(
            Attack,
            "Acceleration, hard",
            field!(live.performer.tuning.attack.1),
        )
        .log(0.005, 0.2)
        .unit(" s")
        .help("The same at velocity 127."),
        Knob::new(
            Attack,
            "Slower at pp",
            field!(live.performer.tuning.pp_attack),
        )
        .range(0.0, 3.0)
        .unit(" ×")
        .help("At dynamics 0 the attack takes (1 + this) times as long."),
        Knob::new(Attack, "Bite", field!(live.performer.tuning.attack_bite))
            .range(0.0, 0.6)
            .help(
                "Extra pressure (position in the Helmholtz band) at the start of a stroke at \
                 velocity 127, scaled by velocity and fading out: the bow biting into the \
                 string. More gives a harder, grainier start.",
            ),
        Knob::new(
            Attack,
            "Bite length",
            field!(live.performer.tuning.attack_bite_time),
        )
        .log(0.01, 0.3)
        .unit(" s"),
        Knob::new(
            Attack,
            "Bow change",
            field!(live.performer.tuning.bow_change),
        )
        .log(0.002, 0.1)
        .unit(" s")
        .help(
            "A new stroke while the bow still moves (fast détaché): the bow slows to \
                 zero over this long before it accelerates. Longer puts the bow change \
                 later in the note.",
        ),
        Knob::new(Attack, "Bow landing", field!(live.performer.tuning.land))
            .log(0.002, 0.1)
            .unit(" s")
            .help("How long the bow takes to settle its weight on the string."),
        Knob::new(
            Attack,
            "Pressure tilt",
            field!(live.performer.tuning.pressure_tilt),
        )
        .range(0.0, 0.4)
        .help(
            "The pressure sits this much higher in the band at dynamics 0 and as much lower \
                 at 1. Quiet attacks only start cleanly high in the band.",
        ),
        Knob::new(
            Sustain,
            "Quiet ease",
            field!(live.performer.tuning.quiet_ease),
        )
        .range(0.0, 1.0)
        .help(
            "Once a quiet stroke is going, the bow eases this far down the band (at \
                 dynamics 0, less when louder): a lighter bow and a darker pp.",
        ),
        Knob::new(
            Sustain,
            "Ease time",
            field!(live.performer.tuning.ease_time),
        )
        .log(0.05, 2.0)
        .unit(" s")
        .help("How long the quiet stroke takes to ease down after its attack."),
        Knob::new(Sustain, "Pressure, normal", field!(live.performer.pressure)).help(
            "Position in the Helmholtz band (0 its lower edge, 1 its upper) at the middle \
                 of the pressure control.",
        ),
        Knob::new(
            Sustain,
            "Pressure, flautando",
            field!(live.performer.pressure_range.0),
        )
        .range(-0.5, 0.65)
        .help(
            "Band position at pressure 0. Below 0 notes break into multiple slips and miss \
                 their pitch.",
        ),
        Knob::new(
            Sustain,
            "Bow position, flautando",
            field!(live.performer.tasto),
        )
        .range(0.0, 0.3)
        .unit(" β")
        .help(
            "Below the pressure control's middle the bow moves from where the dynamics \
                 put it toward this position (sul tasto), never toward the bridge. 0 keeps \
                 it where it is, as for the cello, whose strings play flat above about 0.12.",
        ),
        Knob::new(
            Sustain,
            "Pressure, scratch",
            field!(live.performer.pressure_range.1),
        )
        .range(0.65, 2.0)
        .help("Band position at pressure 1. Above about 1.2 nearly every note is raucous."),
        Knob::new(Sustain, "Bow speed at pp", field!(live.performer.speed.0))
            .log(0.01, 0.3)
            .unit(" m/s"),
        Knob::new(Sustain, "Bow speed at ff", field!(live.performer.speed.1))
            .log(0.1, 1.5)
            .unit(" m/s"),
        Knob::new(Sustain, "Bow position at pp", field!(live.performer.beta.0))
            .range(0.04, 0.2)
            .unit(" β")
            .help(
                "Distance from the bridge as a fraction of the vibrating length. Above about \
                 0.12 the model's cello strings play flat (STATUS.md); the violin's and \
                 viola's don't, but their attacks at pp start slowly above about 0.16.",
            ),
        Knob::new(Sustain, "Bow position at ff", field!(live.performer.beta.1))
            .range(0.03, 0.2)
            .unit(" β"),
        Knob::new(
            Sustain,
            "Closest to the bridge",
            field!(live.performer.bow_distance),
        )
        .range(0.0, 0.05)
        .unit(" m·s/kg")
        .help(
            "The bow's closest distance to the bridge per unit of string impedance \
             (0.024: 3.5 cm on the C string, 1.4 cm on the A), so β rises high on a string. \
             Below about 0.02 notes high up the C and G strings play sharp and noisy.",
        ),
        Knob::new(
            Sustain,
            "Bow follows",
            field!(live.performer.tuning.bow_follow),
        )
        .log(0.01, 1.0)
        .unit(" s")
        .help(
            "The bow keeps its place on the string while the finger moves, and drifts \
                 to the new note's position over about this long: a trill is bowed in \
                 between its notes.",
        ),
        Knob::new(
            Sustain,
            "Wander: pressure",
            field!(live.performer.tuning.wander_pressure),
        )
        .range(0.0, 0.2)
        .help("How far the pressure drifts in the band while a note is held."),
        Knob::new(
            Sustain,
            "Wander: bow speed",
            field!(live.performer.tuning.wander_speed),
        )
        .range(0.0, 0.3)
        .help("How far the bow speed drifts, as a fraction (loudness follows it)."),
        Knob::new(
            Sustain,
            "Wander: bow position",
            field!(live.performer.tuning.wander_beta),
        )
        .range(0.0, 0.15)
        .help(
            "How far the bow drifts along the string, as a fraction of β. It colors the \
                 tone, and moves the bowed pitch of high stopped notes a few cents.",
        ),
        Knob::new(
            Sustain,
            "Wander: pace",
            field!(live.performer.tuning.wander_time),
        )
        .log(0.1, 5.0)
        .unit(" s")
        .help("Mean time between the wander's turning points."),
        Knob::new(
            Release,
            "Lift (off string)",
            field!(live.performer.tuning.release),
        )
        .log(0.02, 0.6)
        .unit(" s")
        .help(
            "Off the string, a note's end: the bow slows and lifts off over this long. \
                 A shorter stroke lifts over its own length, still moving (thrown off). On \
                 the string the bow stops instead (Stop).",
        ),
        Knob::new(
            Release,
            "Force follows speed to",
            field!(live.performer.tuning.speed_floor),
        )
        .range(0.0, 0.5)
        .help(
            "While the bow slows, the force follows its speed down to this fraction of \
                 the full speed, so the note fades with the bow.",
        ),
        Knob::new(
            Release,
            "Finger damping",
            field!(live.performer.tuning.finger_loss),
        )
        .range(0.0, 0.1)
        .unit(" Np")
        .help(
            "Loss at the stopping finger per reflection: how long a stopped note rings \
                 after the bow leaves (open strings ring on). 0.015 is about 1.6 s to −60 dB \
                 at D4, longer for lower notes.",
        ),
        Knob::new(
            Release,
            "Finger release damping",
            field!(live.performer.tuning.mute_loss),
        )
        .range(0.0, 0.5)
        .unit(" Np")
        .help(
            "Extra loss once the note is over (key up and the bow off or stopped) and the \
                 finger eases off the string.",
        ),
        Knob::new(
            Release,
            "Finger release time",
            field!(live.performer.tuning.mute_time),
        )
        .log(0.005, 0.5)
        .unit(" s"),
        Knob::new(
            Release,
            "Shortest stroke",
            field!(live.performer.tuning.min_stroke),
        )
        .log(0.01, 0.2)
        .unit(" s")
        .help("However short the key press, the bow plays a stroke this long."),
        Knob::new(
            Release,
            "Bow change at zero",
            field!(live.performer.tuning.auto_bow_change),
        )
        .log(0.02, 1.0)
        .unit(" s")
        .help(
            "The dynamics brought to zero and resting there this long changes the bow, \
             once, at that quiet moment.",
        ),
        Knob::new(
            Legato,
            "Portamento",
            field!(live.performer.tuning.portamento),
        )
        .log(0.02, 1.0)
        .unit(" s")
        .help("The finger's slide to a legato note landing at velocity 0."),
        Knob::new(
            Legato,
            "Portamento below",
            field!(live.performer.tuning.portamento_velocity),
        )
        .help(
            "Legato notes landing below this velocity slide, slower the softer; above it \
                 they change at once.",
        ),
        Knob::new(Legato, "Shift", field!(live.performer.tuning.shift))
            .log(0.002, 0.2)
            .unit(" s")
            .help("A legato shift of the hand pressed hard: the finger slides this long."),
        Knob::new(
            Legato,
            "Finger placement",
            field!(live.performer.tuning.place),
        )
        .log(0.001, 0.05)
        .unit(" s")
        .help(
            "A finger dropping onto or lifting off the string: legato notes within the \
                 hand, and fingers placed at a bow change or crossing.",
        ),
        Knob::new(Legato, "Hand span", field!(live.performer.tuning.hand_span))
            .range(1.0, 8.0)
            .unit(" st")
            .help("Legato notes within this many semitones of the hand's position need no shift."),
        Knob::new(
            Legato,
            "String crossing",
            field!(live.performer.tuning.crossing),
        )
        .log(0.005, 0.2)
        .unit(" s")
        .help("Force moving from one string to the next."),
        Knob::new(
            Legato,
            "Stay on a string",
            field!(live.performer.tuning.legato_stick),
        )
        .range(0.0, 12.0)
        .unit(" st")
        .help(
            "In a legato line, change string only if the other one plays the note this \
                 many semitones lower in position.",
        ),
        Knob::new(
            Legato,
            "Fingering: mid position",
            field!(live.performer.tuning.mid_bias),
        )
        .range(0.0, 24.0)
        .unit(" st")
        .help(
            "In mid position, notes this far above a lower string's open pitch are played \
                 on it. Just above a fifth avoids open strings.",
        ),
        Knob::new(
            Legato,
            "Fingering: near the bridge",
            field!(live.performer.tuning.bridge_bias),
        )
        .range(0.0, 24.0)
        .unit(" st")
        .help("The same, near the bridge."),
        Knob::new(
            Legato,
            "Double-stop release",
            field!(live.performer.tuning.chord),
        )
        .log(0.005, 0.2)
        .unit(" s")
        .help(
            "Keys of a double stop let go within this long of each other end the stroke \
                 together; otherwise the first one's string is left alone.",
        ),
        Knob::new(OnString, "Grip", field!(live.performer.tuning.grip))
            .log(0.002, 0.1)
            .unit(" s")
            .help("The force is on this long before the bow moves."),
        Knob::new(
            OnString,
            "Acceleration, soft",
            field!(live.performer.tuning.grip_attack.0),
        )
        .log(0.002, 0.3)
        .unit(" s")
        .help("From the string, at velocity 0."),
        Knob::new(
            OnString,
            "Acceleration, hard",
            field!(live.performer.tuning.grip_attack.1),
        )
        .log(0.002, 0.3)
        .unit(" s")
        .help("At velocity 127: a martelé."),
        Knob::new(OnString, "Bite", field!(live.performer.tuning.bite))
            .range(0.0, 0.6)
            .help("Extra pressure in the band during the grip, at velocity 127."),
        Knob::new(
            OnString,
            "Bite length",
            field!(live.performer.tuning.bite_time),
        )
        .log(0.005, 0.2)
        .unit(" s")
        .help("The bite fades out over this long once the bow moves."),
        Knob::new(OnString, "Stop", field!(live.performer.tuning.stop))
            .log(0.005, 0.2)
            .unit(" s")
            .help("The bow slows to a stop on the string at the end of a note."),
        Knob::new(Vibrato, "Rate", field!(live.performer.vibrato_rate))
            .range(3.0, 8.0)
            .unit(" Hz"),
        Knob::new(Vibrato, "Depth", field!(live.performer.vibrato_depth))
            .range(0.0, 1.0)
            .unit(" st")
            .help("Either way, at full vibrato control."),
        Knob::new(
            Vibrato,
            "Delay",
            field!(live.performer.tuning.vibrato_delay),
        )
        .range(0.0, 0.5)
        .unit(" s"),
        Knob::new(
            Vibrato,
            "Fade-in",
            field!(live.performer.tuning.vibrato_fade),
        )
        .log(0.02, 1.5)
        .unit(" s"),
        Knob::new(
            Intonation,
            "Ear time",
            field!(live.performer.tuning.ear_time),
        )
        .log(0.02, 1.0)
        .unit(" s")
        .help("How quickly the performer corrects the pitch of a stopped note by ear."),
        Knob::new(
            Intonation,
            "Ear window",
            field!(live.performer.tuning.ear_window),
        )
        .range(0.1, 2.0)
        .unit(" st"),
        Knob::new(
            Intonation,
            "Largest correction",
            field!(live.performer.tuning.ear_range),
        )
        .range(0.0, 2.0)
        .unit(" st"),
        Knob::new(Bow, "Static friction μs", field!(live.friction.mu_s))
            .range(0.3, 1.5)
            .help("The friction curve of rosin: μs at rest, falling toward μd while slipping."),
        Knob::new(Bow, "Dynamic friction μd", field!(live.friction.mu_d)).range(0.05, 0.8),
        Knob::new(Bow, "Friction slip speed v0", field!(live.friction.v0))
            .log(0.01, 1.0)
            .unit(" m/s")
            .help("How fast friction falls from μs to μd as the slip speeds up."),
        Knob::new(Bow, "Bow noise", field!(live.bow_noise.level))
            .range(0.0, 0.3)
            .help("How much the friction fluctuates while the string slips (rosin and hair). Fitted to recorded cello notes."),
        Knob::new(Bow, "Bow noise bandwidth", field!(live.bow_noise.cutoff))
            .log(200.0, 20_000.0)
            .unit(" Hz")
            .help("Gated to the slips, the noise is broadband whatever this is; it tilts it a little."),
    ];
    if defaults.live.hair.is_some() {
        k.push(
            Knob::new(
                Bow,
                "Hair stiffness",
                (
                    "live.hair.stiffness",
                    Box::new(|t: &Tuning| t.live.hair.map_or(0.0, |h| h.stiffness)),
                    Box::new(|t: &mut Tuning, v| {
                        if let Some(h) = &mut t.live.hair {
                            h.stiffness = v;
                        }
                    }),
                ),
            )
            .log(100.0, 1e6)
            .unit(" N/m")
            .help("The bow hair's give at the contact (fitted to the measured playability map)."),
        );
        k.push(
            Knob::new(
                Bow,
                "Hair damping",
                (
                    "live.hair.damping",
                    Box::new(|t: &Tuning| t.live.hair.map_or(0.0, |h| h.damping)),
                    Box::new(|t: &mut Tuning, v| {
                        if let Some(h) = &mut t.live.hair {
                            h.damping = v;
                        }
                    }),
                ),
            )
            .log(0.3, 100.0)
            .unit(" kg/s"),
        );
        k.push(
            Knob::new(
                Bow,
                "Hair width",
                (
                    "live.hair.width",
                    Box::new(|t: &Tuning| t.live.hair.map_or(0.0, |h| h.width)),
                    Box::new(|t: &mut Tuning, v| {
                        if let Some(h) = &mut t.live.hair {
                            h.width = v;
                        }
                    }),
                ),
            )
            .range(0.0, 0.02)
            .unit(" m")
            .help("How much of the hair ribbon touches the string. Wider: darker at pp, a wider band."),
        );
    }

    for i in 0..defaults.live.body.mode_count {
        let n = i + 1;
        k.push(
            Knob::new(
                Body,
                format!("Mode {n}: frequency"),
                (
                    &format!("live.body.modes[{i}].frequency"),
                    Box::new(move |t: &Tuning| t.live.body.modes[i].frequency),
                    Box::new(move |t: &mut Tuning, v| t.live.body.modes[i].frequency = v),
                ),
            )
            .log(50.0, 600.0)
            .unit(" Hz"),
        );
        k.push(
            Knob::new(
                Body,
                format!("Mode {n}: damping"),
                (
                    &format!("live.body.modes[{i}].damping"),
                    Box::new(move |t: &Tuning| t.live.body.modes[i].damping),
                    Box::new(move |t: &mut Tuning, v| t.live.body.modes[i].damping = v),
                ),
            )
            .log(0.002, 0.1)
            .unit(" ζ"),
        );
        k.push(
            Knob::new(
                Body,
                format!("Mode {n}: gain"),
                (
                    &format!("live.body.modes[{i}].gain"),
                    Box::new(move |t: &Tuning| t.live.body.modes[i].gain),
                    Box::new(move |t: &mut Tuning, v| t.live.body.modes[i].gain = v),
                ),
            )
            .range(-2.0, 2.0)
            .help("Peak gain; the sign sets the polarity, so neighbors can cancel."),
        );
    }
    k.extend([
        Knob::new(Body, "Dense modes from", field!(live.body.dense.from))
            .log(50.0, 1000.0)
            .unit(" Hz")
            .help("Above the listed modes a body has too many to list: seeded random ones."),
        Knob::new(Body, "Dense modes to", field!(live.body.dense.to))
            .log(1000.0, 16000.0)
            .unit(" Hz"),
        Knob::new(
            Body,
            "Dense mode count",
            (
                "live.body.dense.count",
                Box::new(|t: &Tuning| t.live.body.dense.count as f32),
                Box::new(|t: &mut Tuning, v| t.live.body.dense.count = v.round() as usize),
            ),
        )
        .range(0.0, strings_dsp::body::MAX_DENSE_MODES as f32)
        .integer(),
        Knob::new(Body, "Dense damping", field!(live.body.dense.damping))
            .log(0.005, 0.1)
            .unit(" ζ"),
        Knob::new(Body, "Dense level", field!(live.body.dense.level)).range(0.0, 1.5),
        Knob::new(Body, "Roll-off above", field!(live.body.dense.rolloff))
            .log(500.0, 10000.0)
            .unit(" Hz"),
    ]);
    let seed = defaults.live.body.dense.seed;
    k.push(
        Knob::new(
            Body,
            "Body variant",
            (
                "live.body.dense.seed",
                Box::new(move |t: &Tuning| t.live.body.dense.seed.wrapping_sub(seed) as f32),
                Box::new(move |t: &mut Tuning, v| {
                    t.live.body.dense.seed = seed.wrapping_add(v.round() as u32)
                }),
            ),
        )
        .range(0.0, 100.0)
        .integer()
        .help("Another seed for the dense modes: another body of the same kind."),
    );
    for i in 0..defaults.live.body.hill_count {
        let n = i + 1;
        k.push(
            Knob::new(
                Body,
                format!("Hill {n}: frequency"),
                (
                    &format!("live.body.hills[{i}].frequency"),
                    Box::new(move |t: &Tuning| t.live.body.hills[i].frequency),
                    Box::new(move |t: &mut Tuning, v| t.live.body.hills[i].frequency = v),
                ),
            )
            .log(50.0, 8000.0)
            .unit(" Hz")
            .help("A broad rise in the dense modes' level."),
        );
        k.push(
            Knob::new(
                Body,
                format!("Hill {n}: width"),
                (
                    &format!("live.body.hills[{i}].width"),
                    Box::new(move |t: &Tuning| t.live.body.hills[i].width),
                    Box::new(move |t: &mut Tuning, v| t.live.body.hills[i].width = v),
                ),
            )
            .range(0.1, 2.0)
            .unit(" oct"),
        );
        k.push(
            Knob::new(
                Body,
                format!("Hill {n}: gain"),
                (
                    &format!("live.body.hills[{i}].gain"),
                    Box::new(move |t: &Tuning| t.live.body.hills[i].gain),
                    Box::new(move |t: &mut Tuning, v| t.live.body.hills[i].gain = v),
                ),
            )
            .range(0.0, 5.0),
        );
    }
    k.extend([
        Knob::new(Body, "Output gain", field!(live.performer.output_gain))
            .log(0.01, 0.5)
            .help("After the body, before expression and volume."),
        Knob::new(Section, "Detune", field!(live.humanization.detune))
            .range(0.0, 15.0)
            .unit(" cents")
            .help(
                "Each player's own tuning, up to this far either way. Too much beats like a \
                 chorus on held notes.",
            ),
        Knob::new(Section, "Detune drift", field!(live.humanization.detune_drift))
            .range(0.0, 10.0)
            .unit(" cents")
            .help("Slow wandering around each player's tuning, up to this far either way."),
        Knob::new(Section, "Drift time", field!(live.humanization.detune_time))
            .log(0.5, 20.0)
            .unit(" s")
            .help("About how long each drift takes."),
        Knob::new(Section, "Lateness", field!(live.humanization.delay))
            .range(0.0, 0.08)
            .unit(" s")
            .help("Each player comes in up to this late. Player 0 is never late."),
        Knob::new(Section, "Jitter", field!(live.humanization.jitter))
            .range(0.0, 0.03)
            .unit(" s")
            .help("Each note, up to this much either way around the player's lateness."),
        Knob::new(Section, "Vibrato rate", field!(live.humanization.vibrato_rate))
            .range(0.0, 0.3)
            .unit(" ×")
            .help("Each player's vibrato rate, up to this fraction either way."),
        Knob::new(Section, "Vibrato depth", field!(live.humanization.vibrato_depth))
            .range(0.0, 0.6)
            .unit(" ×")
            .help("Each player's vibrato depth, up to this fraction either way."),
        Knob::new(Section, "Dynamics", field!(live.humanization.dynamics))
            .range(0.0, 0.2)
            .help("Each player's dynamics offset, either way (the control runs 0–1)."),
        Knob::new(Section, "Bow position", field!(live.humanization.beta))
            .range(0.0, 0.3)
            .unit(" ×")
            .help("Each player bows up to this fraction closer to the bridge (never farther)."),
        Knob::new(Section, "Pressure", field!(live.humanization.pressure))
            .range(0.0, 0.3)
            .help("Each player's pressure offset in the Helmholtz band, either way."),
        Knob::new(Section, "Timing", field!(live.humanization.timing))
            .range(0.0, 0.5)
            .unit(" ×")
            .help("Each player's attack, legato and portamento times, up to this fraction either way."),
        Knob::new(Section, "Body frequencies", field!(live.humanization.body_frequency))
            .range(0.0, 0.1)
            .unit(" ×")
            .help("Each player's body modes, moved up to this fraction either way."),
        Knob::new(Section, "Body damping", field!(live.humanization.body_damping))
            .range(0.0, 0.5)
            .unit(" ×")
            .help("Each player's body mode damping, up to this fraction either way."),
    ]);
    // The strings' own physics, where the instrument's strings have it (the
    // violin's and viola's have a one-pole loss and neither stiffness nor torsion).
    let string = &spec.strings[0];
    if matches!(string.loss, Loss::Measured(_)) {
        k.extend([
            Knob::new(Strings, "Damping floor", field!(strings.damping.floor))
                .log(1e-4, 1e-2)
                .unit(" ζ")
                .help(
                    "Damping ratio of the low partials: the measured string's plus the energy lost \
                 into the body. Sets how long open strings ring.",
                ),
            Knob::new(Strings, "Damping at 1 kHz", field!(strings.damping.at_1khz))
                .log(1e-5, 1e-2)
                .unit(" ζ")
                .help(
                    "How much the damping rises with frequency (ζ = floor + this × (f/1 kHz)^exp).",
                ),
            Knob::new(
                Strings,
                "Damping exponent",
                field!(strings.damping.exponent),
            )
            .range(1.0, 5.0)
            .help("Measured: 3.54. It sets how dark the ringing string gets."),
        ]);
    }
    if string.bending_stiffness > 0.0 {
        k.push(
            Knob::new(
                Strings,
                "Bending stiffness",
                field!(strings.bending_stiffness),
            )
            .log(1e-6, 2e-2)
            .unit(" N·m²")
            .help(
                "EI of the string: its inharmonicity. Measured on the cello's G string: \
                 3.03e-4; the bass's 5e-3 is an estimate.",
            ),
        );
    }
    if string.torsion.is_some() {
        k.extend([
            Knob::new(
                Strings,
                "Torsion impedance",
                field!(strings.torsion_impedance),
            )
            .log(1.0, 10.0)
            .unit(" × Z")
            .help("Torsional impedance at the string's surface, relative to the transverse one."),
            Knob::new(
                Strings,
                "Torsion frequency",
                field!(strings.torsion_frequency),
            )
            .range(BowedString::LOWEST_TORSION_RATIO, 12.0)
            .unit(" × f0"),
            Knob::new(Strings, "Torsion Q", field!(strings.torsion_q)).log(5.0, 500.0),
        ]);
    }
    k
}

pub struct TuningState {
    pub open: bool,
    /// The instrument whose engine the changes go to.
    instrument: InstrumentParam,
    tuning: Tuning,
    defaults: Tuning,
    knobs: Vec<Knob>,

    /// The engine the changes were sent to; a new one starts from the defaults.
    engine: u32,
    sent_live: Option<LiveTuning>,
    /// The strings as last sent for fitting.
    sent_strings: StringsTuning,
    generation: u32,
    /// The update being fitted or on its way.
    pending: Option<u32>,
    /// When "copied" was last shown (egui time).
    copied_at: Option<f64>,
    /// The body's response in dB at `PLOT_POINTS` frequencies, for the tuning
    /// it was computed for; and the default body's.
    response: Option<(BodyTuning, Vec<f32>)>,
    default_response: Vec<f32>,
}

impl Default for TuningState {
    fn default() -> Self {
        Self::new(InstrumentParam::Cello)
    }
}

impl TuningState {
    fn new(instrument: InstrumentParam) -> Self {
        let spec = instrument.spec();
        let defaults = Tuning::new(spec);
        Self {
            open: false,
            instrument,
            tuning: defaults,
            defaults,
            knobs: knobs(spec, &defaults),
            engine: 0,
            sent_live: None,
            sent_strings: defaults.strings,
            generation: 0,
            pending: None,
            copied_at: None,
            response: None,
            default_response: body_response(&defaults.live.body),
        }
    }

    /// Sends changes to the audio thread and frees returned string filters.
    /// Runs every frame, with the window open or not.
    pub fn sync(&mut self, ctx: &egui::Context, shared: &Arc<Shared>) {
        let t = &shared.telemetry;
        while shared.string_returns.pop().is_some() {}
        if !t.ready.load(Relaxed) {
            return;
        }
        let instrument = t.instrument();
        if instrument != self.instrument {
            // Another instrument's engine: its presets, and none of the
            // changes made to the last one.
            *self = Self {
                open: self.open,
                ..Self::new(instrument)
            };
        }
        let engine = t.engine.load(Relaxed);
        if engine != self.engine {
            // A new engine has the defaults.
            self.engine = engine;
            self.sent_live = None;
            self.sent_strings = self.defaults.strings;
            self.pending = None;
        }
        if self.sent_live != Some(self.tuning.live)
            && shared
                .live_tuning
                .push((self.instrument, self.tuning.live))
                .is_ok()
        {
            self.sent_live = Some(self.tuning.live);
        }
        if self
            .pending
            .is_some_and(|g| t.strings_generation.load(Relaxed) >= g)
        {
            self.pending = None;
        }
        // Fit once the slider is let go, not on every step of a drag.
        let dragging = ctx.input(|i| i.pointer.any_down());
        // The string rate is known once the engine has run.
        let string_rate = t.string_rate.load(Relaxed);
        let player_rate = t.player_string_rate.load(Relaxed);
        if self.tuning.strings != self.sent_strings
            && !dragging
            && self.pending.is_none()
            && string_rate > 0.0
            && player_rate > 0.0
        {
            self.fit_strings(shared, string_rate, player_rate);
        }
        if self.pending.is_some() {
            ctx.request_repaint();
        }
    }

    /// Fits the strings for player 0 at `string_rate` and for the other
    /// players at `player_rate`, and copies the latter for each of them.
    fn fit_strings(&mut self, shared: &Arc<Shared>, string_rate: f32, player_rate: f32) {
        self.generation += 1;
        let generation = self.generation;
        self.pending = Some(generation);
        self.sent_strings = self.tuning.strings;
        let specs = self
            .tuning
            .strings
            .apply_to(&self.instrument.spec().strings);
        let instrument = self.instrument;
        let shared = shared.clone();
        std::thread::spawn(move || {
            let designs = Instrument::design_strings(&specs, string_rate);
            let player = if player_rate == string_rate {
                designs.clone()
            } else {
                Instrument::design_strings(&specs, player_rate)
            };
            let player_designs = vec![player; MAX_PLAYERS - 1];
            let update = StringsUpdate {
                generation,
                instrument,
                specs,
                designs,
                player_designs,
            };
            // A full queue means the audio thread has stopped; drop it.
            let _ = shared.string_updates.push(Box::new(update));
        });
    }

    pub fn window(&mut self, ctx: &egui::Context, t: &Telemetry) {
        let mut open = self.open;
        egui::Window::new(format!("Tuning: {}", self.instrument.spec().name))
            .open(&mut open)
            .default_pos(pos2(440.0, 70.0))
            .default_size(vec2(470.0, 520.0))
            .vscroll(true)
            .show(ctx, |ui| self.ui(ui, t));
        self.open = open;
    }

    fn changed(&self) -> usize {
        self.knobs
            .iter()
            .filter(|k| (k.get)(&self.tuning) != (k.get)(&self.defaults))
            .count()
    }

    fn ui(&mut self, ui: &mut egui::Ui, t: &Telemetry) {
        let changed = self.changed();
        ui.horizontal(|ui| {
            let text = match changed {
                0 => "Presets unchanged".to_string(),
                1 => "1 value changed".to_string(),
                n => format!("{n} values changed"),
            };
            ui.label(RichText::new(text).color(if changed > 0 {
                CHANGED
            } else {
                ui.visuals().weak_text_color()
            }));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(changed > 0, egui::Button::new("Reset all"))
                    .clicked()
                {
                    self.tuning = self.defaults;
                }
                if ui
                    .button("Copy all")
                    .on_hover_text("Every value, as field = value")
                    .clicked()
                {
                    ui.ctx().copy_text(self.text(false));
                    self.copied_at = Some(ui.input(|i| i.time));
                }
                if ui
                    .add_enabled(changed > 0, egui::Button::new("Copy changes"))
                    .on_hover_text("The changed values, to paste into a chat or the presets")
                    .clicked()
                {
                    ui.ctx().copy_text(self.text(true));
                    self.copied_at = Some(ui.input(|i| i.time));
                }
            });
        });
        let now = ui.input(|i| i.time);
        let status = if self.pending.is_some() {
            Some(("Fitting the strings…", ui.visuals().text_color()))
        } else if self.copied_at.is_some_and(|at| now - at < 2.0) {
            ui.ctx().request_repaint();
            Some(("Copied to the clipboard", CHANGED))
        } else {
            None
        };
        ui.label(
            RichText::new(status.map_or(
                "Changes are lost when the plugin closes: copy them to keep them.",
                |s| s.0,
            ))
            .small()
            .color(status.map_or(ui.visuals().weak_text_color(), |s| s.1)),
        );
        ui.separator();

        for group in Group::ALL {
            egui::CollapsingHeader::new(group.name())
                .default_open(false)
                .show(ui, |ui| {
                    if group == Group::Body {
                        self.body_plot(ui, t);
                    }
                    self.group_ui(ui, group);
                });
        }
    }

    fn group_ui(&mut self, ui: &mut egui::Ui, group: Group) {
        egui::Grid::new(group.name())
            .num_columns(3)
            .spacing([8.0, 3.0])
            .show(ui, |ui| {
                ui.spacing_mut().slider_width = 170.0;
                for k in self.knobs.iter().filter(|k| k.group == group) {
                    let default = (k.get)(&self.defaults);
                    let mut value = (k.get)(&self.tuning);
                    let changed = value != default;
                    let label = RichText::new(&k.label);
                    let label = if changed { label.color(CHANGED) } else { label };
                    let response = ui.label(label);
                    if !k.help.is_empty() {
                        response.on_hover_text(k.help);
                    }
                    let mut slider = egui::Slider::new(&mut value, k.range.clone())
                        .logarithmic(k.log)
                        .suffix(k.unit)
                        .custom_formatter(|v, _| format_value(v as f32))
                        .custom_parser(|s| s.trim().parse::<f64>().ok());
                    if k.integer {
                        slider = slider.integer();
                    }
                    if ui.add(slider).changed() {
                        (k.set)(&mut self.tuning, value);
                    }
                    if changed {
                        let reset = ui
                            .small_button("↺")
                            .on_hover_text(format!("Default: {}", format_value(default)));
                        if reset.clicked() {
                            (k.set)(&mut self.tuning, default);
                        }
                    } else {
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
    }

    /// The body's response, with the default body behind it and the partials
    /// of the note being played along the bottom.
    fn body_plot(&mut self, ui: &mut egui::Ui, t: &Telemetry) {
        let body = self.tuning.live.body;
        if self.response.as_ref().is_none_or(|(b, _)| *b != body) {
            self.response = Some((body, body_response(&body)));
        }
        let Some((_, response)) = &self.response else {
            return;
        };

        let width = ui.available_width().min(460.0);
        let (rect, _) = ui.allocate_exact_size(vec2(width, 130.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals();
        painter.rect_filled(rect, 3.0, visuals.extreme_bg_color);
        let (lo, hi) = PLOT_RANGE;
        let x_of = |f: f32| rect.left() + rect.width() * (f / lo).ln() / (hi / lo).ln();
        let top = self
            .default_response
            .iter()
            .chain(response)
            .fold(f32::MIN, |a, &b| a.max(b))
            + 3.0;
        let y_of = |db: f32| rect.top() + rect.height() * ((top - db) / PLOT_DB).clamp(0.0, 1.0);

        let grid = Stroke::new(1.0_f32, Color32::from_white_alpha(18));
        let small = egui::FontId::proportional(10.0);
        for f in [100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0] {
            let x = x_of(f);
            painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], grid);
            let label = if f >= 1000.0 {
                format!("{}k", f / 1000.0)
            } else {
                format!("{f}")
            };
            painter.text(
                pos2(x + 2.0, rect.bottom() - 2.0),
                egui::Align2::LEFT_BOTTOM,
                label,
                small.clone(),
                visuals.weak_text_color(),
            );
        }
        for db in [0.0, -20.0, -40.0] {
            let y = y_of(top - 3.0 + db);
            painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], grid);
        }

        // The partials of the sounding note.
        let bowed = t.string.load(Relaxed) as usize % 4;
        if t.note().is_some() {
            let f0 = t.strings[bowed].frequency.load(Relaxed);
            let mut n = 1;
            while f0 > 0.0 && n as f32 * f0 < hi {
                let x = x_of(n as f32 * f0);
                let alpha = (160.0 / n as f32).max(30.0) as u8;
                painter.line_segment(
                    [pos2(x, rect.bottom() - 10.0), pos2(x, rect.bottom())],
                    Stroke::new(
                        1.5_f32,
                        Color32::from_rgba_unmultiplied(240, 170, 60, alpha),
                    ),
                );
                n += 1;
            }
        }

        let points = |dbs: &[f32]| -> Vec<egui::Pos2> {
            dbs.iter()
                .enumerate()
                .map(|(i, &db)| pos2(x_of(plot_frequency(i)), y_of(db)))
                .collect()
        };
        if body != self.defaults.live.body {
            painter.add(egui::Shape::line(
                points(&self.default_response),
                Stroke::new(1.0_f32, Color32::from_white_alpha(60)),
            ));
        }
        painter.add(egui::Shape::line(
            points(response),
            Stroke::new(1.5_f32, visuals.selection.bg_fill),
        ));
        ui.label(
            RichText::new(
                "Body response (dB); grey: the preset; ticks: the partials of the note playing",
            )
            .small()
            .weak(),
        );
        ui.add_space(4.0);
    }

    /// The values as `field = value`, one per line: only the changed ones, or all.
    fn text(&self, changed_only: bool) -> String {
        let mut out = String::new();
        let changed = self.changed();
        let _ = writeln!(
            out,
            "# Strings tuning, {}: {changed} of {} values changed",
            self.instrument.spec().name,
            self.knobs.len()
        );
        for group in Group::ALL {
            let mut header = false;
            for k in self.knobs.iter().filter(|k| k.group == group) {
                let value = (k.get)(&self.tuning);
                let default = (k.get)(&self.defaults);
                if changed_only && value == default {
                    continue;
                }
                if !header {
                    let _ = writeln!(out, "# {}", group.name());
                    header = true;
                }
                let mark = if value != default {
                    format!("  # was {}", format_value(default))
                } else {
                    String::new()
                };
                let _ = writeln!(out, "{} = {}{mark}", k.path, format_value(value));
            }
        }
        out
    }
}

/// Plot range (Hz) and the dB shown below the top.
const PLOT_RANGE: (f32, f32) = (40.0, 8000.0);
const PLOT_DB: f32 = 50.0;
const PLOT_POINTS: usize = 320;

fn plot_frequency(i: usize) -> f32 {
    let (lo, hi) = PLOT_RANGE;
    lo * (hi / lo).powf(i as f32 / (PLOT_POINTS - 1) as f32)
}

/// The body's response in dB at the plot's frequencies (at 48 kHz; allocates).
fn body_response(tuning: &BodyTuning) -> Vec<f32> {
    let body = Body::from_tuning(tuning, 48_000.0);
    (0..PLOT_POINTS)
        .map(|i| 20.0 * body.magnitude(plot_frequency(i)).max(1e-6).log10())
        .collect()
}

/// Four significant digits, in scientific notation for small values.
fn format_value(v: f32) -> String {
    let a = v.abs();
    if a == 0.0 {
        "0".into()
    } else if a < 0.01 {
        format!("{v:.3e}")
    } else if a >= 1000.0 {
        format!("{v:.0}")
    } else {
        let decimals = (3 - a.log10().floor() as i32).clamp(0, 4) as usize;
        let s = format!("{v:.decimals$}");
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_knob_reads_and_writes_its_own_field() {
        for instrument in InstrumentParam::ALL {
            every_knob_reads_and_writes_its_own_field_of(instrument.spec());
        }
    }

    fn every_knob_reads_and_writes_its_own_field_of(spec: &InstrumentSpec) {
        let defaults = Tuning::new(spec);
        let knobs = knobs(spec, &defaults);
        for (i, k) in knobs.iter().enumerate() {
            let mut t = defaults;
            let value = if k.integer { 7.0 } else { 0.123 };
            (k.set)(&mut t, value);
            assert_eq!((k.get)(&t), value, "{}", k.path);
            // No other knob sees the change (two knobs on one field would).
            for (j, other) in knobs.iter().enumerate() {
                if j != i {
                    assert_eq!(
                        (other.get)(&t),
                        (other.get)(&defaults),
                        "{} also changed {}",
                        k.path,
                        other.path
                    );
                }
            }
        }
    }

    #[test]
    fn defaults_are_inside_their_ranges() {
        for instrument in InstrumentParam::ALL {
            let spec = instrument.spec();
            let defaults = Tuning::new(spec);
            for k in knobs(spec, &defaults) {
                let v = (k.get)(&defaults);
                assert!(k.range.contains(&v), "{}: {} = {v}", spec.name, k.path);
            }
        }
    }

    #[test]
    fn values_format_compactly() {
        assert_eq!(format_value(0.2), "0.2");
        assert_eq!(format_value(0.035), "0.035");
        assert_eq!(format_value(3.032e-4), "3.032e-4");
        assert_eq!(format_value(1300.0), "1300");
        assert_eq!(format_value(5.5), "5.5");
    }
}
