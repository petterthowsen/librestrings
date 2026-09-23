//! Real-time safety: after construction, playing the instrument never allocates.
//!
//! A counting global allocator stands in for `assert_no_alloc` (PLAN.md 6). It
//! counts allocations on this thread only, so the test harness's own threads
//! don't interfere. It is the only test in this binary.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use strings_dsp::presets::cello;
use strings_dsp::{Articulation, Performer, PerformerSettings};

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
        p.set_expression(0.9);
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
        for articulation in [Articulation::Staccato, Articulation::Spiccato] {
            p.set_articulation(articulation);
            for note in [50, 57, 64] {
                p.note_on(note, 0.8);
                block(&mut p, 0.05);
                p.note_off(note);
                block(&mut p, 0.1);
            }
        }
        p.reset();
        block(&mut p, 0.05);
    });
    assert_eq!(count, 0, "allocations on the audio path");
}
