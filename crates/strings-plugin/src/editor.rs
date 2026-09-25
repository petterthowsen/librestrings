//! The editor (ROADMAP.md "Plugin GUI"): a status row, instrument and
//! ensemble selection, a view of the instrument with the performer's state,
//! and at the bottom an on-screen keyboard and faders, so the plugin can be
//! played without a MIDI controller.

use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, RichText};
use nih_plug_egui::{EguiState, create_egui_editor};

use crate::keyswitch_base;
use crate::params::{
    AbsorptionParam, BowLiftParam, FingeringParam, InstrumentParam, PolyphonyParam, RoomParam,
    StringsParams,
};
use crate::shared::{GuiEvent, Shared, Telemetry};

mod fader;
mod instrument_view;
mod keyboard;
mod performer_status;
mod tuning_window;

/// Editor-only state, kept while the window is open.
#[derive(Default)]
struct EditorState {
    keyboard: keyboard::KeyboardState,
    view: instrument_view::ViewState,
    tuning: tuning_window::TuningState,
}

pub fn create(params: Arc<StringsParams>, shared: Arc<Shared>) -> Option<Box<dyn Editor>> {
    let egui_state: Arc<EguiState> = params.editor_state.clone();
    create_egui_editor(
        egui_state,
        EditorState::default(),
        |_, _| {},
        move |ctx, setter, state| {
            let t = &shared.telemetry;
            keyboard::computer_keys(ctx, &params, &shared, &mut state.keyboard);
            state.tuning.sync(ctx, &shared);

            egui::TopBottomPanel::top("status")
                .show(ctx, |ui| status_row(ui, &params, t, &mut state.tuning.open));
            egui::TopBottomPanel::top("selection").show(ctx, |ui| {
                selection_row(ui, &params, setter, &shared);
                stage_row(ui, &params, setter);
            });
            egui::TopBottomPanel::bottom("controls")
                .exact_height(190.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            keyboard::piano(ui, &params, &shared, &mut state.keyboard);
                        });
                        ui.separator();
                        fader::faders(ui, &params, setter, t);
                    });
                });
            egui::SidePanel::right("readout")
                .exact_width(230.0)
                .resizable(false)
                .show(ctx, |ui| readout(ui, &params, t));
            egui::CentralPanel::default().show(ctx, |ui| {
                let size = ui.available_size() - egui::vec2(0.0, performer_status::HEIGHT);
                ui.allocate_ui(size, |ui| instrument_view::show(ui, t, &mut state.view));
                performer_status::show(ui, t, &shared);
            });
            if state.tuning.open {
                state.tuning.window(ctx, t);
            }

            // The strings animate.
            ctx.request_repaint();
        },
    )
}

fn status_row(ui: &mut egui::Ui, params: &StringsParams, t: &Telemetry, tuning: &mut bool) {
    ui.horizontal(|ui| {
        let small = |text: String| RichText::new(text).small().monospace();
        ui.label(small(format!("Strings {}", env!("CARGO_PKG_VERSION"))));
        ui.separator();
        if !t.ready.load(Relaxed) {
            ui.label(small("not processing".into()).color(Color32::YELLOW));
        } else {
            let fs = t.sample_rate.load(Relaxed);
            let block = t.block_size.load(Relaxed);
            ui.label(small(format!("{:.1} kHz · {block} smp", fs / 1000.0)));
            ui.separator();
            let load = t.load.load(Relaxed) * 100.0;
            let peak = t.load_peak.load(Relaxed) * 100.0;
            // The PLAN.md budget: 3% of one core for a solo instrument, 25%
            // for a 12-player section.
            let budget = if t.players.load(Relaxed) > 1 {
                25.0
            } else {
                3.0
            };
            let color = if load > budget {
                Color32::YELLOW
            } else {
                ui.visuals().weak_text_color()
            };
            ui.label(small(format!("DSP {load:4.1}% (peak {peak:4.1}%)")).color(color));
            ui.separator();
            let out = util::gain_to_db(t.output_peak.load(Relaxed));
            let out_color = if out > -0.1 {
                Color32::RED
            } else {
                ui.visuals().weak_text_color()
            };
            let out_text = if out > -90.0 {
                format!("out {out:5.1} dBFS")
            } else {
                "out  -inf dBFS".into()
            };
            ui.label(small(out_text).color(out_color));
            let resets = t.resets.load(Relaxed);
            if resets > 0 {
                ui.separator();
                ui.label(small(format!("NaN resets {resets}")).color(Color32::RED));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            toggle(ui, &params.debug_view, "Debug");
            toggle(ui, &params.computer_keys, "Computer keys");
            ui.checkbox(tuning, RichText::new("Tuning").small())
                .on_hover_text("The model's numbers, editable while playing");
        });
    });
}

fn toggle(ui: &mut egui::Ui, flag: &std::sync::atomic::AtomicBool, label: &str) {
    let mut value = flag.load(Relaxed);
    if ui
        .checkbox(&mut value, RichText::new(label).small())
        .changed()
    {
        flag.store(value, Relaxed);
    }
}

fn selection_row(ui: &mut egui::Ui, params: &StringsParams, setter: &ParamSetter, shared: &Shared) {
    ui.horizontal(|ui| {
        ui.label("Instrument");
        enum_combo(
            ui,
            setter,
            &params.instrument,
            &InstrumentParam::ALL,
            InstrumentParam::name,
            130.0,
        )
        .on_hover_text("Changing it cuts off what is sounding.");
        ui.add_space(12.0);
        ui.label("Players");
        players(ui, params, setter);
        ui.add_space(24.0);

        ui.label("Bow lift");
        let live = shared.telemetry.bow_lift();
        let base = keyswitch_base(shared.telemetry.instrument().spec());
        for (i, b) in BowLiftParam::ALL.into_iter().enumerate() {
            let key = keyboard::note_name(base + 2 * i as u8);
            let what = match b {
                BowLiftParam::OffString => {
                    "The bow leaves the string at the end of a note, which rings on. Short \
                     notes are thrown off (spiccato-like)."
                }
                BowLiftParam::OnString => {
                    "The bow stops on the string at the end of a note: short notes are \
                     staccato. Notes start from a grip; pressed hard, a martelé."
                }
            };
            let response = ui
                .selectable_label(live == b, b.name())
                .on_hover_text(format!("{what}\nKeyswitch {key}"));
            if response.clicked() {
                setter.begin_set_parameter(&params.bow_lift);
                setter.set_parameter(&params.bow_lift, b);
                setter.end_set_parameter(&params.bow_lift);
                shared.send(GuiEvent::BowLift(b));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Polyphony");
        let live = shared.telemetry.polyphony();
        let base = keyswitch_base(shared.telemetry.instrument().spec());
        for (i, p) in PolyphonyParam::ALL.into_iter().enumerate() {
            let key = keyboard::note_name(base + 7 + 2 * i as u8);
            let what = match p {
                PolyphonyParam::Mono => "Overlapping notes play legato: one note at a time.",
                PolyphonyParam::DoubleStops => {
                    "A note held with another plays with it on the next string, where one \
                     hand can play both. Otherwise it plays legato."
                }
                PolyphonyParam::Divisi => {
                    "A section divides a chord's notes among its players, one each. A solo \
                     plays the double stops it can."
                }
            };
            let response = ui
                .selectable_label(live == p, p.name())
                .on_hover_text(format!("{what}\nKeyswitch {key}"));
            if response.clicked() {
                setter.begin_set_parameter(&params.polyphony);
                setter.set_parameter(&params.polyphony, p);
                setter.end_set_parameter(&params.polyphony);
                shared.send(GuiEvent::Polyphony(p));
            }
        }
        ui.add_space(12.0);
        ui.label("Fingering");
        enum_combo(
            ui,
            setter,
            &params.fingering,
            &FingeringParam::ALL,
            FingeringParam::name,
            160.0,
        )
        .on_hover_text(
            "Where the left hand plays. Near the nut: open strings and low positions. \
             Mid position: no open strings but the lowest. Near the bridge: high \
             positions on lower strings, a darker sound.",
        );
    });
}

/// The section's size: `‹ Solo ›`, `‹ 8 players ›`.
fn players(ui: &mut egui::Ui, params: &StringsParams, setter: &ParamSetter) {
    let param = &params.players;
    let n = param.value();
    let (min, max) = (1, strings_dsp::MAX_PLAYERS as i32);
    let set = |value: i32| {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    };
    if ui.add_enabled(n > min, egui::Button::new("‹")).clicked() {
        set(n - 1);
    }
    let text = if n == 1 {
        "Solo".to_string()
    } else {
        format!("{n} players")
    };
    ui.add_sized(
        [72.0, 18.0],
        egui::Label::new(RichText::new(text).monospace()),
    )
    .on_hover_text(
        "Players in the section, each a little different in tuning, timing, \
             vibrato, bowing and instrument.",
    );
    if ui.add_enabled(n < max, egui::Button::new("›")).clicked() {
        set(n + 1);
    }
}

/// Where the section sits and the room it plays in, until the stage view
/// (docs/SECTIONS.md Phase B). Every instance should have the same room and
/// mics.
fn stage_row(ui: &mut egui::Ui, params: &StringsParams, setter: &ParamSetter) {
    ui.horizontal(|ui| {
        let mut on = params.stage.value();
        if ui
            .checkbox(&mut on, "Stage")
            .on_hover_text(
                "On: the players sit on a stage in a room, picked up by a stereo mic \
                 pair, with early reflections (the late reverb is left to your reverb). \
                 Off: dry and mono.",
            )
            .changed()
        {
            setter.begin_set_parameter(&params.stage);
            setter.set_parameter(&params.stage, on);
            setter.end_set_parameter(&params.stage);
        }
        ui.add_enabled_ui(on, |ui| {
            ui.add_space(8.0);
            ui.label("x");
            param_drag(ui, setter, &params.stage_x, -12.0..=12.0, " m")
                .on_hover_text("To the audience's right (m); 0 is the middle");
            ui.label("y");
            param_drag(ui, setter, &params.stage_y, 0.0..=12.0, " m")
                .on_hover_text("Upstage from the front of the stage (m)");
            ui.label("Size");
            param_drag(ui, setter, &params.stage_width, 0.0..=12.0, " m")
                .on_hover_text("Width of the area the section fills (m)");
            ui.label("×");
            param_drag(ui, setter, &params.stage_depth, 0.0..=8.0, " m")
                .on_hover_text("Depth of the area the section fills (m)");
            ui.add_space(12.0);
            ui.label("Room");
            enum_combo(
                ui,
                setter,
                &params.room,
                &RoomParam::ALL,
                RoomParam::name,
                110.0,
            )
            .on_hover_text("Set the same room in every instance");
            enum_combo(
                ui,
                setter,
                &params.absorption,
                &AbsorptionParam::ALL,
                AbsorptionParam::name,
                70.0,
            )
            .on_hover_text("How much the walls absorb");
            ui.label("Mics");
            param_drag(ui, setter, &params.mic_distance, 0.5..=20.0, " m")
                .on_hover_text("Distance of the mics in front of the stage: close to far");
            ui.label("Reflections");
            let r = &params.reflections;
            let mut percent = 100.0 * r.value();
            let response = ui.add(
                egui::DragValue::new(&mut percent)
                    .speed(1.0)
                    .range(0.0..=100.0)
                    .suffix(" %"),
            );
            edit_param(setter, r, &response, percent / 100.0);
        });
    });
}

/// A number field for a parameter, dragged or typed.
fn param_drag(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &FloatParam,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
) -> egui::Response {
    let mut value = param.value();
    let response = ui.add(
        egui::DragValue::new(&mut value)
            .speed(0.05)
            .range(range)
            .fixed_decimals(1)
            .suffix(suffix),
    );
    edit_param(setter, param, &response, value);
    response
}

/// Sets `param` from a drag value's `response`: one gesture per drag, or
/// one for a typed value.
fn edit_param(setter: &ParamSetter, param: &FloatParam, response: &egui::Response, value: f32) {
    if response.drag_started() {
        setter.begin_set_parameter(param);
    }
    if response.changed() {
        if response.dragged() {
            setter.set_parameter(param, value);
        } else {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, value);
            setter.end_set_parameter(param);
        }
    }
    if response.drag_stopped() {
        setter.end_set_parameter(param);
    }
}

/// A drop-down for an enum parameter.
fn enum_combo<T: Enum + PartialEq + Copy + 'static>(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &EnumParam<T>,
    all: &[T],
    name: fn(T) -> &'static str,
    width: f32,
) -> egui::Response {
    let current = param.value();
    egui::ComboBox::from_id_salt(param.name())
        .selected_text(name(current))
        .width(width)
        .show_ui(ui, |ui| {
            for &value in all {
                if ui.selectable_label(value == current, name(value)).clicked() {
                    setter.begin_set_parameter(param);
                    setter.set_parameter(param, value);
                    setter.end_set_parameter(param);
                }
            }
        })
        .response
}

/// The performer's state in numbers, beside the instrument.
fn readout(ui: &mut egui::Ui, params: &StringsParams, t: &Telemetry) {
    let bowed = t.string.load(Relaxed) as usize % 4;
    let spec = &t.instrument().spec().strings[bowed];
    let string = &t.strings[bowed];
    let frequency = string.frequency.load(Relaxed);
    let beta = string.beta.load(Relaxed);
    let v = t.bow_velocity.load(Relaxed);
    let force = t.bow_force.load(Relaxed);
    let slips = t.slips_per_period.load(Relaxed);

    ui.add_space(4.0);
    let note = match (t.note(), t.second_note()) {
        (Some(n), Some(second)) => {
            let (lo, hi) = (n.min(second), n.max(second));
            format!("{} {}", keyboard::note_name(lo), keyboard::note_name(hi))
        }
        (Some(n), None) => keyboard::note_name(n),
        (None, _) => "–".into(),
    };
    ui.label(RichText::new(note).size(28.0).strong());
    ui.add_space(4.0);

    egui::Grid::new("readout")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            let mut row = |name: &str, value: String| {
                ui.label(RichText::new(name).weak());
                ui.label(RichText::new(value).monospace());
                ui.end_row();
            };
            let position = 12.0 * (frequency / spec.frequency).log2();
            row(
                "String",
                format!("{} ({:.0} Hz open)", spec.name, spec.frequency),
            );
            row("Position", format!("{position:+.2} st"));
            row("Pitch", format!("{frequency:.2} Hz"));
            row("Bow β", format!("{beta:.3}"));
            let arrow = if v > 1e-4 {
                "↑"
            } else if v < -1e-4 {
                "↓"
            } else {
                " "
            };
            row("Bow speed", format!("{:.3} m/s {arrow}", v.abs()));
            row("Bow force", format!("{force:.2} N"));
            let (motion, _) = motion(slips);
            row("Motion", motion.into());
        });

    let (_, color) = motion(slips);
    if slips > 0.0 {
        ui.label(
            RichText::new(format!("{slips:.2} slips / period"))
                .small()
                .color(color),
        );
    }

    if params.debug_view.load(Relaxed) {
        ui.separator();
        debug_view(ui, t, bowed, v, beta);
    }
}

/// Names the bowed-string motion from the slips per period.
fn motion(slips: f32) -> (&'static str, Color32) {
    if slips <= 0.0 {
        ("–", Color32::GRAY)
    } else if slips < 0.9 {
        ("irregular", Color32::YELLOW)
    } else if slips <= 1.1 {
        ("Helmholtz", Color32::from_rgb(120, 200, 120))
    } else {
        ("multiple slips", Color32::from_rgb(230, 140, 80))
    }
}

fn debug_view(ui: &mut egui::Ui, t: &Telemetry, bowed: usize, v: f32, beta: f32) {
    // The calibrated Helmholtz band at the current speed and position (PLAN.md 4.2).
    let instrument = t.instrument().spec();
    let spec = &instrument.strings[bowed];
    let semitones = 12.0 * (t.strings[bowed].frequency.load(Relaxed) / spec.frequency).log2();
    let (lo, hi) =
        instrument
            .force_limits_at(bowed, semitones.max(0.0))
            .band(spec.impedance(), v, beta);
    let force = t.bow_force.load(Relaxed);
    ui.label(RichText::new("Force band").weak());
    ui.label(RichText::new(format!("{lo:.2} – {hi:.2} N")).monospace());
    if force > 0.0 && hi > lo {
        let p = (force / lo).ln() / (hi / lo).ln();
        ui.label(RichText::new(format!("position {p:.2}")).monospace());
    }
    ui.add_space(6.0);

    egui::Grid::new("strings")
        .num_columns(4)
        .spacing([10.0, 2.0])
        .show(ui, |ui| {
            for h in ["", "Hz", "bow", "N rms"] {
                ui.label(RichText::new(h).small().weak());
            }
            ui.end_row();
            for (i, s) in t.strings.iter().enumerate().rev() {
                let small = |text: String| RichText::new(text).small().monospace();
                ui.label(small(instrument.strings[i].name.into()));
                ui.label(small(format!("{:.1}", s.frequency.load(Relaxed))));
                ui.label(small(format!("{:.2}", s.contact.load(Relaxed))));
                ui.label(small(format!("{:.3}", s.level.load(Relaxed))));
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    let controls = [
        ("dynamics", &t.dynamics),
        ("vibrato", &t.vibrato),
        ("pressure", &t.pressure),
    ];
    for (name, value) in controls {
        ui.label(
            RichText::new(format!("{name:<10} {:.2}", value.load(Relaxed)))
                .small()
                .monospace(),
        );
    }
}
