//! Real-time safety: after construction, playing the instrument never allocates.
//!
//! A counting global allocator stands in for `assert_no_alloc` (PLAN.md 6). It
//! counts allocations on this thread only, so the test harness's own threads
//! don't interfere.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use strings_dsp::presets::cello;
use strings_dsp::{
    Absorption, BodyTuning, BowHair, BowLift, DampingCurve, Fingering, FrictionParams,
    Humanization, Instrument, Loss, MAX_PLAYERS, Performer, PerformerSettings, Placement,
    Polyphony, RoomPreset, Section, Stage, StageSettings,
};

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // `try_with`: thread-locals may be gone while a thread shuts down.
        let _ = COUNTING.try_with(|c| {
            if c.get() {
                ALLOCATIONS.with(|a| a.set(a.get() + 1));
            }
        });
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn allocations_during(f: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|a| a.set(0));
    COUNTING.with(|c| c.set(true));
    f();
    COUNTING.with(|c| c.set(false));
    ALLOCATIONS.with(|a| a.get())
}

#[test]
fn playing_never_allocates() {
    let fs = 48_000.0;
    let mut p = Performer::new(&cello::INSTRUMENT, PerformerSettings::default(), fs);
    let block = |p: &mut Performer, seconds: f32| {
        for _ in 0..(seconds * fs) as usize {
            std::hint::black_box(p.process());
        }
    };
    let count = allocations_during(|| {
        p.set_dynamics(0.7);
        p.set_vibrato(0.8);
        p.set_pressure(0.9);
        // Détaché, legato across strings, and a long held-note stack.
        p.note_on(36, 0.5);
        block(&mut p, 0.2);
        for note in 40..60 {
            p.note_on(note, 0.6);
            block(&mut p, 0.02);
        }
        for note in (36..60).rev() {
            p.note_off(note);
            block(&mut p, 0.01);
        }
        for bow_lift in [BowLift::OnString, BowLift::OffString] {
            p.set_bow_lift(bow_lift);
            for note in [50, 57, 64] {
                p.note_on(note, 0.8);
                block(&mut p, 0.05);
                p.note_off(note);
                block(&mut p, 0.1);
            }
        }
        // Double stops, a line over a held note, and the other fingerings.
        p.set_polyphony(Polyphony::DoubleStops);
        p.set_fingering(Fingering::Bridge);
        p.note_on(50, 0.6);
        p.note_on(57, 0.6);
        block(&mut p, 0.1);
        for note in [59, 61, 62] {
            p.note_on(note, 0.6);
            block(&mut p, 0.05);
        }
        p.release_all();
        block(&mut p, 0.2);
        p.reset();
        block(&mut p, 0.05);
    });
    assert_eq!(count, 0, "allocations on the audio path");
}

/// The tuning window's changes are applied on the audio thread: retuning the
/// body, bow and performer, and swapping in refitted strings, never allocate.
#[test]
fn retuning_never_allocates() {
    let fs = 48_000.0;
    let mut p = Performer::new(&cello::INSTRUMENT, PerformerSettings::default(), fs);
    // Built off the audio thread: a body with every dense mode, and refitted strings.
    let mut body = BodyTuning::from(&cello::BODY);
    body.dense.count = strings_dsp::body::MAX_DENSE_MODES;
    body.dense.from = 80.0;
    let strings = cello::STRINGS.map(|s| strings_dsp::StringSpec {
        loss: Loss::Measured(DampingCurve {
            floor: 2e-3,
            at_1khz: 1e-3,
            exponent: 3.0,
        }),
        bending_stiffness: 1e-4,
        ..s
    });
    let mut designs = Instrument::design_strings(&strings, p.instrument().string_sample_rate());
    let mut settings = PerformerSettings::default();
    settings.tuning.attack_bite = 0.4;
    settings.tuning.finger_loss = 0.05;

    p.note_on(52, 0.8);
    let count = allocations_during(|| {
        for _ in 0..4800 {
            std::hint::black_box(p.process());
        }
        p.set_settings(settings);
        let instrument = p.instrument_mut();
        instrument.set_body(&body);
        instrument.set_friction(FrictionParams {
            mu_s: 0.9,
            ..cello::INSTRUMENT.friction
        });
        instrument.set_hair(Some(BowHair {
            stiffness: 2000.0,
            damping: 5.0,
            width: 0.008,
        }));
        assert!(instrument.apply_strings(&strings, &mut designs));
        for _ in 0..4800 {
            std::hint::black_box(p.process());
        }
    });
    assert_eq!(count, 0, "allocations while retuning");
}

/// A section plays through the stage, changes size and place, retunes its humanization and bodies, and
/// releases notes still waiting for late players, all without allocating.
#[test]
fn a_section_never_allocates() {
    let fs = 48_000.0;
    let mut s = Section::new(
        &cello::INSTRUMENT,
        PerformerSettings::default(),
        Humanization::default(),
        fs,
    );
    let mut stage = Stage::new(StageSettings::default(), Placement::CELLOS, 7, fs);
    let body = BodyTuning::from(&cello::BODY);
    let mut out = [0.0; MAX_PLAYERS];
    let mut block = |s: &mut Section, stage: &mut Stage, seconds: f32| {
        for _ in 0..(seconds * fs) as usize {
            s.process(&mut out);
            std::hint::black_box(stage.process(&out));
        }
    };
    let count = allocations_during(|| {
        s.set_players(MAX_PLAYERS);
        stage.set_players(MAX_PLAYERS);
        s.set_dynamics(0.7);
        s.set_vibrato(0.8);
        for note in 45..70 {
            s.note_on(note, 0.6);
            block(&mut s, &mut stage, 0.01);
        }
        for note in 45..70 {
            s.note_off(note);
        }
        block(&mut s, &mut stage, 0.1);
        s.set_players(3);
        stage.set_players(3);
        stage.set_placement(Placement {
            x: -4.0,
            ..Placement::CELLOS
        });
        stage.set_settings(StageSettings {
            room: RoomPreset::ConcertHall,
            absorption: Absorption::High,
            mic_distance: 12.0,
            reflections: 0.5,
        });
        stage.reset();
        s.set_humanization(Humanization::NONE);
        s.set_body(&body);
        s.set_settings(PerformerSettings::default());
        s.note_on(50, 0.7);
        block(&mut s, &mut stage, 0.1);
        s.set_players(8);
        s.note_on(57, 0.7);
        s.release_all();
        block(&mut s, &mut stage, 0.1);
        s.reset();
        block(&mut s, &mut stage, 0.02);
    });
    assert_eq!(count, 0, "allocations on the audio path");
}
