//! The editor (ROADMAP.md "Plugin GUI"): a status row, instrument and
//! ensemble selection, a view of the instrument with the performer's state,
//! and at the bottom an on-screen keyboard and faders, so the plugin can be
//! played without a MIDI controller.

use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;

use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, RichText};
use nih_plug_egui::{EguiState, create_egui_editor};

use crate::params::{BowLiftParam, FingeringParam, PolyphonyParam, StringsParams};
use crate::shared::{GuiEvent, Shared, Telemetry};
use crate::{INSTRUMENT, keyswitch_base};

mod fader;
mod instrument_view;
mod keyboard;
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
                instrument_view::show(ui, t, &mut state.view);
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
            // The PLAN.md budget for a solo instrument is 3% of one core.
            let color = if load > 3.0 {
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
        egui::ComboBox::from_id_salt("instrument")
            .selected_text("Cello")
            .width(130.0)
            .show_ui(ui, |ui| {
                let _ = ui.selectable_label(true, "Cello");
                for later in ["Violin", "Viola", "Double bass"] {
                    ui.add_enabled(false, egui::SelectableLabel::new(false, later))
                        .on_disabled_hover_text("Phase 5");
                }
            });
        ui.add_space(12.0);
        ui.label("Ensemble");
        egui::ComboBox::from_id_salt("ensemble")
            .selected_text("Solo")
            .width(110.0)
            .show_ui(ui, |ui| {
                let _ = ui.selectable_label(true, "Solo");
                ui.add_enabled(false, egui::SelectableLabel::new(false, "Section"))
                    .on_disabled_hover_text("Phase 6");
            });
        ui.add_space(24.0);

        ui.label("Bow lift");
        let live = shared.telemetry.bow_lift();
        let base = keyswitch_base(INSTRUMENT);
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
        enum_combo(
            ui,
            setter,
            &params.polyphony,
            &PolyphonyParam::ALL,
            PolyphonyParam::name,
            110.0,
        )
        .on_hover_text(
            "Double stops: a note held with another plays with it on the next string, \
             where one hand can play both. Otherwise it plays legato.",
        );
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
    let spec = &INSTRUMENT.strings[bowed];
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
    let spec = &INSTRUMENT.strings[bowed];
    let (lo, hi) = INSTRUMENT.force_limits[bowed].band(spec.impedance(), v, beta);
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
                ui.label(small(INSTRUMENT.strings[i].name.into()));
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
