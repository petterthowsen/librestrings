//! This instance's place on the stage and the room it plays in (docs/SECTIONS.md
//! B1–B2), as the editor and the instance sync (`sync`) set them and the
//! engine plays them.
//!
//! These are persisted state, not host parameters: another instance's editor
//! can move this section, and the sync thread that applies the move can't set
//! host parameters. The room is shared by every instance on the stage; each
//! keeps a copy, saved with its project. The audio thread reads the atomics
//! once per block.

use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};
use std::time::{SystemTime, UNIX_EPOCH};

use nih_plug::params::persist::PersistentField;
use nih_plug::prelude::{AtomicF32, PluginState};
use nih_plug::wrapper::state::ParamValue;
use serde::{Deserialize, Serialize};
use strings_dsp::{Absorption, Placement, Room, RoomPreset, StageSettings};

use crate::params::InstrumentParam;

/// The stage instances share unless they are given another name.
pub const DEFAULT_STAGE: &str = "Main";
/// Sections and mics stay this far inside the walls (m).
const WALL_MARGIN: f32 = 0.5;
/// The smallest area a section fills (m).
pub const MIN_SIZE: f32 = 0.5;
/// The farthest the mics go from the front of the stage (m).
const MAX_MIC_DISTANCE: f32 = 20.0;

pub struct Layout {
    // The section: its centre and the area its players fill (`Placement`).
    x: AtomicF32,
    y: AtomicF32,
    width: AtomicF32,
    depth: AtomicF32,
    /// Counts this instance's own changes to the placement (its editor, a
    /// loaded project), which the sync then publishes; changes the sync
    /// brings in from other instances don't count.
    pub placement_edits: AtomicU32,

    // The room, shared by the stage (`StageSettings`).
    room: AtomicU32,
    absorption: AtomicU32,
    mic_distance: AtomicF32,
    mic_x: AtomicF32,
    reflections: AtomicF32,
    /// When the room was last changed, in any instance (ms since the Unix
    /// epoch; 0 is never). An instance whose project has a newer room than the
    /// stage's gives the stage its room.
    room_changed: AtomicU64,
    /// Counts this instance's own changes to the room, as `placement_edits`.
    pub room_edits: AtomicU32,

    /// The section's name ("" for the instrument's) and the stage's. Never
    /// read on the audio thread.
    name: RwLock<String>,
    stage: RwLock<String>,
}

impl Default for Layout {
    fn default() -> Self {
        // A new instance is a soloist at the front, in the middle.
        let placement = Placement {
            x: 0.0,
            y: 1.5,
            width: 4.0,
            depth: 3.0,
        };
        let settings = StageSettings::default();
        Self {
            x: AtomicF32::new(placement.x),
            y: AtomicF32::new(placement.y),
            width: AtomicF32::new(placement.width),
            depth: AtomicF32::new(placement.depth),
            placement_edits: AtomicU32::new(0),
            room: AtomicU32::new(room_index(settings.room)),
            absorption: AtomicU32::new(absorption_index(settings.absorption)),
            mic_distance: AtomicF32::new(settings.mic_distance),
            mic_x: AtomicF32::new(settings.mic_x),
            reflections: AtomicF32::new(settings.reflections),
            room_changed: AtomicU64::new(0),
            room_edits: AtomicU32::new(0),
            name: RwLock::new(String::new()),
            stage: RwLock::new(DEFAULT_STAGE.into()),
        }
    }
}

impl Layout {
    pub fn placement(&self) -> Placement {
        Placement {
            x: self.x.load(Relaxed),
            y: self.y.load(Relaxed),
            width: self.width.load(Relaxed),
            depth: self.depth.load(Relaxed),
        }
    }

    /// Moves the section here (the sync publishes it).
    pub fn set_placement(&self, p: Placement) {
        self.adopt_placement(p);
        self.placement_edits.fetch_add(1, Relaxed);
    }

    /// Takes a placement set by another instance.
    pub fn adopt_placement(&self, p: Placement) {
        self.x.store(p.x, Relaxed);
        self.y.store(p.y, Relaxed);
        self.width.store(p.width, Relaxed);
        self.depth.store(p.depth, Relaxed);
    }

    pub fn settings(&self) -> StageSettings {
        StageSettings {
            room: RoomPreset::ALL[self.room.load(Relaxed) as usize % RoomPreset::ALL.len()],
            absorption: Absorption::ALL
                [self.absorption.load(Relaxed) as usize % Absorption::ALL.len()],
            mic_distance: self.mic_distance.load(Relaxed),
            mic_x: self.mic_x.load(Relaxed),
            reflections: self.reflections.load(Relaxed),
        }
    }

    pub fn room_changed(&self) -> u64 {
        self.room_changed.load(Relaxed)
    }

    /// Changes the room here, now (the sync gives it to the stage).
    pub fn set_settings(&self, s: StageSettings) {
        self.adopt_settings(s, now_ms());
        self.room_edits.fetch_add(1, Relaxed);
    }

    /// Takes the stage's room, last changed at `changed`.
    pub fn adopt_settings(&self, s: StageSettings, changed: u64) {
        self.room.store(room_index(s.room), Relaxed);
        self.absorption
            .store(absorption_index(s.absorption), Relaxed);
        self.mic_distance.store(s.mic_distance, Relaxed);
        self.mic_x.store(s.mic_x, Relaxed);
        self.reflections.store(s.reflections, Relaxed);
        self.room_changed.store(changed, Relaxed);
    }

    /// The name set for the section, or "" for the instrument's.
    pub fn name(&self) -> String {
        self.name.read().map(|n| n.clone()).unwrap_or_default()
    }

    pub fn set_name(&self, name: &str) {
        if let Ok(mut n) = self.name.write() {
            name.trim().clone_into(&mut n);
        }
    }

    pub fn stage(&self) -> String {
        self.stage.read().map(|n| n.clone()).unwrap_or_default()
    }

    pub fn set_stage(&self, stage: &str) {
        if let Ok(mut s) = self.stage.write() {
            stage.trim().clone_into(&mut s);
        }
    }

    fn saved(&self) -> Saved {
        let (p, s) = (self.placement(), self.settings());
        Saved {
            name: self.name(),
            stage: self.stage(),
            x: p.x,
            y: p.y,
            width: p.width,
            depth: p.depth,
            room: room_id(s.room).into(),
            absorption: absorption_id(s.absorption).into(),
            mic_distance: s.mic_distance,
            mic_x: s.mic_x,
            reflections: s.reflections,
            room_changed: self.room_changed(),
        }
    }

    fn load(&self, saved: Saved) {
        self.set_name(&saved.name);
        self.set_stage(&saved.stage);
        self.set_placement(Placement {
            x: saved.x,
            y: saved.y,
            width: saved.width,
            depth: saved.depth,
        });
        let settings = StageSettings {
            room: RoomPreset::ALL
                .into_iter()
                .find(|&r| room_id(r) == saved.room)
                .unwrap_or_default(),
            absorption: Absorption::ALL
                .into_iter()
                .find(|&a| absorption_id(a) == saved.absorption)
                .unwrap_or_default(),
            mic_distance: saved.mic_distance,
            mic_x: saved.mic_x,
            reflections: saved.reflections,
        };
        self.adopt_settings(settings, saved.room_changed);
        self.room_edits.fetch_add(1, Relaxed);
    }
}

/// The layout as projects save it.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    name: String,
    stage: String,
    x: f32,
    y: f32,
    width: f32,
    depth: f32,
    room: String,
    absorption: String,
    mic_distance: f32,
    mic_x: f32,
    reflections: f32,
    room_changed: u64,
}

impl Default for Saved {
    fn default() -> Self {
        Layout::default().saved()
    }
}

impl PersistentField<'_, Saved> for std::sync::Arc<Layout> {
    fn set(&self, saved: Saved) {
        self.load(saved);
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&Saved) -> R,
    {
        f(&self.saved())
    }
}

/// The key the layout is saved under.
pub const PERSIST_KEY: &str = "stage-layout";

/// Moves the stage parameters of a project saved before the stage view
/// (0.2.0) into the layout.
pub fn migrate(state: &mut PluginState) {
    if state.fields.contains_key(PERSIST_KEY) || !state.params.contains_key("stage-x") {
        return;
    }
    let mut take = |id: &str| state.params.remove(id);
    let float = |v: Option<ParamValue>, default: f32| match v {
        Some(ParamValue::F32(v)) => v,
        _ => default,
    };
    let index = |v: Option<ParamValue>| match v {
        Some(ParamValue::I32(i)) => usize::try_from(i).ok(),
        _ => None,
    };
    let mut saved = Saved::default();
    saved.x = float(take("stage-x"), saved.x);
    saved.y = float(take("stage-y"), saved.y);
    saved.width = float(take("stage-width"), saved.width);
    saved.depth = float(take("stage-depth"), saved.depth);
    if let Some(&r) = index(take("room")).and_then(|i| RoomPreset::ALL.get(i)) {
        saved.room = room_id(r).into();
    }
    if let Some(&a) = index(take("absorption")).and_then(|i| Absorption::ALL.get(i)) {
        saved.absorption = absorption_id(a).into();
    }
    saved.mic_distance = float(take("mic-distance"), saved.mic_distance);
    saved.reflections = float(take("reflections"), saved.reflections);
    // Older than any change made since, newer than a new instance's room.
    saved.room_changed = 1;
    if let Ok(json) = serde_json::to_string(&saved) {
        state.fields.insert(PERSIST_KEY.into(), json);
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn room_index(r: RoomPreset) -> u32 {
    RoomPreset::ALL.iter().position(|&p| p == r).unwrap_or(0) as u32
}

pub fn absorption_index(a: Absorption) -> u32 {
    Absorption::ALL.iter().position(|&b| b == a).unwrap_or(0) as u32
}

fn room_id(r: RoomPreset) -> &'static str {
    match r {
        RoomPreset::Studio => "studio",
        RoomPreset::ChamberHall => "chamber-hall",
        RoomPreset::ConcertHall => "concert-hall",
        RoomPreset::ScoringStage => "scoring-stage",
    }
}

fn absorption_id(a: Absorption) -> &'static str {
    match a {
        Absorption::Low => "low",
        Absorption::Medium => "medium",
        Absorption::High => "high",
    }
}

/// A section's name when none is set: "Solo cello", "Celli".
pub fn default_name(instrument: InstrumentParam, players: u32) -> String {
    if players > 1 {
        match instrument {
            InstrumentParam::Violin => "Violins",
            InstrumentParam::Viola => "Violas",
            InstrumentParam::Cello => "Celli",
            InstrumentParam::Bass => "Basses",
        }
        .into()
    } else {
        format!("Solo {}", instrument.name().to_lowercase())
    }
}

/// Where a section's area may go: across the room inside its walls, and from
/// the front of the stage to its back wall (x and y ranges, m).
pub fn section_bounds(room: Room) -> ((f32, f32), (f32, f32)) {
    let half = 0.5 * room.width - WALL_MARGIN;
    ((-half, half), (0.0, room.back - WALL_MARGIN))
}

/// How far the mics may stand from the front of the stage, and across the
/// room (m).
pub fn mic_bounds(room: Room) -> ((f32, f32), (f32, f32)) {
    let farthest = (room.length - room.back - WALL_MARGIN).min(MAX_MIC_DISTANCE);
    let half = 0.5 * room.width - WALL_MARGIN;
    ((WALL_MARGIN, farthest), (-half, half))
}

/// The same area kept inside `room`: moved in, and made smaller only if it
/// doesn't fit.
pub fn clamp_placement(p: Placement, room: Room) -> Placement {
    let ((x0, x1), (y0, y1)) = section_bounds(room);
    let width = p.width.clamp(MIN_SIZE, x1 - x0);
    let depth = p.depth.clamp(MIN_SIZE, y1 - y0);
    Placement {
        x: p.x.clamp(x0 + 0.5 * width, x1 - 0.5 * width),
        y: p.y.clamp(y0 + 0.5 * depth, y1 - 0.5 * depth),
        width,
        depth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn a_saved_layout_loads_back() {
        let a = Layout::default();
        a.set_name("Celli I");
        a.set_placement(Placement {
            x: 3.5,
            y: 3.0,
            width: 4.0,
            depth: 2.5,
        });
        a.set_settings(StageSettings {
            room: RoomPreset::ConcertHall,
            absorption: Absorption::High,
            mic_distance: 8.0,
            mic_x: -1.0,
            reflections: 0.5,
        });
        let json = serde_json::to_string(&a.saved()).unwrap();
        let b = Layout::default();
        let edits = b.placement_edits.load(Relaxed);
        b.load(serde_json::from_str(&json).unwrap());
        assert_eq!(b.name(), "Celli I");
        assert_eq!(b.placement(), a.placement());
        assert_eq!(b.settings(), a.settings());
        assert_eq!(b.room_changed(), a.room_changed());
        // Loading a project is this instance's own change: the sync publishes it.
        assert_ne!(b.placement_edits.load(Relaxed), edits);
    }

    #[test]
    fn old_stage_parameters_move_into_the_layout() {
        let mut params = BTreeMap::new();
        params.insert("stage-x".into(), ParamValue::F32(-3.5));
        params.insert("stage-depth".into(), ParamValue::F32(2.0));
        params.insert("room".into(), ParamValue::I32(2));
        params.insert("mic-distance".into(), ParamValue::F32(9.0));
        params.insert("players".into(), ParamValue::I32(8));
        let mut state = PluginState {
            version: "0.2.0".into(),
            params,
            fields: BTreeMap::new(),
        };
        migrate(&mut state);
        assert!(!state.params.contains_key("stage-x"));
        assert!(state.params.contains_key("players"));
        let layout = Layout::default();
        layout.load(serde_json::from_str(&state.fields[PERSIST_KEY]).unwrap());
        let (p, s) = (layout.placement(), layout.settings());
        assert_eq!((p.x, p.y, p.depth), (-3.5, 1.5, 2.0));
        assert_eq!(s.room, RoomPreset::ConcertHall);
        assert_eq!(s.mic_distance, 9.0);
        assert_eq!(layout.room_changed(), 1);
    }

    #[test]
    fn a_placement_is_kept_inside_the_room() {
        let room = RoomPreset::Studio.room();
        let p = clamp_placement(
            Placement {
                x: 20.0,
                y: -2.0,
                width: 30.0,
                depth: 0.1,
            },
            room,
        );
        let ((x0, x1), (y0, _)) = section_bounds(room);
        assert_eq!(p.width, x1 - x0);
        assert_eq!(p.depth, MIN_SIZE);
        assert!((p.x - 0.0).abs() < 1e-6);
        assert!((p.y - 0.5 * p.depth - y0).abs() < 1e-6);
    }
}
