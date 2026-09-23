//! A small text score format for driving the performer offline.
//!
//! One event per line; `#` at the start of a word starts a comment. Times are in beats (see `tempo`).
//!
//! ```text
//! tempo 72                 # beats per minute (default 60: beats are seconds)
//! 0    dyn 0.6             # dynamics 0–1 (also: vib, pressure; pressure 0.5 is normal)
//! 0    bow on              # bow lift: on (stops on the string) | off (lifts, the default)
//! 0    fingering mid       # nut | mid | bridge
//! 0    poly on             # on: overlapping notes are double stops where they can be
//! 0    note C3 1.0 80      # note, length in beats, velocity 1–127 (default 64)
//! 1    on D3 90            # note on (velocity optional) ...
//! 2.1  off D3              # ... and off; overlapping notes play legato
//! ```
//!
//! Notes are named with an octave, C4 being middle C (MIDI 60): `C2`, `F#3`, `Bb2`.

use strings_dsp::{BowLift, Fingering, Polyphony};

#[derive(Clone, Copy, Debug)]
pub enum Event {
    On(u8, f32),
    Off(u8),
    Dynamics(f32),
    Vibrato(f32),
    Pressure(f32),
    BowLift(BowLift),
    Fingering(Fingering),
    Polyphony(Polyphony),
}

/// A fingering mode by name: nut, mid or bridge.
pub fn fingering(name: &str) -> Result<Fingering, String> {
    match name {
        "nut" => Ok(Fingering::NutAndOpen),
        "mid" => Ok(Fingering::Mid),
        "bridge" => Ok(Fingering::Bridge),
        _ => Err("expected nut, mid or bridge".into()),
    }
}

/// Events with their times in seconds, sorted by time (stable, so events at the
/// same time keep their order in the file, with note-offs first).
pub fn parse(text: &str) -> Result<Vec<(f32, Event)>, String> {
    let mut tempo = 60.0;
    let mut events = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        let err = |msg: &str| format!("line {}: {msg}: {line}", n + 1);
        let words: Vec<&str> = line.split_whitespace().collect();
        if words[0] == "tempo" {
            tempo = number(words.get(1).copied()).ok_or_else(|| err("expected a tempo"))?;
            continue;
        }
        let beat = 60.0 / tempo;
        let time = number(words.first().copied()).ok_or_else(|| err("expected a time"))? * beat;
        let arg = |i: usize| words.get(i).copied();
        let value = || number(arg(2)).ok_or_else(|| err("expected a number"));
        let velocity = |i: usize| number(arg(i)).unwrap_or(64.0) / 127.0;
        match arg(1) {
            Some("note") => {
                let note = note(arg(2)).ok_or_else(|| err("bad note"))?;
                let length = number(arg(3)).ok_or_else(|| err("expected a length"))?;
                events.push((time, Event::On(note, velocity(4))));
                events.push((time + length * beat, Event::Off(note)));
            }
            Some("on") => {
                let note = note(arg(2)).ok_or_else(|| err("bad note"))?;
                events.push((time, Event::On(note, velocity(3))));
            }
            Some("off") => {
                events.push((
                    time,
                    Event::Off(note(arg(2)).ok_or_else(|| err("bad note"))?),
                ));
            }
            Some("dyn") => events.push((time, Event::Dynamics(value()?))),
            Some("vib") => events.push((time, Event::Vibrato(value()?))),
            Some("pressure") => events.push((time, Event::Pressure(value()?))),
            Some("bow") => {
                let lift = match arg(2) {
                    Some("on") => BowLift::OnString,
                    Some("off") => BowLift::OffString,
                    _ => return Err(err("expected on or off")),
                };
                events.push((time, Event::BowLift(lift)));
            }
            Some("fingering") => {
                let f = fingering(arg(2).unwrap_or("")).map_err(|e| err(&e))?;
                events.push((time, Event::Fingering(f)));
            }
            Some("poly") => {
                let p = match arg(2) {
                    Some("on") => Polyphony::DoubleStops,
                    Some("off") => Polyphony::Mono,
                    _ => return Err(err("expected on or off")),
                };
                events.push((time, Event::Polyphony(p)));
            }
            _ => return Err(err("unknown event")),
        }
    }
    // Note-offs sort before note-ons at the same time, so repeated notes re-bow.
    events.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| matches!(b.1, Event::Off(_)).cmp(&matches!(a.1, Event::Off(_))))
    });
    Ok(events)
}

/// The line up to a `#` that starts a word (a `#` inside a note name is a sharp).
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let start = (0..bytes.len())
        .find(|&i| bytes[i] == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()));
    &line[..start.unwrap_or(line.len())]
}

fn number(word: Option<&str>) -> Option<f32> {
    word?.parse().ok()
}

/// MIDI note number of a name like `C4`, `F#3` or `Bb2` (C4 = 60).
pub fn note(word: Option<&str>) -> Option<u8> {
    let word = word?;
    let mut chars = word.chars();
    let base = match chars.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (accidental, octave) = match rest.chars().next()? {
        '#' => (1, &rest[1..]),
        'b' => (-1, &rest[1..]),
        _ => (0, rest),
    };
    let octave: i32 = octave.parse().ok()?;
    u8::try_from(12 * (octave + 1) + base + accidental).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_names() {
        assert_eq!(note(Some("C4")), Some(60));
        assert_eq!(note(Some("A4")), Some(69));
        assert_eq!(note(Some("C2")), Some(36));
        assert_eq!(note(Some("F#3")), Some(54));
        assert_eq!(note(Some("Bb2")), Some(46));
        assert_eq!(note(Some("H2")), None);
    }

    #[test]
    fn parses_and_sorts() {
        let events =
            parse("# header\ntempo 120\n1 note C3 1 100 # comment\n2 note D3 1\n0 dyn 0.5")
                .unwrap();
        let times: Vec<f32> = events.iter().map(|e| e.0).collect();
        assert_eq!(times, [0.0, 0.5, 1.0, 1.0, 1.5]);
        // At 1.0 s the off of C3 comes before the on of D3.
        assert!(matches!(events[2].1, Event::Off(48)));
        assert!(matches!(events[3].1, Event::On(50, _)));
        assert!(parse("0 jump C3").is_err());
        assert!(matches!(parse("0 on F#3").unwrap()[0].1, Event::On(54, _)));
    }
}
