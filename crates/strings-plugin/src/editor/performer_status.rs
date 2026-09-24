//! What the performer is doing, below the instrument: its last three
//! articulations (newest on top, largest and brightest), as SWAM shows them,
//! and beside them the direction of the current or last stroke, drawn as the
//! down-bow (⊓) or up-bow (V) mark, and on the other side the sustain
//! pedal, a button for playing without one.

use std::sync::atomic::Ordering::Relaxed;

use nih_plug_egui::egui::{self, Align2, Color32, FontId, Rect, Stroke, pos2, vec2};
use strings_dsp::{ARTICULATIONS, Articulation};

use crate::shared::{GuiEvent, Shared, Telemetry};

/// Height the status takes below the instrument.
pub const HEIGHT: f32 = 78.0;

/// Font size and brightness of each line, newest first.
const LINES: [(f32, u8); ARTICULATIONS] = [(17.0, 255), (13.0, 175), (11.0, 120)];

pub fn show(ui: &mut egui::Ui, t: &Telemetry, shared: &Shared) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let panel = Rect::from_center_size(rect.center(), vec2(190.0, HEIGHT - 8.0));
    painter.rect_filled(panel, 6.0, Color32::from_black_alpha(150));
    let mut y = panel.top() + 7.0;
    for (slot, (size, gray)) in t.articulations.iter().zip(LINES) {
        let index = slot.load(Relaxed);
        if let Some(&a) = usize::try_from(index)
            .ok()
            .and_then(|i| Articulation::ALL.get(i))
        {
            painter.text(
                pos2(panel.center().x, y),
                Align2::CENTER_TOP,
                label(a),
                FontId::proportional(size),
                Color32::from_gray(gray),
            );
        }
        y += size + 6.0;
    }

    // The bow mark, dimmed between notes.
    let sounding = t.note.load(Relaxed) >= 0;
    let color = Color32::from_gray(if sounding { 235 } else { 120 });
    let down = t.bow_direction.load(Relaxed) > 0.0;
    let center = pos2(panel.right() + 34.0, panel.center().y - 8.0);
    let stroke = Stroke::new(2.5_f32, color);
    let (w, h) = (18.0, 16.0);
    if down {
        // ⊓: a heavy bar across the top, two legs.
        let (l, r, top) = (center.x - w / 2.0, center.x + w / 2.0, center.y - h / 2.0);
        painter.rect_filled(
            Rect::from_min_max(pos2(l, top), pos2(r, top + 4.0)),
            0.0,
            color,
        );
        painter.line_segment([pos2(l + 1.0, top), pos2(l + 1.0, top + h)], stroke);
        painter.line_segment([pos2(r - 1.0, top), pos2(r - 1.0, top + h)], stroke);
    } else {
        let top = center.y - h / 2.0;
        painter.line_segment(
            [pos2(center.x - w / 2.0, top), pos2(center.x, top + h)],
            stroke,
        );
        painter.line_segment(
            [pos2(center.x + w / 2.0, top), pos2(center.x, top + h)],
            stroke,
        );
    }
    painter.text(
        pos2(center.x, center.y + h / 2.0 + 6.0),
        Align2::CENTER_TOP,
        if down { "Down bow" } else { "Up bow" },
        FontId::proportional(11.0),
        color,
    );
    let mark = Rect::from_center_size(pos2(center.x, center.y + 8.0), vec2(64.0, 44.0));
    ui.interact(mark, ui.id().with("bow mark"), egui::Sense::hover())
        .on_hover_text(
            "The current or last stroke. The keyswitches after the bow lift's (cello: E1 \
             down, F1 up) change bow during a note, or set the next stroke. Bringing the \
             dynamics to zero for a moment also changes bow.",
        );

    // The pedal: a button for playing without one, lit while it is down
    // (from the button or CC64).
    let pedal = t.sustain.load(Relaxed);
    let button = Rect::from_center_size(
        pos2(panel.left() - 38.0, panel.center().y),
        vec2(52.0, 24.0),
    );
    let response = ui
        .interact(button, ui.id().with("pedal"), egui::Sense::click())
        .on_hover_text(
            "The sustain pedal (CC64); click to press or let up. While it is down, a note \
             let go keeps the bow going, and the next one changes bow: détaché. \
             Overlapping notes stay slurred.",
        );
    if response.clicked() {
        shared.send(GuiEvent::Sustain(!pedal));
    }
    let visuals = ui.visuals();
    if pedal {
        painter.rect_filled(button, 4.0, visuals.selection.bg_fill);
    } else {
        let gray = if response.hovered() { 140 } else { 90 };
        painter.rect_stroke(
            button,
            4.0,
            Stroke::new(1.0_f32, Color32::from_gray(gray)),
            egui::StrokeKind::Inside,
        );
    }
    painter.text(
        button.center(),
        Align2::CENTER_CENTER,
        "Pedal",
        FontId::proportional(11.0),
        Color32::from_gray(if pedal { 255 } else { 150 }),
    );
}

fn label(a: Articulation) -> &'static str {
    match a {
        Articulation::SoftAttack => "Soft Attack",
        Articulation::Detache => "Détaché",
        Articulation::AccentedAttack => "Accented Attack",
        Articulation::StaccatoAttack => "Staccato Attack",
        Articulation::Martele => "Martelé",
        Articulation::Legato => "Slurred Legato",
        Articulation::Shift => "Shift",
        Articulation::Portamento => "Portamento",
        Articulation::StringCrossing => "Cross-String Legato",
        Articulation::DoubleStop => "Double Stop",
        Articulation::BowLift => "Bow Lift",
        Articulation::Spiccato => "Spiccato",
        Articulation::BowStop => "Bow Stop",
        Articulation::BowChange => "Bow Change",
    }
}
