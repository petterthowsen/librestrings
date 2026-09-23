//! The instrument, drawn lying down: scroll on the left, body on the right,
//! lowest string at the bottom. A placeholder graphic.
//!
//! Each string is drawn in parts: the stopped part (nut to finger), the
//! vibrating part (finger to bridge) and the afterlength (bridge to
//! tailpiece). The vibrating part shows Helmholtz motion: a corner travels
//! between finger and bridge along a parabolic envelope. It runs at a slow
//! display rate (like a stroboscope), with an amplitude from the string's
//! bridge force. The bow crosses the strings at its position β and moves
//! along its length with the bow velocity; the finger is a dot where the
//! vibrating length starts.

use std::sync::atomic::Ordering::Relaxed;

use nih_plug_egui::egui::{self, Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};

use crate::shared::Telemetry;

/// Helmholtz cycles per second on screen.
const DISPLAY_RATE: f32 = 1.2;
/// Bow hair length (m).
const BOW_LENGTH: f32 = 0.65;

#[derive(Default)]
pub struct ViewState {
    /// Helmholtz phase per string, 0–1.
    phase: [f32; 4],
    /// Displayed amplitude per string (0–1), smoothed.
    amplitude: [f32; 4],
    /// Bow position along its length (m from its middle).
    bow_offset: f32,
}

/// Where things are, in screen coordinates.
struct Geometry {
    rect: Rect,
    nut_x: f32,
    bridge_x: f32,
    tail_x: f32,
    center_y: f32,
    /// String spacing at the nut and at the bridge.
    spacing: (f32, f32),
}

impl Geometry {
    fn new(rect: Rect) -> Self {
        let x = |f: f32| rect.left() + f * rect.width();
        let h = rect.height().min(rect.width() * 0.45);
        Self {
            rect,
            nut_x: x(0.12),
            bridge_x: x(0.74),
            tail_x: x(0.90),
            center_y: rect.center().y,
            spacing: (0.035 * h, 0.07 * h),
        }
    }

    fn height(&self) -> f32 {
        self.rect.height().min(self.rect.width() * 0.45)
    }

    /// Height of string `i` (0 is the lowest) at `x`.
    fn string_y(&self, i: usize, x: f32) -> f32 {
        let t = ((x - self.nut_x) / (self.bridge_x - self.nut_x)).clamp(0.0, 1.0);
        let spacing = self.spacing.0 + t * (self.spacing.1 - self.spacing.0);
        self.center_y + (1.5 - i as f32) * spacing
    }

    /// Where the vibrating length starts for a string sounding `ratio` times
    /// its open pitch.
    fn finger_x(&self, ratio: f32) -> f32 {
        self.bridge_x - (self.bridge_x - self.nut_x) / ratio.max(1.0)
    }
}

pub fn show(ui: &mut egui::Ui, t: &Telemetry, state: &mut ViewState) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let g = Geometry::new(rect);
    let dt = ui.input(|i| i.stable_dt).min(0.1);

    draw_body(&painter, &g);

    let bowed = t.string.load(Relaxed) as usize % 4;
    let spec = t.instrument().spec();
    let mut bow_x = g.bridge_x - 0.1 * (g.bridge_x - g.nut_x);
    for i in 0..4 {
        let s = &t.strings[i];
        let ratio = s.frequency.load(Relaxed) / spec.strings[i].frequency;
        let finger_x = g.finger_x(ratio);
        if i == bowed {
            bow_x = g.bridge_x - s.beta.load(Relaxed) * (g.bridge_x - finger_x);
        }

        // Bridge force (N rms) to a display amplitude; a few newtons is loud.
        let level = s.level.load(Relaxed);
        let target = level / (level + 0.4);
        let a = &mut state.amplitude[i];
        *a += (target - *a) * (1.0 - (-dt / 0.08).exp());
        if *a > 0.002 {
            state.phase[i] = (state.phase[i] + DISPLAY_RATE * dt) % 1.0;
        }
        let max = 0.45 * g.spacing.1;
        draw_string(&painter, &g, i, finger_x, *a * max, state.phase[i]);
        if ratio > 1.003 {
            let y = g.string_y(i, finger_x);
            painter.circle_filled(pos2(finger_x, y), 5.0, Color32::from_rgb(230, 190, 160));
        }
    }

    // The bow moves along its length with the bow velocity and turns at its ends.
    let v = t.bow_velocity.load(Relaxed);
    state.bow_offset = (state.bow_offset + v * dt).clamp(-BOW_LENGTH / 2.0, BOW_LENGTH / 2.0);
    let contacts = std::array::from_fn(|i| t.strings[i].contact.load(Relaxed));
    draw_bow(&painter, &g, bowed, bow_x, state.bow_offset, contacts);
}

fn draw_body(painter: &egui::Painter, g: &Geometry) {
    let x = |f: f32| g.rect.left() + f * g.rect.width();
    let w = g.rect.width();
    let h = g.height();
    let cy = g.center_y;
    let wood = Color32::from_rgb(150, 80, 35);
    let edge = Color32::from_rgb(95, 45, 20);
    let ebony = Color32::from_rgb(28, 26, 26);

    // Body: upper bout, waist, lower bout.
    for (cx, rx, ry, color) in [
        (0.56, 0.115, 0.34, edge),
        (0.83, 0.145, 0.42, edge),
        (0.69, 0.10, 0.26, edge),
    ] {
        painter.add(Shape::ellipse_filled(
            pos2(x(cx), cy),
            vec2(rx * w + 3.0, ry * h + 3.0),
            color,
        ));
    }
    for (cx, rx, ry) in [(0.56, 0.115, 0.34), (0.83, 0.145, 0.42), (0.69, 0.10, 0.26)] {
        painter.add(Shape::ellipse_filled(
            pos2(x(cx), cy),
            vec2(rx * w, ry * h),
            wood,
        ));
    }
    // f-holes, either side of the bridge.
    for side in [-1.0, 1.0] {
        let y = cy + side * 0.2 * h;
        painter.line_segment(
            [
                pos2(g.bridge_x - 0.03 * w, y),
                pos2(g.bridge_x + 0.03 * w, y),
            ],
            Stroke::new(3.0_f32, Color32::from_rgb(40, 20, 10)),
        );
    }

    // Neck and pegbox, then the scroll.
    let neck = Rect::from_min_max(pos2(x(0.06), cy - 0.05 * h), pos2(x(0.46), cy + 0.05 * h));
    painter.rect_filled(neck, 4.0, edge);
    painter.circle_filled(pos2(x(0.045), cy), 0.07 * h, edge);
    painter.circle_filled(pos2(x(0.045), cy), 0.04 * h, wood);
    // Pegs.
    for (i, dx) in [0.07, 0.085, 0.1, 0.115].into_iter().enumerate() {
        let side = if i % 2 == 0 { -1.0 } else { 1.0 };
        painter.line_segment(
            [pos2(x(dx), cy), pos2(x(dx), cy + side * 0.13 * h)],
            Stroke::new(5.0_f32, ebony),
        );
    }

    // Fingerboard: widens toward the body.
    let board_end = x(0.64);
    let board = vec![
        pos2(g.nut_x, g.string_y(3, g.nut_x) - 8.0),
        pos2(board_end, g.string_y(3, board_end) - 10.0),
        pos2(board_end, g.string_y(0, board_end) + 10.0),
        pos2(g.nut_x, g.string_y(0, g.nut_x) + 8.0),
    ];
    painter.add(Shape::convex_polygon(board, ebony, Stroke::NONE));
    // Nut.
    painter.line_segment(
        [
            pos2(g.nut_x, g.string_y(3, g.nut_x) - 8.0),
            pos2(g.nut_x, g.string_y(0, g.nut_x) + 8.0),
        ],
        Stroke::new(3.0_f32, Color32::from_gray(220)),
    );

    // Tailpiece and endpin.
    let tail = vec![
        pos2(g.bridge_x + 0.04 * w, g.string_y(3, g.bridge_x) - 6.0),
        pos2(g.tail_x + 0.01 * w, cy - 0.04 * h),
        pos2(g.tail_x + 0.01 * w, cy + 0.04 * h),
        pos2(g.bridge_x + 0.04 * w, g.string_y(0, g.bridge_x) + 6.0),
    ];
    painter.add(Shape::convex_polygon(tail, ebony, Stroke::NONE));
    painter.line_segment(
        [
            pos2(g.tail_x + 0.06 * w, cy),
            pos2(g.rect.right() - 4.0, cy),
        ],
        Stroke::new(3.0_f32, Color32::from_gray(160)),
    );

    // Bridge.
    let bridge = Rect::from_center_size(pos2(g.bridge_x, cy), vec2(6.0, 4.0 * g.spacing.1 + 16.0));
    painter.rect_filled(bridge, 2.0, Color32::from_rgb(225, 200, 150));
}

/// One string: stopped part, vibrating part and afterlength.
fn draw_string(
    painter: &egui::Painter,
    g: &Geometry,
    i: usize,
    finger_x: f32,
    amplitude: f32,
    phase: f32,
) {
    // Lower strings are thicker.
    let width = 2.6 - 0.4 * i as f32;
    let color = Color32::from_gray(200);
    let dim = Color32::from_gray(140);
    let y = |x: f32| g.string_y(i, x);

    // Stopped part and afterlength don't vibrate.
    if finger_x > g.nut_x + 0.5 {
        painter.line_segment(
            [pos2(g.nut_x, y(g.nut_x)), pos2(finger_x, y(finger_x))],
            Stroke::new(width, dim),
        );
    }
    let tail_end = g.tail_x - 0.1 * (g.tail_x - g.bridge_x);
    painter.line_segment(
        [
            pos2(g.bridge_x, y(g.bridge_x)),
            pos2(tail_end, g.center_y + (1.5 - i as f32) * 5.0),
        ],
        Stroke::new(width, dim),
    );

    // Vibrating part: the Helmholtz corner goes from the finger to the bridge
    // on one side of the envelope and back on the other.
    let (a, b) = (finger_x, g.bridge_x);
    let point = |x: f32, dy: f32| pos2(x, y(x) + dy);
    if amplitude < 0.3 {
        painter.line_segment([point(a, 0.0), point(b, 0.0)], Stroke::new(width, color));
        return;
    }
    let (u, side) = if phase < 0.5 {
        (2.0 * phase, 1.0)
    } else {
        (2.0 - 2.0 * phase, -1.0)
    };
    let corner = a + u * (b - a);
    let height = side * amplitude * 4.0 * u * (1.0 - u);

    // Faint envelope, as the eye sees a fast-vibrating string.
    let envelope: Vec<Pos2> = (0..=24)
        .map(|k| {
            let s = k as f32 / 24.0;
            point(a + s * (b - a), amplitude * 4.0 * s * (1.0 - s))
        })
        .collect();
    let mirrored: Vec<Pos2> = envelope
        .iter()
        .map(|p| pos2(p.x, 2.0 * y(p.x) - p.y))
        .collect();
    let faint = Stroke::new(1.0_f32, Color32::from_white_alpha(40));
    painter.add(Shape::line(envelope, faint));
    painter.add(Shape::line(mirrored, faint));

    painter.add(Shape::line(
        vec![point(a, 0.0), point(corner, height), point(b, 0.0)],
        Stroke::new(width, color),
    ));
}

fn draw_bow(
    painter: &egui::Painter,
    g: &Geometry,
    bowed: usize,
    x: f32,
    offset: f32,
    contacts: [f32; 4],
) {
    let contact = contacts[bowed];
    let h = g.height();
    let y = g.string_y(bowed, x);
    // Screen length of the hair; the offset moves it along its length.
    let length = 1.1 * h;
    let shift = offset / BOW_LENGTH * length;
    let top = y - length / 2.0 + shift;
    let bottom = y + length / 2.0 + shift;
    // Lifted: the bow is drawn off to the side a little.
    let lift = (1.0 - contact.clamp(0.0, 1.0)) * 10.0;
    let hair_x = x + lift;
    let stick_x = hair_x + 7.0;

    let hair = Color32::from_rgba_unmultiplied(235, 230, 210, 220);
    let stick = Color32::from_rgb(90, 50, 30);
    painter.line_segment(
        [pos2(hair_x, top), pos2(hair_x, bottom)],
        Stroke::new(4.0_f32, hair),
    );
    // The stick bows toward the hair in the middle.
    let stick_points: Vec<Pos2> = (0..=16)
        .map(|k| {
            let s = k as f32 / 16.0;
            let bend = 4.0 * s * (1.0 - s) * 4.0;
            pos2(stick_x - bend, top + s * (bottom - top))
        })
        .collect();
    painter.add(Shape::line(stick_points, Stroke::new(3.5_f32, stick)));
    // Frog and tip.
    painter.rect_filled(
        Rect::from_center_size(pos2(stick_x + 2.0, bottom - 10.0), vec2(12.0, 22.0)),
        2.0,
        Color32::from_gray(20),
    );
    painter.rect_filled(
        Rect::from_center_size(pos2(stick_x - 1.0, top + 3.0), vec2(9.0, 8.0)),
        2.0,
        Color32::from_gray(225),
    );

    // Every string the bow touches: a double stop, or both in a crossing.
    for (i, contact) in contacts.into_iter().enumerate() {
        if contact > 0.01 {
            let alpha = (contact.min(1.0) * 200.0) as u8;
            painter.circle_filled(
                pos2(x, g.string_y(i, x)),
                6.0,
                Color32::from_rgba_unmultiplied(240, 170, 60, alpha),
            );
        }
    }
}
