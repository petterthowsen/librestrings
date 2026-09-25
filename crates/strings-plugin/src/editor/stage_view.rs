//! The stage view (docs/SECTIONS.md B2): the room from above, with every
//! instance's section and the mics on it, and beside it the section's and the
//! room's settings.
//!
//! Each instance's section is a box, colored by instrument, with its players
//! drawn where they sit; this instance's is drawn on top and outlined. Drag a
//! box to move it, or its sides and corners to change its size; drag the mics
//! to move them. Another instance's section is moved through the stage file
//! (`sync`), and that instance takes the move over. The audience is at the
//! bottom, as seen from the hall.

use std::sync::atomic::Ordering::Relaxed;

use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, Align2, Color32, CursorIcon, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke,
    StrokeKind, pos2, vec2,
};
use strings_dsp::{Absorption, Placement, RoomPreset, StageSettings};

use crate::layout::{
    DEFAULT_STAGE, MIN_SIZE, clamp_placement, default_name, mic_bounds, section_bounds,
};
use crate::params::{InstrumentParam, StringsParams};
use crate::sync::{Link, Section};

/// Positions snap to this (m).
const SNAP: f32 = 0.1;
/// How close to a box's side the pointer grabs the side (px).
const GRAB: f32 = 6.0;
/// The mics' angle off the centre line (as `strings_dsp::stage`).
const MIC_ANGLE: f32 = 55.0 * std::f32::consts::PI / 180.0;

#[derive(Default)]
pub struct StageState {
    drag: Option<Drag>,
    /// The metres the view shows, held while dragging so it doesn't rescale
    /// under the pointer.
    frame: Option<Frame>,
    /// Text being edited: the section's name and the stage's.
    name: String,
    stage: String,
}

/// What the pointer is on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Own,
    Other { slot: usize, owner: u64 },
    Mics,
}

/// The sides of a box being dragged; none is the whole box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Sides {
    left: bool,
    right: bool,
    /// Toward the audience (down on screen).
    front: bool,
    back: bool,
}

impl Sides {
    fn any(self) -> bool {
        self.left || self.right || self.front || self.back
    }

    fn cursor(self, dragging: bool) -> CursorIcon {
        let across = self.left || self.right;
        let along = self.front || self.back;
        match (across, along) {
            (true, true) if (self.left && self.back) || (self.right && self.front) => {
                CursorIcon::ResizeNwSe
            }
            (true, true) => CursorIcon::ResizeNeSw,
            (true, false) => CursorIcon::ResizeHorizontal,
            (false, true) => CursorIcon::ResizeVertical,
            (false, false) if dragging => CursorIcon::Grabbing,
            (false, false) => CursorIcon::Grab,
        }
    }
}

struct Drag {
    target: Target,
    sides: Sides,
    /// Where the drag started (m).
    start: (f32, f32),
    placement: Placement,
    /// The mics' x and distance when it started.
    mics: (f32, f32),
}

/// The part of the room shown (m): across, and from the front of the view
/// to the back wall.
#[derive(Clone, Copy, PartialEq)]
struct Frame {
    x: (f32, f32),
    y: (f32, f32),
}

/// Metres to the screen and back.
struct Transform {
    /// The screen point of (0, top).
    origin: Pos2,
    top: f32,
    /// Pixels per metre.
    scale: f32,
}

impl Transform {
    fn new(rect: Rect, frame: Frame) -> Self {
        let (w, h) = (frame.x.1 - frame.x.0, frame.y.1 - frame.y.0);
        let area = rect.shrink(20.0);
        let scale = (area.width() / w).min(area.height() / h).max(1.0);
        let centre = area.center();
        let mid_x = 0.5 * (frame.x.0 + frame.x.1);
        Self {
            origin: pos2(centre.x - mid_x * scale, centre.y - 0.5 * h * scale),
            top: frame.y.1,
            scale,
        }
    }

    fn pos(&self, x: f32, y: f32) -> Pos2 {
        pos2(
            self.origin.x + x * self.scale,
            self.origin.y + (self.top - y) * self.scale,
        )
    }

    fn metres(&self, p: Pos2) -> (f32, f32) {
        (
            (p.x - self.origin.x) / self.scale,
            self.top - (p.y - self.origin.y) / self.scale,
        )
    }

    fn rect(&self, p: Placement) -> Rect {
        let rect = Rect::from_two_pos(
            self.pos(p.x - 0.5 * p.width, p.y - 0.5 * p.depth),
            self.pos(p.x + 0.5 * p.width, p.y + 0.5 * p.depth),
        );
        // Small enough to lose: keep it big enough to grab.
        Rect::from_center_size(rect.center(), rect.size().max(vec2(8.0, 8.0)))
    }
}

/// A color per instrument, to tell the sections apart.
pub fn instrument_color(i: InstrumentParam) -> Color32 {
    match i {
        InstrumentParam::Violin => Color32::from_rgb(232, 176, 92),
        InstrumentParam::Viola => Color32::from_rgb(120, 196, 150),
        InstrumentParam::Cello => Color32::from_rgb(108, 160, 232),
        InstrumentParam::Bass => Color32::from_rgb(190, 136, 226),
    }
}

/// `v` kept between `lo` and `hi` (`hi` wins if they cross, unlike `clamp`,
/// which panics).
fn within(v: f32, lo: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}

fn snap(v: f32) -> f32 {
    (v / SNAP).round() * SNAP
}

/// This instance's section, as the view draws it.
fn own_section(params: &StringsParams) -> Section {
    let instrument = params.instrument.value();
    let players = params.players.value().max(1) as u32;
    let name = params.layout.name();
    Section {
        slot: usize::MAX,
        owner: 0,
        name: if name.is_empty() {
            default_name(instrument, players)
        } else {
            name
        },
        instrument,
        players,
        staged: params.stage.value(),
        placement: params.layout.placement(),
    }
}

/// The other instances' sections on the stage.
pub fn others(link: &Link) -> Vec<Section> {
    link.registry()
        .map(|r| r.sections(link.id))
        .unwrap_or_default()
}

/// The part of the room to show: all of it across, and from the back wall
/// to a little in front of the mics (the hall behind them is empty).
fn frame(settings: &StageSettings) -> Frame {
    let room = settings.room.room();
    let front = room.back - room.length;
    let near = -(settings.mic_distance + 1.5).max(3.0);
    Frame {
        x: (-0.5 * room.width, 0.5 * room.width),
        y: (near.max(front), room.back),
    }
}

pub fn view(
    ui: &mut egui::Ui,
    params: &StringsParams,
    link: &Link,
    others: &[Section],
    state: &mut StageState,
) {
    let layout = &params.layout;
    let settings = layout.settings();
    let room = settings.room.room();
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    let ink = visuals.text_color();

    let wanted = frame(&settings);
    let frame = match (state.drag.is_some(), state.frame) {
        (true, Some(f)) => f,
        _ => wanted,
    };
    state.frame = Some(frame);
    let tf = Transform::new(rect, frame);

    // The room: the hall's floor, the stage, a metre grid on it, the walls.
    let walls = Rect::from_two_pos(
        tf.pos(-0.5 * room.width, room.back - room.length),
        tf.pos(0.5 * room.width, room.back),
    );
    painter.rect_filled(walls, 0.0, visuals.extreme_bg_color);
    let stage = Rect::from_two_pos(
        tf.pos(-0.5 * room.width, 0.0),
        tf.pos(0.5 * room.width, room.back),
    );
    painter.rect_filled(stage, 0.0, ink.gamma_multiply(0.05));
    let grid = Stroke::new(1.0_f32, ink.gamma_multiply(0.05));
    let half = (0.5 * room.width).floor() as i32;
    for i in -half..=half {
        let x = i as f32;
        painter.line_segment([tf.pos(x, 0.0), tf.pos(x, room.back)], grid);
    }
    for j in 1..=room.back.floor() as i32 {
        let y = j as f32;
        painter.line_segment(
            [tf.pos(-0.5 * room.width, y), tf.pos(0.5 * room.width, y)],
            grid,
        );
    }
    painter.extend(Shape::dashed_line(
        &[tf.pos(0.0, room.back), tf.pos(0.0, frame.y.0)],
        Stroke::new(1.0_f32, ink.gamma_multiply(0.12)),
        6.0,
        6.0,
    ));
    painter.line_segment(
        [
            tf.pos(-0.5 * room.width, 0.0),
            tf.pos(0.5 * room.width, 0.0),
        ],
        Stroke::new(1.5_f32, ink.gamma_multiply(0.4)),
    );
    painter.text(
        tf.pos(-0.5 * room.width, 0.0) + vec2(6.0, -4.0),
        Align2::LEFT_BOTTOM,
        "Front of the stage",
        FontId::proportional(10.0),
        ink.gamma_multiply(0.45),
    );
    painter.rect_stroke(
        walls,
        0.0,
        Stroke::new(2.0_f32, ink.gamma_multiply(0.45)),
        StrokeKind::Outside,
    );
    let title = format!(
        "{} · {:.0} × {:.0} m · {} absorption",
        settings.room.name(),
        room.width,
        room.length,
        settings.absorption.name().to_lowercase()
    );
    painter.text(
        rect.left_top() + vec2(8.0, 6.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(11.0),
        ink.gamma_multiply(0.6),
    );
    scale_bar(&painter, &tf, rect, ink);

    // Draw order: the others, then this one on top; the mics above all.
    let own = own_section(params);
    let mics = tf.pos(settings.mic_x, -settings.mic_distance);

    // What the pointer is on, topmost first.
    let hit = |pos: Pos2| -> Option<(Target, Sides)> {
        if pos.distance(mics) < 12.0 {
            return Some((Target::Mics, Sides::default()));
        }
        let boxes =
            std::iter::once((Target::Own, own.placement)).chain(others.iter().rev().map(|s| {
                (
                    Target::Other {
                        slot: s.slot,
                        owner: s.owner,
                    },
                    s.placement,
                )
            }));
        for (target, placement) in boxes {
            if let Some(sides) = sides_at(tf.rect(placement), pos) {
                return Some((target, sides));
            }
        }
        None
    };

    let pointer = response.hover_pos();
    if response.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        && let Some((target, sides)) = hit(origin)
    {
        let placement = match target {
            Target::Own => own.placement,
            Target::Other { owner, .. } => others
                .iter()
                .find(|s| s.owner == owner)
                .map_or(own.placement, |s| s.placement),
            Target::Mics => own.placement,
        };
        state.drag = Some(Drag {
            target,
            sides,
            start: tf.metres(origin),
            placement,
            mics: (settings.mic_x, settings.mic_distance),
        });
    }
    let active = state.drag.as_ref().map(|d| (d.target, d.sides));
    if let Some(drag) = &state.drag
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (x, y) = tf.metres(pos);
        let (dx, dy) = (x - drag.start.0, y - drag.start.1);
        match drag.target {
            Target::Mics => {
                let ((d0, d1), (x0, x1)) = mic_bounds(room);
                // Not past the front of the view, which is held while dragging.
                let d1 = d1.min(-frame.y.0 - 0.5);
                layout.set_settings(StageSettings {
                    mic_x: within(snap(drag.mics.0 + dx), x0, x1),
                    mic_distance: within(snap(drag.mics.1 - dy), d0, d1),
                    ..settings
                });
            }
            Target::Own => {
                layout.set_placement(dragged(drag.placement, drag.sides, dx, dy, room));
            }
            Target::Other { slot, owner } => {
                if let Some(registry) = link.registry() {
                    let p = dragged(drag.placement, drag.sides, dx, dy, room);
                    registry.move_section(slot, owner, p);
                }
            }
        }
    }
    if response.drag_stopped() {
        state.drag = None;
    }
    let hovered = if active.is_some() {
        active
    } else {
        pointer.and_then(hit)
    };
    if let Some((target, sides)) = hovered {
        let cursor = if target == Target::Mics {
            if active.is_some() {
                CursorIcon::Grabbing
            } else {
                CursorIcon::Grab
            }
        } else {
            sides.cursor(active.is_some())
        };
        ui.ctx().set_cursor_icon(cursor);
    }

    // The sections, as they are now (a dragged other one from the stage file
    // next frame; drawn from the drag meanwhile, so it doesn't lag).
    let mut dragged_other = None;
    if let (Some(drag), Some(pos)) = (&state.drag, response.interact_pointer_pos())
        && let Target::Other { owner, .. } = drag.target
    {
        let (x, y) = tf.metres(pos);
        let p = dragged(
            drag.placement,
            drag.sides,
            x - drag.start.0,
            y - drag.start.1,
            room,
        );
        dragged_other = Some((owner, p));
    }
    for s in others {
        let mut s = s.clone();
        if let Some((owner, p)) = dragged_other
            && owner == s.owner
        {
            s.placement = p;
        }
        let target = Target::Other {
            slot: s.slot,
            owner: s.owner,
        };
        let sides = hovered.filter(|h| h.0 == target).map(|h| h.1);
        draw_section(&painter, &tf, &s, false, sides, &visuals);
    }
    let sides = hovered.filter(|h| h.0 == Target::Own).map(|h| h.1);
    draw_section(&painter, &tf, &own, true, sides, &visuals);
    draw_mics(
        &painter,
        mics,
        hovered.is_some_and(|h| h.0 == Target::Mics),
        ink,
    );

    // What the pointer is on, in numbers.
    let readout = hovered.and_then(|(target, _)| match target {
        Target::Mics => Some(format!(
            "Mics · {:.1} m from the stage · {:+.1} m across",
            settings.mic_distance, settings.mic_x
        )),
        Target::Own => Some(describe(&own)),
        Target::Other { owner, .. } => others.iter().find(|s| s.owner == owner).map(|s| {
            let mut s = s.clone();
            if let Some((_, p)) = dragged_other.filter(|d| d.0 == owner) {
                s.placement = p;
            }
            describe(&s)
        }),
    });
    if let Some(text) = readout {
        painter.text(
            rect.right_bottom() - vec2(8.0, 6.0),
            Align2::RIGHT_BOTTOM,
            text,
            FontId::monospace(11.0),
            ink.gamma_multiply(0.8),
        );
    }
}

/// A section in words and numbers.
fn describe(s: &Section) -> String {
    let p = s.placement;
    let players = if s.players > 1 {
        format!("{} players", s.players)
    } else {
        "solo".into()
    };
    format!(
        "{} · {players} · x {:+.1} y {:.1} · {:.1} × {:.1} m",
        s.name, p.x, p.y, p.width, p.depth
    )
}

/// The sides of `rect` near `pos`, none for its inside, or `None` if `pos` is
/// off it.
fn sides_at(rect: Rect, pos: Pos2) -> Option<Sides> {
    let m = GRAB
        .min(rect.width() / 4.0)
        .min(rect.height() / 4.0)
        .max(2.0);
    if !rect.expand(m).contains(pos) {
        return None;
    }
    Some(Sides {
        left: (pos.x - rect.left()).abs() <= m,
        right: (pos.x - rect.right()).abs() <= m,
        back: (pos.y - rect.top()).abs() <= m,
        front: (pos.y - rect.bottom()).abs() <= m,
    })
}

/// `from` moved by (dx, dy) m, or with the dragged sides moved, kept on the
/// stage and snapped.
fn dragged(from: Placement, sides: Sides, dx: f32, dy: f32, room: strings_dsp::Room) -> Placement {
    let ((x0, x1), (y0, y1)) = section_bounds(room);
    let from = clamp_placement(from, room);
    let (mut l, mut r) = (from.x - 0.5 * from.width, from.x + 0.5 * from.width);
    let (mut f, mut b) = (from.y - 0.5 * from.depth, from.y + 0.5 * from.depth);
    if sides.any() {
        if sides.left {
            l = within(snap(l + dx), x0, r - MIN_SIZE);
        }
        if sides.right {
            r = within(snap(r + dx), l + MIN_SIZE, x1);
        }
        if sides.front {
            f = within(snap(f + dy), y0, b - MIN_SIZE);
        }
        if sides.back {
            b = within(snap(b + dy), f + MIN_SIZE, y1);
        }
    } else {
        let (w, d) = (r - l, b - f);
        l = within(snap(l + dx), x0, x1 - w);
        r = l + w;
        f = within(snap(f + dy), y0, y1 - d);
        b = f + d;
    }
    Placement {
        x: 0.5 * (l + r),
        y: 0.5 * (f + b),
        width: r - l,
        depth: b - f,
    }
}

fn draw_section(
    painter: &egui::Painter,
    tf: &Transform,
    s: &Section,
    own: bool,
    hovered: Option<Sides>,
    visuals: &egui::Visuals,
) {
    let color = instrument_color(s.instrument);
    let rect = tf.rect(s.placement);
    let (fill, line) = match (s.staged, own) {
        (false, _) => (0.06, 0.45),
        (true, true) => (0.28, 1.0),
        (true, false) => (0.16, 0.7),
    };
    painter.rect_filled(rect, 4.0, color.gamma_multiply(fill));
    let width: f32 = if own { 2.0 } else { 1.0 };
    let stroke = Stroke::new(width, color.gamma_multiply(line));
    if s.staged {
        painter.rect_stroke(rect, 4.0, stroke, StrokeKind::Inside);
    } else {
        // Dry: not heard from the stage.
        let r = rect.shrink(0.5);
        let corners = [
            r.left_top(),
            r.right_top(),
            r.right_bottom(),
            r.left_bottom(),
            r.left_top(),
        ];
        painter.extend(Shape::dashed_line(&corners, stroke, 5.0, 4.0));
    }

    // The players where they sit (without the stage's small offsets).
    let n = s.players as usize;
    let dot = (0.18 * tf.scale).clamp(1.5, 3.5);
    for i in 0..n {
        let (x, y) = s.placement.seat(n, i);
        painter.circle_filled(tf.pos(x, y), dot, color.gamma_multiply(0.55 * line));
    }

    let text = visuals.strong_text_color();
    let c = rect.center();
    painter.text(
        c - vec2(0.0, 7.0),
        Align2::CENTER_CENTER,
        &s.name,
        FontId::proportional(13.0),
        if s.staged {
            text
        } else {
            text.gamma_multiply(0.6)
        },
    );
    let detail = match (s.staged, s.players) {
        (false, _) => "dry".to_string(),
        (true, 1) => "solo".to_string(),
        (true, n) => format!("{n} players"),
    };
    painter.text(
        c + vec2(0.0, 8.0),
        Align2::CENTER_CENTER,
        detail,
        FontId::proportional(10.0),
        visuals.weak_text_color(),
    );

    // Resize handles on the sides, on the box under the pointer.
    if let Some(sides) = hovered {
        let handle = |at: Pos2, size: egui::Vec2, on: bool| {
            let color = if on { text } else { color.gamma_multiply(0.9) };
            painter.rect_filled(Rect::from_center_size(at, size), 2.0, color);
        };
        let (across, along) = (vec2(4.0, 14.0), vec2(14.0, 4.0));
        handle(pos2(rect.left(), c.y), across, sides.left);
        handle(pos2(rect.right(), c.y), across, sides.right);
        handle(pos2(c.x, rect.top()), along, sides.back);
        handle(pos2(c.x, rect.bottom()), along, sides.front);
    }
}

/// The mic pair: two capsules, each aimed 55° off the centre line.
fn draw_mics(painter: &egui::Painter, at: Pos2, hovered: bool, ink: Color32) {
    let color = if hovered {
        ink
    } else {
        ink.gamma_multiply(0.75)
    };
    if hovered {
        painter.circle_filled(at, 12.0, ink.gamma_multiply(0.08));
    }
    for side in [-1.0f32, 1.0] {
        let capsule = at + vec2(side * 4.5, 0.0);
        let aim = vec2(side * MIC_ANGLE.sin(), -MIC_ANGLE.cos());
        painter.line_segment([capsule, capsule + 11.0 * aim], Stroke::new(1.5_f32, color));
        painter.circle_filled(capsule, 3.0, color);
    }
    painter.text(
        at + vec2(0.0, 8.0),
        Align2::CENTER_TOP,
        "Mics",
        FontId::proportional(10.0),
        color,
    );
}

/// A bar a round number of metres long, bottom left.
fn scale_bar(painter: &egui::Painter, tf: &Transform, rect: Rect, ink: Color32) {
    let metres = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .find(|m| m * tf.scale >= 40.0)
        .unwrap_or(10.0);
    let start = rect.left_bottom() + vec2(10.0, -10.0);
    let end = start + vec2(metres * tf.scale, 0.0);
    let stroke = Stroke::new(1.0_f32, ink.gamma_multiply(0.6));
    painter.line_segment([start, end], stroke);
    for p in [start, end] {
        painter.line_segment([p, p - vec2(0.0, 4.0)], stroke);
    }
    painter.text(
        end + vec2(6.0, 0.0),
        Align2::LEFT_CENTER,
        format!("{metres:.0} m"),
        FontId::proportional(10.0),
        ink.gamma_multiply(0.6),
    );
}

/// The settings beside the stage: this section, the room, and the stage it
/// shares.
pub fn panel(
    ui: &mut egui::Ui,
    params: &StringsParams,
    setter: &ParamSetter,
    link: &Link,
    others: &[Section],
    state: &mut StageState,
) {
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| settings(ui, params, setter, link, others, state));
}

fn settings(
    ui: &mut egui::Ui,
    params: &StringsParams,
    setter: &ParamSetter,
    link: &Link,
    others: &[Section],
    state: &mut StageState,
) {
    let layout = &params.layout;
    let heading = |ui: &mut egui::Ui, text: &str| {
        ui.add_space(6.0);
        ui.label(RichText::new(text).strong());
        ui.add_space(2.0);
    };

    heading(ui, "This section");
    let own = own_section(params);
    egui::Grid::new("stage-section")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Name").weak());
            let id = egui::Id::new("section-name");
            if !ui.memory(|m| m.has_focus(id)) {
                state.name = layout.name();
            }
            let hint = default_name(own.instrument, own.players);
            let edit = egui::TextEdit::singleline(&mut state.name)
                .id(id)
                .hint_text(hint)
                .desired_width(130.0);
            if ui.add(edit).changed() {
                layout.set_name(&state.name);
            }
            ui.end_row();

            ui.label(RichText::new("Stage").weak());
            let mut on = params.stage.value();
            let text = if on { "In the room" } else { "Dry" };
            if ui
                .checkbox(&mut on, text)
                .on_hover_text(
                    "In the room: the players sit on the stage, picked up by the stereo \
                     mic pair, with early reflections (the late reverb is left to your \
                     reverb). Dry: the players' mono sum.",
                )
                .changed()
            {
                setter.begin_set_parameter(&params.stage);
                setter.set_parameter(&params.stage, on);
                setter.end_set_parameter(&params.stage);
            }
            ui.end_row();

            let room = layout.settings().room.room();
            let mut p = layout.placement();
            let mut changed = false;
            let mut row = |ui: &mut egui::Ui, name: &str, value: &mut f32, help: &str| {
                ui.label(RichText::new(name).weak());
                changed |= metres(ui, value).on_hover_text(help).changed();
                ui.end_row();
            };
            row(
                ui,
                "Across",
                &mut p.x,
                "To the audience's right (m); 0 is the middle",
            );
            row(ui, "Upstage", &mut p.y, "From the front of the stage (m)");
            row(
                ui,
                "Width",
                &mut p.width,
                "Of the area the players fill (m)",
            );
            row(
                ui,
                "Depth",
                &mut p.depth,
                "Of the area the players fill (m)",
            );
            if changed {
                layout.set_placement(clamp_placement(p, room));
            }
        });

    ui.add_space(4.0);
    ui.separator();
    heading(ui, "Room");
    ui.label(
        RichText::new("Shared by every instance on the stage")
            .small()
            .weak(),
    );
    let mut s = layout.settings();
    let before = s;
    egui::Grid::new("stage-room")
        .num_columns(2)
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Room").weak());
            combo(ui, "room", &mut s.room, &RoomPreset::ALL, RoomPreset::name);
            ui.end_row();
            ui.label(RichText::new("Walls").weak());
            combo(
                ui,
                "absorption",
                &mut s.absorption,
                &Absorption::ALL,
                |a| match a {
                    Absorption::Low => "Low absorption",
                    Absorption::Medium => "Medium absorption",
                    Absorption::High => "High absorption",
                },
            )
            .on_hover_text("How much the walls absorb");
            ui.end_row();
            ui.label(RichText::new("Reflections").weak());
            let mut percent = 100.0 * s.reflections;
            if ui
                .add(
                    egui::DragValue::new(&mut percent)
                        .speed(1.0)
                        .range(0.0..=100.0)
                        .suffix(" %"),
                )
                .on_hover_text("Level of the early reflections")
                .changed()
            {
                s.reflections = percent / 100.0;
            }
            ui.end_row();
            let ((d0, d1), (x0, x1)) = mic_bounds(s.room.room());
            ui.label(RichText::new("Mics").weak());
            metres(ui, &mut s.mic_distance)
                .on_hover_text("In front of the stage (m): close to far. Or drag them.");
            ui.end_row();
            ui.label(RichText::new("Mics across").weak());
            metres(ui, &mut s.mic_x).on_hover_text("To the audience's right (m)");
            ui.end_row();
            s.mic_distance = within(s.mic_distance, d0, d1);
            s.mic_x = within(s.mic_x, x0, x1);
        });
    if s != before {
        layout.set_settings(s);
    }

    ui.add_space(4.0);
    ui.separator();
    heading(ui, "Shared stage");
    ui.horizontal(|ui| {
        ui.label(RichText::new("Name").weak());
        let id = egui::Id::new("stage-name");
        if !ui.memory(|m| m.has_focus(id)) {
            state.stage = layout.stage();
        }
        let edit = egui::TextEdit::singleline(&mut state.stage)
            .id(id)
            .hint_text(DEFAULT_STAGE)
            .desired_width(130.0);
        let response = ui.add(edit).on_hover_text(
            "Instances with the same stage name share it: give another project's \
             instances another name to keep them apart.",
        );
        if response.lost_focus() {
            let name = state.stage.trim();
            layout.set_stage(if name.is_empty() { DEFAULT_STAGE } else { name });
        }
    });
    ui.add_space(4.0);
    match link.error() {
        Some(error) => {
            ui.label(RichText::new(error).small().color(Color32::YELLOW));
        }
        None if link.registry().is_none() => {
            ui.label(RichText::new("Joining the stage…").small().weak());
        }
        None => {
            let text = match others.len() {
                0 => "No other instances on it".to_string(),
                1 => "1 other instance on it".to_string(),
                n => format!("{n} other instances on it"),
            };
            ui.label(RichText::new(text).small().weak());
        }
    }
    ui.add_space(6.0);
    ui.label(
        RichText::new(
            "Drag a section to move it, or its sides to resize it; any instance's \
             section can be moved from here. Drag the mics to move them.",
        )
        .small()
        .weak(),
    );
}

/// A number field in metres.
fn metres(ui: &mut egui::Ui, value: &mut f32) -> egui::Response {
    ui.add(
        egui::DragValue::new(value)
            .speed(0.05)
            .fixed_decimals(1)
            .suffix(" m"),
    )
}

fn combo<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    all: &[T],
    name: fn(T) -> &'static str,
) -> egui::Response {
    egui::ComboBox::from_id_salt(id)
        .selected_text(name(*value))
        .width(130.0)
        .show_ui(ui, |ui| {
            for &v in all {
                if ui.selectable_label(v == *value, name(v)).clicked() {
                    *value = v;
                }
            }
        })
        .response
}

/// The view toggle: the instrument or the stage.
pub fn toggle(ui: &mut egui::Ui, params: &StringsParams) {
    let on = params.stage_view.load(Relaxed);
    if ui
        .add(egui::Button::new("Stage").selected(on))
        .on_hover_text("Where every instance's section sits, and the room they share")
        .clicked()
    {
        params.stage_view.store(!on, Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: RoomPreset = RoomPreset::ChamberHall;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn moving_a_section_keeps_its_size_and_stays_on_stage() {
        let room = ROOM.room();
        let from = Placement::CELLOS;
        let p = dragged(from, Sides::default(), 1.23, -0.5, room);
        assert!(close(p.width, from.width) && close(p.depth, from.depth));
        assert!(close(p.x, from.x + 1.2) && close(p.y, from.y - 0.5));
        // Far past the right wall and the front of the stage: stopped there.
        let p = dragged(from, Sides::default(), 50.0, -50.0, room);
        let ((_, x1), (y0, _)) = section_bounds(room);
        assert!(close(p.x + 0.5 * p.width, x1));
        assert!(close(p.y - 0.5 * p.depth, y0));
        assert!(close(p.width, from.width));
    }

    #[test]
    fn dragging_a_side_moves_only_that_side() {
        let room = ROOM.room();
        let from = Placement::CELLOS;
        let left = Sides {
            left: true,
            ..Sides::default()
        };
        let p = dragged(from, left, -1.0, 0.0, room);
        assert!(close(p.x + 0.5 * p.width, from.x + 0.5 * from.width));
        assert!(close(p.width, from.width + 1.0));
        assert_eq!((p.y, p.depth), (from.y, from.depth));
        // Not past the other side.
        let p = dragged(from, left, 10.0, 0.0, room);
        assert!(close(p.width, MIN_SIZE));
        let corner = Sides {
            right: true,
            back: true,
            ..Sides::default()
        };
        let p = dragged(from, corner, 0.5, 0.5, room);
        assert!(close(p.width, from.width + 0.5) && close(p.depth, from.depth + 0.5));
        assert!(close(p.x - 0.5 * p.width, from.x - 0.5 * from.width));
        assert!(close(p.y - 0.5 * p.depth, from.y - 0.5 * from.depth));
    }

    #[test]
    fn the_pointer_finds_sides_and_corners() {
        let rect = Rect::from_min_max(pos2(100.0, 100.0), pos2(200.0, 160.0));
        assert_eq!(sides_at(rect, pos2(150.0, 130.0)), Some(Sides::default()));
        assert_eq!(
            sides_at(rect, pos2(98.0, 130.0)),
            Some(Sides {
                left: true,
                ..Sides::default()
            })
        );
        let corner = sides_at(rect, pos2(203.0, 97.0)).unwrap();
        assert!(corner.right && corner.back && !corner.left && !corner.front);
        assert_eq!(sides_at(rect, pos2(220.0, 130.0)), None);
    }

    #[test]
    fn the_transform_goes_both_ways() {
        let frame = frame(&StageSettings::default());
        let tf = Transform::new(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0)), frame);
        let (x, y) = tf.metres(tf.pos(3.5, 2.0));
        assert!(close(x, 3.5) && close(y, 2.0));
        // Upstage is up the screen.
        assert!(tf.pos(0.0, 5.0).y < tf.pos(0.0, 1.0).y);
    }
}
