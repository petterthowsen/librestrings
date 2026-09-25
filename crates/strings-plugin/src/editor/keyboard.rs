//! The on-screen keyboard and the computer-keyboard mapping.
//!
//! One octave and a third are mapped to the computer keyboard in the tracker
//! layout: the Q row plays the white keys and the number row the black keys
//! (Q = C, 2 = C♯, W = D … P = E). Z and X transpose by an octave. Keys are
//! taken by physical position, so the layout works with any keymap.
//!
//! The piano plays with the mouse: the lower on a key, the louder; dragging
//! across keys plays legato. Keyswitches (the white keys from C1) set the
//! bow lift (C, D) and the bow direction (E down, F up).

use std::sync::atomic::Ordering::Relaxed;

use nih_plug_egui::egui::{self, Color32, FontId, Key, RichText, Sense, Stroke, pos2, vec2};

use crate::params::StringsParams;
use crate::shared::{GuiEvent, Shared};
use crate::{Keyswitch, keyswitch, keyswitch_base, midi_note};
use strings_dsp::InstrumentSpec;

const KEYS: [(Key, &str); 17] = [
    (Key::Q, "Q"),
    (Key::Num2, "2"),
    (Key::W, "W"),
    (Key::Num3, "3"),
    (Key::E, "E"),
    (Key::R, "R"),
    (Key::Num5, "5"),
    (Key::T, "T"),
    (Key::Num6, "6"),
    (Key::Y, "Y"),
    (Key::Num7, "7"),
    (Key::U, "U"),
    (Key::I, "I"),
    (Key::Num9, "9"),
    (Key::O, "O"),
    (Key::Num0, "0"),
    (Key::P, "P"),
];

/// How long a key release waits for a following press before it counts.
/// X11 auto-repeat sends a release and a press for every repeat, and baseview
/// never marks them as repeats, so a held key would retrigger its note.
const RELEASE_DEBOUNCE: f64 = 0.02;

/// Transpose range in octaves around C3.
const TRANSPOSE: (i32, i32) = (-2, 3);
/// Range of the on-screen keyboard: five octaves from the keyswitches (violin
/// C3–C8, viola C2–C7, cello C1–C6, bass C0–C5).
fn range(spec: &InstrumentSpec) -> (u8, u8) {
    let lowest = keyswitch_base(spec);
    (lowest, lowest + 60)
}

#[derive(Default)]
pub struct KeyboardState {
    /// The note under the mouse, while a button is held.
    mouse_note: Option<u8>,
    /// The note each computer key started, so a transpose while it is held
    /// still releases the right one.
    computer: [Option<u8>; KEYS.len()],
    /// When each held key's release arrived, while it waits out
    /// [`RELEASE_DEBOUNCE`].
    released_at: [Option<f64>; KEYS.len()],
}

pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B",
    ];
    // C4 is middle C (MIDI 60).
    format!("{}{}", NAMES[note as usize % 12], note as i32 / 12 - 1)
}

fn is_black(note: u8) -> bool {
    matches!(note % 12, 1 | 3 | 6 | 8 | 10)
}

/// The first note mapped to the computer keyboard.
fn first_mapped(params: &StringsParams) -> u8 {
    (48 + 12 * params.transpose.load(Relaxed)) as u8
}

/// The playable range: the lowest open string to the top of the highest string.
fn playable(spec: &InstrumentSpec, note: u8) -> bool {
    let low = midi_note(spec.strings[0].frequency);
    let high = midi_note(spec.strings[3].frequency) + spec.reach as u8;
    (low..=high).contains(&note)
}

fn release_computer_keys(shared: &Shared, state: &mut KeyboardState) {
    for held in &mut state.computer {
        if let Some(note) = held.take() {
            shared.send(GuiEvent::NoteOff { note });
        }
    }
    state.released_at = [None; KEYS.len()];
}

/// Reads the computer keyboard (while the editor has focus).
pub fn computer_keys(
    ctx: &egui::Context,
    params: &StringsParams,
    shared: &Shared,
    state: &mut KeyboardState,
) {
    let enabled = params.computer_keys.load(Relaxed);
    let focused = ctx.input(|i| i.focused);
    if !enabled || !focused || ctx.wants_keyboard_input() {
        // Keys released while we weren't listening would hang.
        release_computer_keys(shared, state);
        return;
    }
    let velocity = params.key_velocity.load(Relaxed) as f32 / 127.0;
    let (events, now) = ctx.input(|i| (i.events.clone(), i.time));
    for event in events {
        let egui::Event::Key {
            key,
            physical_key,
            pressed,
            repeat,
            modifiers,
        } = event
        else {
            continue;
        };
        // Releases always count, or a note would hang. Alt is not checked:
        // on X11 baseview reports the left mouse button as Alt, which would
        // mute the keys while a fader is dragged.
        if pressed && (repeat || modifiers.command) {
            continue;
        }
        let key = physical_key.unwrap_or(key);
        match key {
            Key::Z | Key::X if pressed => {
                let step = if key == Key::Z { -1 } else { 1 };
                let t = (params.transpose.load(Relaxed) + step).clamp(TRANSPOSE.0, TRANSPOSE.1);
                params.transpose.store(t, Relaxed);
            }
            _ => {
                let Some(i) = KEYS.iter().position(|(k, _)| *k == key) else {
                    continue;
                };
                if pressed {
                    // A press right after a release is auto-repeat: keep the note.
                    state.released_at[i] = None;
                    if state.computer[i].is_none() {
                        let note = first_mapped(params) + i as u8;
                        state.computer[i] = Some(note);
                        shared.send(GuiEvent::NoteOn { note, velocity });
                    }
                } else if state.computer[i].is_some() {
                    state.released_at[i] = Some(now);
                }
            }
        }
    }
    let mut waiting = false;
    for (held, released) in state.computer.iter_mut().zip(&mut state.released_at) {
        let Some(t) = *released else { continue };
        if now - t >= RELEASE_DEBOUNCE {
            *released = None;
            if let Some(note) = held.take() {
                shared.send(GuiEvent::NoteOff { note });
            }
        } else {
            waiting = true;
        }
    }
    if waiting {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(RELEASE_DEBOUNCE));
    }
}

/// The on-screen piano, with transpose and velocity controls below it.
pub fn piano(
    ui: &mut egui::Ui,
    params: &StringsParams,
    shared: &Shared,
    state: &mut KeyboardState,
) {
    let t = &shared.telemetry;
    let spec = t.instrument().spec();
    let (lowest, highest) = range(spec);
    let whites = (lowest..=highest).filter(|&n| !is_black(n)).count() as f32;
    let width = ui.available_width() - 390.0;
    let size = vec2(width.max(300.0), 128.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let white_w = rect.width() / whites;
    let black_h = rect.height() * 0.62;

    let white_rect = |note: u8| {
        let index = (lowest..note).filter(|&n| !is_black(n)).count() as f32;
        egui::Rect::from_min_size(
            pos2(rect.left() + index * white_w, rect.top()),
            vec2(white_w, rect.height()),
        )
    };
    let black_rect = |note: u8| {
        let right_edge = white_rect(note + 1).left();
        egui::Rect::from_center_size(
            pos2(right_edge, rect.top() + black_h / 2.0),
            vec2(white_w * 0.62, black_h),
        )
    };
    let key_rect = |note: u8| {
        if is_black(note) {
            black_rect(note)
        } else {
            white_rect(note)
        }
    };
    let note_at = |pos: egui::Pos2| {
        let black = (lowest..=highest)
            .filter(|&n| is_black(n))
            .find(|&n| black_rect(n).contains(pos));
        black.or_else(|| {
            (lowest..=highest)
                .filter(|&n| !is_black(n))
                .find(|&n| white_rect(n).contains(pos))
        })
    };

    // Mouse: press plays, dragging onto another key plays legato.
    let pointer = response
        .is_pointer_button_down_on()
        .then(|| response.interact_pointer_pos())
        .flatten();
    let target = pointer.and_then(|pos| note_at(pos).map(|n| (n, pos)));
    let current = state.mouse_note;
    match target {
        Some((note, pos)) if current != Some(note) => {
            let r = key_rect(note);
            let velocity = (0.25 + 0.75 * (pos.y - r.top()) / r.height()).clamp(0.0, 1.0);
            shared.send(GuiEvent::NoteOn { note, velocity });
            if let Some(old) = current {
                shared.send(GuiEvent::NoteOff { note: old });
            }
            state.mouse_note = Some(note);
        }
        None if current.is_some() => {
            if let Some(old) = state.mouse_note.take() {
                shared.send(GuiEvent::NoteOff { note: old });
            }
        }
        _ => {}
    }

    let sounding = |note: u8| t.is_sounding(note);
    let bow_lift = t.bow_lift();
    let direction = t.bow_direction.load(Relaxed);
    let first = first_mapped(params);
    let computer = params.computer_keys.load(Relaxed);
    let held = |note: u8| state.mouse_note == Some(note) || state.computer.contains(&Some(note));
    let label = |note: u8| {
        let i = note.checked_sub(first)? as usize;
        (computer && i < KEYS.len()).then(|| KEYS[i].1)
    };
    let sounding_color = Color32::from_rgb(240, 170, 60);
    let held_color = Color32::from_rgb(90, 150, 230);
    let switch_color = |active: bool| {
        if active {
            Color32::from_rgb(200, 90, 90)
        } else {
            Color32::from_rgb(120, 70, 70)
        }
    };

    let painter = ui.painter_at(rect);
    let small = FontId::proportional(9.0);
    for note in (lowest..=highest).filter(|&n| !is_black(n)) {
        let r = white_rect(note).shrink2(vec2(0.5, 0.0));
        let fill = if sounding(note) {
            sounding_color
        } else if held(note) {
            held_color
        } else if let Some(k) = keyswitch(spec, note) {
            switch_color(match k {
                Keyswitch::BowLift(b) => b == bow_lift,
                Keyswitch::Bow(d) => d == direction,
            })
        } else if playable(spec, note) {
            Color32::from_gray(235)
        } else {
            Color32::from_gray(150)
        };
        painter.rect_filled(r, 2.0, fill);
        if let Some(label) = label(note) {
            painter.text(
                pos2(r.center().x, r.bottom() - 22.0),
                egui::Align2::CENTER_CENTER,
                label,
                small.clone(),
                Color32::from_gray(60),
            );
        }
        if note % 12 == 0 {
            painter.text(
                pos2(r.center().x, r.bottom() - 8.0),
                egui::Align2::CENTER_CENTER,
                note_name(note),
                small.clone(),
                Color32::from_gray(90),
            );
        }
    }
    for note in (lowest..=highest).filter(|&n| is_black(n)) {
        let r = black_rect(note);
        let fill = if sounding(note) {
            sounding_color
        } else if held(note) {
            held_color
        } else if playable(spec, note) {
            Color32::from_gray(25)
        } else {
            Color32::from_gray(70)
        };
        painter.rect(
            r,
            2.0,
            fill,
            Stroke::new(1.0_f32, Color32::BLACK),
            egui::StrokeKind::Inside,
        );
        if let Some(label) = label(note) {
            painter.text(
                pos2(r.center().x, r.bottom() - 9.0),
                egui::Align2::CENTER_CENTER,
                label,
                small.clone(),
                Color32::from_gray(200),
            );
        }
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Keys").small());
        let transpose = params.transpose.load(Relaxed);
        if ui
            .add_enabled(transpose > TRANSPOSE.0, egui::Button::new("◀ Z"))
            .clicked()
        {
            params.transpose.store(transpose - 1, Relaxed);
        }
        ui.label(
            RichText::new(format!("{}–{}", note_name(first), note_name(first + 16))).monospace(),
        );
        if ui
            .add_enabled(transpose < TRANSPOSE.1, egui::Button::new("X ▶"))
            .clicked()
        {
            params.transpose.store(transpose + 1, Relaxed);
        }
        ui.add_space(16.0);
        ui.label(RichText::new("Velocity").small());
        let mut velocity = params.key_velocity.load(Relaxed);
        if ui
            .add(egui::Slider::new(&mut velocity, 1..=127).show_value(true))
            .changed()
        {
            params.key_velocity.store(velocity, Relaxed);
        }
    });
}
