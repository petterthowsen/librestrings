//! Vertical faders for the performance controls.
//!
//! Each fader shows its parameter. A MIDI CC can move the performer's control
//! away from the parameter (whichever changed last wins), so a marker shows
//! the value the performer has when it differs.

use std::sync::atomic::Ordering::Relaxed;

use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, RichText, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::params::StringsParams;
use crate::shared::Telemetry;

const SIZE: egui::Vec2 = vec2(30.0, 118.0);

pub fn faders(ui: &mut egui::Ui, params: &StringsParams, setter: &ParamSetter, t: &Telemetry) {
    let live = |a: &AtomicF32| Some(a.load(Relaxed));
    fader(
        ui,
        &params.dynamics,
        "Dynamics",
        "CC11",
        live(&t.dynamics),
        setter,
    );
    fader(
        ui,
        &params.vibrato,
        "Vibrato",
        "CC1",
        live(&t.vibrato),
        setter,
    );
    fader(
        ui,
        &params.pressure,
        "Pressure",
        "",
        live(&t.pressure),
        setter,
    );
    ui.separator();
    fader(ui, &params.volume, "Volume", "", None, setter);
}

/// `live` is the control's value as the performer has it (0–1, which is the
/// parameter's normalized value for the unit controls).
fn fader(
    ui: &mut egui::Ui,
    param: &FloatParam,
    name: &str,
    cc: &str,
    live: Option<f32>,
    setter: &ParamSetter,
) {
    ui.vertical(|ui| {
        ui.set_width(62.0);
        let value = param.modulated_normalized_value();
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new(param.normalized_value_to_string(value, true))
                    .small()
                    .monospace(),
            );
            let (rect, response) = ui.allocate_exact_size(SIZE, Sense::click_and_drag());

            if response.double_clicked() {
                setter.begin_set_parameter(param);
                setter.set_parameter(param, param.default_plain_value());
                setter.end_set_parameter(param);
            } else {
                if response.drag_started() {
                    setter.begin_set_parameter(param);
                }
                if response.dragged()
                    && let Some(pos) = response.interact_pointer_pos()
                {
                    let v = ((rect.bottom() - pos.y) / rect.height()).clamp(0.0, 1.0);
                    setter.set_parameter_normalized(param, v);
                }
                if response.drag_stopped() {
                    setter.end_set_parameter(param);
                }
            }

            let painter = ui.painter();
            let visuals = ui.visuals();
            let track = egui::Rect::from_center_size(rect.center(), vec2(8.0, rect.height()));
            painter.rect_filled(track, 4.0, visuals.extreme_bg_color);
            let y = rect.bottom() - value * rect.height();
            let fill = egui::Rect::from_min_max(pos2(track.left(), y), track.right_bottom());
            painter.rect_filled(fill, 4.0, visuals.selection.bg_fill);
            let handle = egui::Rect::from_center_size(pos2(rect.center().x, y), vec2(26.0, 8.0));
            let handle_color = if response.hovered() || response.dragged() {
                visuals.widgets.hovered.fg_stroke.color
            } else {
                visuals.widgets.inactive.fg_stroke.color
            };
            painter.rect_filled(handle, 2.0, handle_color);

            if let Some(live) = live.filter(|l| (l - value).abs() > 0.005) {
                let y = rect.bottom() - live.clamp(0.0, 1.0) * rect.height();
                let marker = Color32::from_rgb(240, 170, 60);
                painter.line_segment(
                    [pos2(rect.left(), y), pos2(rect.right(), y)],
                    Stroke::new(2.0_f32, marker),
                );
            }
            if response.hovered() {
                painter.rect_stroke(
                    rect,
                    2.0,
                    visuals.widgets.hovered.bg_stroke,
                    StrokeKind::Outside,
                );
            }
            response.on_hover_text("Drag to set, double-click to reset");

            ui.label(RichText::new(name).small());
            if !cc.is_empty() {
                ui.label(RichText::new(cc).small().weak());
            }
        });
    });
}
