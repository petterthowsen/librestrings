# Status

Open issues as of September 2026, end of Phase 2 (built, not yet judged by ear). [PLAN.md](PLAN.md) has the design and the full results; this file lists what is unresolved. Remove an entry when it is fixed, and add one when a finding leaves something open.

## Where things stand

| Phase | State |
|---|---|
| 0. Scaffold | Done |
| 1. One bowed string | Done (violin presets) |
| 1b. Measured string physics | Stiffness, torsion and measured damping built and verified. The acceptance target is **met with the bow hair and its width** (83–86% of the measured Helmholtz area; see below) |
| 2. Solo cello | Built: cello presets, bow hair, body, instrument, performer (legato, détaché, the bow lift, double stops), calibrated force band, `play` renderer with example scores. Objective checks pass (`tests/performer.rs`). **First listening done** (Phase 3 notes); the "sounds like a cello" criterion is open |
| 3. CLAP plugin | Built: nih-plug CLAP plugin with MIDI, CCs, keyswitches, parameters and an egui editor (status, instrument view, keyboard, faders, tuning window); standalone app. Tests pass and the engine runs at 3.5% of real time dry and 4.2% on the stage (strings at 2×). **Plays in Bitwig** (September 2026); not yet tried in Reaper |
| 5. More instruments | **All four built:** violin (PLAN.md "Phase 5: the violin"; its bow positions heard), viola and double bass (PLAN.md "Phase 5: viola and double bass"), each with a body from published mode frequencies and a calibrated force band, in the renderer and the plugin. Objective checks pass; the viola and bass renders sound good (first listening, September 2026) |

## Next steps

In this order, from the comparison with recorded notes (items 40–45). Quiet held notes now ease down the band (PLAN.md "Soft, dark pp"), which darkens pp a little; the rest of item 40 needs the friction model. The high positions on a lower string (item 41) are fixed: the bow keeps its distance from the bridge (PLAN.md "High positions: the bow's distance from the bridge"). The low strings' weak fundamental (item 43) is fixed from A2 to D3 and sounds better (PLAN.md "The body's low end").

The bow's width (the cello's and bass's bows touch the string over 12 and 14 mm; PLAN.md "The bow's width") was listened to in `out/ab-bow-width/` (September 2026): it sounds better.

The bow noise (item 42) was listened to in `out/ab-bow-noise/` (September 2026): it sounds good.

Thermal friction (Woodhouse) is built as an option (PLAN.md "Thermal friction"): closer to the measured string's playability map and darker at pp, but no audible difference in `compare` (September 2026), so it is off by default (item 63).

The attacks were compared with the measured Guettler diagrams (item 44; PLAN.md "Attacks against measured data"): the model needed twice the measured bow force for a given acceleration, which the friction's μs sets. The cello's μs is now 0.9 (was 0.8) and its quiet attacks faster (`pp_attack` 0.6, was 1.6). Thermal friction fits the attacks worse than the curve.

The A/B renders (`out/ab-attack-friction/`) were listened to (September 2026): the change sounds good.

The bass now follows the cello's μs 0.9 (item 64; PLAN.md "Attacks against measured data"), with its band recalibrated and slower quiet attacks (`pp_attack` 2.2); its seed sweep passes all 1296 checks. The A/B renders (`out/ab-bass-friction/`) were listened to (September 2026): the change sounds good.

All four bodies are now fitted to the recorded notes' spectral envelope (item 65; PLAN.md "The body fitted to recordings"): `compare` measures the envelope and `fit-body` refits the body to it. The envelope distance to the recordings fell from 7.4–10.3 dB to 6.3–6.7 dB on every instrument.

1. **Play the refitted bodies** (item 65): all four instruments sound different, the bass and viola most.
2. **The violin and viola friction** (item 64): whether they follow the cello's μs.

Each now has its own recorded set to re-run `compare --instrument <name>` against (item 61; the viola needs `--file-rate 96000`), and the model is compared with each instrument's own bow positions, speed and band. The recordings' attacks are slow swells, so they can't set μs the way the Guettler diagrams set the cello's: the change has to be judged by ear, plus a band recalibration and a seed sweep per instrument.

## Listening

1. **First listening only.** Pitch and releases sound right; attacks lacked bite, held notes were static, low notes lacked weight and spiccato sounded plucked (PLAN.md "Phase 3 notes: tuning and first listening"). The fixes for those (attack bite, bow wander, a heavier body, finger damping) are in. A second, informal listening in the standalone app (September 2026) found nothing that stood out, and the A/B renders of the force band, the torsional loss and 2× oversampling all favored the change. The first comparison against recorded notes (items 40–44) has been measured and listened to.
2. **Short notes off the string may sound plucked on low notes.** The scripted spiccato touch is gone (PLAN.md 4.3): a short note is now a real stroke of at least 60 ms, thrown off the string. That is still only a few periods of C2–G2, shorter than the 35–100 ms an attack needs to settle. A bouncing bow is Phase 4.

## Playability vs the measured cello string

The target is a simulated Helmholtz region within ±30% of the measured one, with Helmholtz motion at small β. Measured on the A T1 string (`strings-render measured`):

| | Helmholtz points (0.05 / 0.1 / 0.2 m/s) | at β < 0.05 |
|---|---|---|
| Measured | 701 / 700 / 392 | 529 |
| Best model without hair (measured damping only) | 337 / 277 / 165 | 156 |
| Reference string (damping + stiffness + torsion) | 211 / 194 / 104 | 174 |
| Reference string + cello bow hair (`--hair-stiffness 1000 --hair-damping 3`) | 335 / 325 / 186 | 287 |
| The same with the hair's width (`--bow-width 0.012`, the cello's) | 602 / 579 / 334 | 328 |

The two model rows include the constant-Q torsional loss (PLAN.md "Constant-Q torsional loss"); before it they were 119 / 154 / 107 (127) and 304 / 317 / 181 (274).

3. **The Helmholtz area is 83–86% of the measured one** with the bow hair and its width (PLAN.md "The bow's width"), within Phase 1b's ±30%; 86–94% with the cello's μs 0.9 (`measured --mu-s 0.9`, PLAN.md "Attacks against measured data"). At β < 0.05 it is 62% (328 of 529 points). The width (12 mm) is an estimate of a cello bow's ribbon, not measured; wider gives more (16 mm: 684 / 643 / 371).
4. **The prompt lower force limit is still high.** In the steady state the width brought the model's lower limit at β = 0.1 to 0.24 / 0.44 / 1.03 N at 0.05 / 0.1 / 0.2 m/s (measured 0.21 / 0.31 / 0.76; point contact 0.38 / 0.88 / 1.66). The performer's band counts only forces where notes settle within 0.15 s (PLAN.md "Force band from prompt settling"), and that one hardly moved: for the G string at β = 0.1, v_b = 0.1 m/s it was 1.17–3.12 N (measured 0.31–1.89 N). With μs 0.9 the steady lower limit is 0.19 / 0.37 / 0.88 N and the band 0.95–2.46 N.
5. **Small β, high force fails.** Measured Helmholtz motion reaches 2–4 N at β ≈ 0.02–0.05; the model shows it only in part of that region.
6. **Stiffness and torsion still shrink the Helmholtz region a little** with the measured damping. With the constant-Q torsional loss, torsion alone costs about 10% of the flexible string's Helmholtz points (it was 37%) and adds some at small β; stiffness and torsion together have 509 against the flexible string's 779. Bow-hair compliance (Phase 2) and the bow's width more than make up for it. Next candidate: the friction model (thermal friction, Phase 4).

## Pitch of the bowed string

7. **Open strings play flat at mf–ff: 6–14 cents** in the performer (the flattening effect grows with force). Stopped notes are corrected by the performer's intonation by ear; open strings can't be. Near the top of the band it is far more: the open D at pressure 0.8 (the default is 0.65) plays 34–44 cents flat at mf–ff. In a section, players the humanization sets harder and closer to the bridge play open strings 29 and 46 cents flat (16–20 before the bow's width, whose force band reaches higher near the bridge; PLAN.md "The bow's width"). The force band's upper edge counts regular slipping only, not pitch; an edge that also stops where the pitch falls away would fix it.
8. **Flat zone at β ≈ 0.124–0.156** (1/β ≈ 6.4–8.1): with hair and torsion the cello strings play up to 45 cents flat there, still with one slip per period. It needs torsion; the torsional Q doesn't change it. The dynamics mapping stays below β = 0.115, which also rules out real sul tasto for now. High stopped notes go above it, because the bow keeps its distance from the bridge; the ear corrects stopped notes.
9. **Short stopped notes aren't intonated.** The performer listens only after the finger has settled in a held stroke, so short notes mostly use the correction learned on that string so far.

## Uncertain data

10. **The torsion parameters are estimates, not measurements of this string.**
   - The paper's `Zto` column doesn't convert to a plausible impedance under either reading of its units, so it is unused.
   - The frequency (5.5 × f0) comes from a different steel cello G string (Mores 2019).
   - The impedance (3.3 × Z) assumes κ = 0.6.
   - Q = 50 is a guess between "an order of magnitude below transverse Q" (Mores 2019) and "more than fifty times lower" (Bavu et al. 2005). Constant Q across the modes follows Woodhouse & Loach (1999).
   - The cello C, D and A strings reuse all three.
   - See PLAN.md 3.6 and [docs/Literature.md](docs/Literature.md).
11. **The bow-hair parameters are fitted, not measured:** k = 1000 N/m and R = 3 kg/s maximize the Helmholtz area on the one measured string (PLAN.md "Phase 2 notes"). R is plausible for the hairs in contact; k is softer than the hair ribbon alone.
12. **The damping curve is valid only up to 1.7 kHz (mode 17).** Above that it is extrapolated (the results barely depend on it). Stopped notes extrapolate further: ζ(f) at several kHz is pure extrapolation.
13. **The damping data is digitized from a small plot** (Fig. 1, A T1 panel). Mode 3 has a wide spread and is left out of the fit.
14. **Only one measured string.** The cello C, D and A strings take their tension from a published set but borrow the G string's damping, bending stiffness and torsion. The comparison itself rests on one G string on a monochord (rigid terminations, no body).
15. **The instrument damping is partly estimated.** The cello presets add ζ = 7e-4 to the measured monochord damping for energy lost into the body (open G: about 11 s to −60 dB). The finger's damping (0.015 Np per reflection, 0.095 once the note is over) is a guess, and the same at every frequency; a real fingertip damps the upper partials more.
16. **The body is only partly sourced.** The six low mode frequencies are from the literature. Their damping and signs, and the dense modes' damping and density, are estimates. The modes at 118 and 144 Hz are fitted to the recorded notes' fundamentals (PLAN.md "The body's low end"). The dense modes' envelope (hills, level, rolloff) is fitted to the recorded notes' spectral envelope (item 65), so it describes one cello heard through one microphone, not a bridge admittance. There is no measured cello bridge admittance or radiation data yet.

## Model limits

17. **The damping fit is within 0.68–1.45 × the curve** over modes 1–15: too low around modes 2–4, too high around 9–11. A one-pole × Butterworth structure can't do better.
18. **Dispersion is accurate only up to about 2.5 kHz** (≤ 1.5 cents; ≤ 3.2 cents to 3.4 kHz) and under-dispersed above. The double bass (B about 5× larger) may need more sections or another design.
19. **Dispersion delay limits short loops.** The 16-section cascade adds about 60 samples on the cello G string. On a short, stiff loop (high stopped notes, violin E if it gets stiffness) the nut delay would clamp and tuning would drift. There is no check or test for that case beyond two octaves.
20. **The torsional bridge-side line is clamped to 2 samples,** so torsion is wrong below β ≈ 0.02 on the cello.
21. **The violin strings have no stiffness, torsion or measured damping.** They use the Phase 1 one-pole loss (decay times of 1.2–2 s are guesses), and their lower force limit sits 5–10× above Schelleng's F_min (known behavior; PLAN.md 4.2). The violin instrument plays them with the cello's bow hair; the physics tests and the violin Schelleng map use a rigid bow.
22. **Quiet short notes off the string are unmeasured.** The old spiccato on the A string barely rang at pp (21–35 dB below its touch); the new thrown stroke is checked only at mf.

## Engineering gaps

23. **Filter coefficients are modulated but untested for artifacts.** Vibrato and legato change the loss filter (a Butterworth biquad) and the dispersion coefficient at the performer's 3 kHz control rate. Neither has been checked for zipper noise or transients.
25. **CPU cost is measured only roughly:** the whole cello (4 strings, body, performer) renders at about 6.4% of real time on one core at 48 kHz with the strings at 2× (renderer timing, before idle strings were skipped), and the plugin's engine at 3.5% dry and 4.2% on the stage (`cpu_cost` test in `strings-plugin`; 3.0% and 3.7% before the bow's width, 4.8% before strings that have rung out with the bow off were skipped). 12 players on the stage cost 27.7% (`cpu_cost_players`, docs/SECTIONS.md A1–A5; 24.5% before the width); nearly all of it is the bowed string, not the body. There are no `criterion` benchmarks yet (PLAN.md 6). The standalone's load meter showed about 4% while playing, probably CPU frequency scaling under light real-time load; unconfirmed.
26. **The force band is calibrated on open strings only.** Stopped notes rely on the band scaling with Z·v·β; the performer tests cover C2–E5 at three dynamics, plus six notes 18–24 semitones up the C, G and D strings. High on the A string (above E5) isn't tested.
27. **The classifiers are approximate.** The bridge-force classifier agrees with the contact-state one on 77–93% of simulated violin points, and it counts the paper's multiple-flyback and S-motion regimes as multi-slip or raucous.

## Plugin

28. **Not yet tried in Reaper.** The Phase 3 criterion is "playable in Bitwig and Reaper". It loads and plays in Bitwig on Linux with no problems found (September 2026).
29. **The crates still carry the placeholder names** (`strings-dsp`, `strings-plugin`, `strings-render`). The product is LibreStrings, with CLAP ID `io.github.petterthowsen.librestrings` (fixed for good: hosts save projects against it).
31. **Computer-keyboard input depends on the host** giving the editor keyboard focus. Some hosts keep the keys for their own shortcuts.
32. **Keyswitches and CC numbers are fixed** (PLAN.md 4.1); user mapping is still to come.
33. **The tuning window's changes aren't saved** with the plugin; they are for finding values to put into the code ("Copy changes"). Its clipboard buttons and the strings' refit have only been tested in code, not clicked in a host.


## Sections

47. **12 cello players are over PLAN.md's 25% budget:** 27.7% in the plugin engine's test with the bow's width (24.5% before; at most 2 contact points still cost 25.5%, so it is the contact loop more than the solves). A legato line cost 29% in the renderer before the width: more strings ring on in legato. SIMD across players (Phase 6) is the planned way down.
48. **The solo cello costs 4.2% on the stage** (3.5% dry, with Stage off), within the 5% solo budget; 3.7% before the bow's width.

## Playing controls

36. **The cello's flautando is only a lighter bow.** Real flautando also moves toward the fingerboard, but the model's cello strings play flat above β ≈ 0.115 (item 8), so the pressure control can't move the bow there. The violin's does (`PerformerSettings::tasto`, item 52).
37. **MIDI pitch bend is ignored.** It could move the finger on the bowed string(s); open strings can't bend.
38. **Double stops are limited:** two notes, a fixed hand span (4 semitones at every position, where high positions allow more), and intonation by ear only on the older note. Chords of three or four strings (broken or with high force) aren't played.
39. **The fingering modes are one number each** (how far above a lower string's open pitch a note stays on it). In a double stop they only rank the pairs of strings, so mid position can still take an open string when the other note would be past its bias.

## Against recorded notes

From `strings-render compare` against the Iowa cello notes (PLAN.md "Phase 4: comparison with recorded notes"). First listening to the pairs in `out/compare/` (September 2026):
- pp on the C string: the recordings are very floaty and soft; the model is harder (item 40).
- mf on the G string: the recording sounds brighter overall, with more characteristic bow noise. Its harmonics are actually darker than the model's (partials 4–7: −13 against −6 dB), so the brightness heard is probably the noise (item 42). Its attacks are slightly longer, brighter and more obvious.
- The sharp, noisy high positions are audible (fixed since; see item 45), and so is the weak low end: the low strings lack the deep cello presence (item 43).

40. **The spectrum doesn't follow the dynamics.** Recorded notes brighten from pp to ff by 7–11 dB in partials 4–7 and 9–14 dB in partials 8–15; the model's spectrum stays about the same. The bow's width darkened pp on the low strings above partial 8 (C string −8 → −14 dB, recorded −32; centroid 4.7 → 3.0, recorded 1.3) and brought up the C string's pp fundamental (−10 → −5 dB, recorded −1), but partials 4–7 at pp are still about 13–15 dB too strong on the C and G strings, and at mf–ff the A string is now 13 dB too dark above partial 8 (−36 against −23). Easing quiet held notes down the band (`quiet_ease`, cello and bass) brought the C string's pp partials 8–15 from −15 to −18.5 dB and 4–7 from −5.5 to −6.6 (PLAN.md "Soft, dark pp"); the A/B files in `out/ab-quiet-ease/` sound good (September 2026). **The model can't get much darker than an ideal sawtooth:** once settled, even far below the band a pp note's bridge force keeps partials 4–7 near −9 dB, and no bow width, hair or friction-curve setting tried goes further. Real strings round the Helmholtz corner at low force (Cremer); the model's friction curve keeps sharpening it. Thermal friction (item 63) takes pp partials 4–7 from −9 to −13 dB and 8–15 from −24 to −28 (recorded −19, −32), inaudibly in `compare`; it is off by default.
42. **Bow noise is in, and sounds good** (September 2026). Friction noise while the string slips (PLAN.md "Bow noise") brings the HNR to 31–32.5 dB at pp–ff (recorded 27–32; 43–49 before). Still open:
   - pp is 2–6 dB cleaner than the recording;
   - the noise spectrum is within 3 dB of the recording's since the fitted body, except 5 dB weak at 4–8 kHz at pp (item 65; it was 6–8 dB too strong at 1–2 kHz and 2–5 dB weak at 4–8 kHz, from the estimated 1.3 kHz bridge hill);
   - the attack's noise against the recorded mf onset isn't measured (`compare` measures the sustain only);
   - the seed sweeps fail a few more quiet attacks (violin 4 → 6; the bass's went 0 → 9 of 1296, and back to 0 with μs 0.9 and slower quiet attacks), within the 1% limit.
   The level is fitted to one recorded cello and used on all four instruments.
43. **The low strings' fundamental is mostly fixed.** The body lost it, not the string (`compare --bridge`). Two fitted modes (118 and 144 Hz) fill the gap between A0 and the dense modes. On the C string A2–D3 went from −6 to −31 dB to −1 to −8 at mf–ff (recorded −1 to −5), and to −3 to −15 at pp, and the G string's median from −5 to −2.5 dB (recorded −2.2) at mf (PLAN.md "The body's low end"). Still weak: G2–Ab2 (−7 to −17 against −1 to −2) and, on the C string, Ab3–C4 around 220 Hz (−10 to −13 against 0 to −2). The A/B renders (`out/ab-body-low-end/`) were listened to (September 2026): the change sounds good.
44. **One recorded player and one microphone.** The Iowa notes have no vibrato and one way of starting (a slow swell at pp–mf), and the player plays a median 15 cents sharp. Attack times and rings compare how the notes were played as much as the instrument. For what the string can do, `strings-render guettler` compares attacks from rest with the mdw Guettler diagrams (four G strings on a monochord at one β, constant force and acceleration; PLAN.md "Attacks against measured data"): with μs 0.9 the successful region's right edge is F = 1.28–1.65·a (measured 0.91–1.63), success and failure agree in 83–84% of strokes, and the model is too willing at low force and slow acceleration (0.5–1 N), where the real strings fail. The data has no other β, speeds above 0.7 m/s, or the grip and bite the performer adds.
45. **Two high positions are still a little off:** ff sul D B4 plays 17 cents sharp and sul D C#5 5 cents flat, with clean Helmholtz motion, in `compare`. In the seed sweep sul G D4 at mf settles slowly (0.4–1 s) on 2 of 24 seeds. The bow's minimum distance (0.024 m per kg/s of impedance) is fitted to where the model fails, not to players; it works from 3.5 to 4.5 cm on the C string. The A/B renders (`out/ab-bow-distance/`: `play sul` and `play phrase --fingering bridge`, before and after) were listened to (September 2026): the change sounds better.

## Violin

51. **The violin's bow hair is the cello's,** fitted to a cello string. There is no measured violin string or Schelleng diagram to fit it to.
52. **The violin's wider bow positions have limits** (PLAN.md "Phase 5: the violin"): β 0.16–0.065 with the dynamics, and flautando moving to β 0.2 (sul tasto) at band position 0.3. The A/B renders (`out/ab-violin-beta/`) were listened to (September 2026): they sound pretty good. Quiet attacks are no longer slowed (`pp_attack` 0), so pp attacks are faster than the cello's. Above β ≈ 0.16 at pp the performer's attacks hold multiple slips, so real sul tasto at pp (β up to 0.2 with normal pressure) isn't reachable; the seed sweep has 5 failed checks, all quiet attacks. Halfway to flautando (pressure 0.25), the open G at pp holds multiple slips on a few seeds.
53. **The violin and viola bows touch at one point.** With the hair's width their force bands shrink by about a third (violin 1346 → 688 Helmholtz cells at 10 mm, 959 even at 3 mm; viola 1114 → 704 at 11 mm): the upper limit falls (PLAN.md "The bow's width"). Violinists tilt the bow, so fewer hairs touch, but there is no measured violin string to decide how wide the contact should be.
54. **Changing the instrument in the plugin cuts off what is sounding.** The old engine is swapped out at once, with no fade. Not tried in a host yet.

## Viola and double bass

55. **The viola and the double bass have had one listening** (September 2026: the renders in `out/viola/` and `out/bass/` sound good). As on the violin, much of what shapes the sound is a first guess: the bodies' damping and signs (the mode frequencies are from data, the levels and the dense modes' envelope are fitted to the recorded notes since item 65; the viola's CBR frequency is an estimate), the viola strings' decay times, the output gains (matched to the cello's level, not by ear) and the performer's timings, which are the cello's.
56. **The bass strings are not measured.** The bending stiffness (EI 5e-3, B ≈ 1.4e-4) is an estimate from core size; the damping curve and the torsion are the cello's. Real bass strings (steel rope cores) may be less stiff. Dispersion is accurate only to about 2.5 kHz (item 18), about partial 60 of the open E, which the body's rolloff above 1.5 kHz mostly hides.
57. **The bass E string's Helmholtz band is narrow** (PLAN.md "Phase 5: viola and double bass"): found in 22 of 48 calibration columns, one or two force rows deep; prompt Helmholtz motion at band positions 0.5–0.8 in 79–88% of cells. No string or bow parameter tried widens it. The performer's range checks pass on it, but a bass-specific bow (more hair, heavier), fitted to a measured bass string, is open.
58. **The bass plays slower and farther from the bridge:** at most 0.3 m/s at ff (the others 0.5) and 0.032 m per kg/s from the bridge (the others 0.024), fitted to where high positions fail, not to players. Notes 18–23 semitones up the E string hold multiple slips at pp (the bow sits near the middle of the string there) and are left out of the checks; the seed sweep failed 4 of 1152 checks, ff thumb-position notes 33–72 cents flat; with the bow's width it fails none of 1296.
59. **The bass's open strings may play up to 30 cents flat:** the open E played 20–25 cents flat at mf–ff before the C extension (the other instruments' open strings 6–14); the range checks still allow the bass 30. E1 is now a stopped note and intonated; the open C1 plays 15.6 cents flat at mf.
60. **The bass's left hand is the cello's:** it spans 4 semitones (`hand_span`), where a bassist covers about 2 in the lower positions, so legato lines shift less often than on a real bass. Its fingering modes assume nothing about tuning, and work in fourths.
61. **The violin, viola and bass now compare against their own recorded notes** (`compare --instrument`, PLAN.md "The other instruments' recorded notes"): the model's ring-off, pitch and HNR sit within a few dB of the recordings, 89–101 pairs per dynamic. Still open:
   - **the viola set's 16/44.1 kHz files hold 96 kHz audio** (the take named `A4B4` reads 202 Hz at 44.1 kHz, 440.4 Hz read at 96 kHz), so it must be run with `--file-rate 96000`; the rate is wrong in Iowa's published files, not in the download (docs/Violin Reference Recordings.md §1);
   - the pairs in `out/compare-violin/`, `out/compare-viola/` and `out/compare-bass/` haven't been listened to;
   - the model's partials 4–7 at pp are still brighter than the recordings' on all three (the cello's item 40 pattern);
   - the recorded attacks are 4–30× the model's at every dynamic (these players swell into the note, as the cello's do), so they can't set the model's attack times or μs.
62. **The bass's C extension has no gates.** E1 is a stopped note on the extended string (finger damping, intonated by ear) where a real extension stops it with a metal gate or a machine, closer to an open string. The extended string's force band blends between two fits (PLAN.md "The bass's C extension"); C1 and D1 have been checked, not heard.

## Thermal friction

63. **Thermal friction is an experiment, off by default** (PLAN.md "Thermal friction"). Before it could be the default:
   - the pitch falls with the force (open strings up to 20 cents flat, some stopped notes 25–49 after the ear's correction) and wobbles by 4–5 cents;
   - the bow noise's level was fitted with the friction curve: with thermal friction the HNR is 3–5 dB too low;
   - ff is 4–7 dB too dark above partial 8;
   - its speed dependence (`speed_exponent` 0.5) is a fit, not from the literature: Woodhouse's model alone fails above about 0.2 m/s;
   - it costs 30–50% more per string, which puts 12-player sections further over budget (item 47);
   - the plugin's debug view shows the friction curve's band while it is on;
   - it fits the measured attacks worse than the curve: its successful region is a narrow band (F ≈ 0.7·a) that fails at slow accelerations (`guettler --thermal`, strings B and C: 14–18% of strokes succeed against 40–43% measured; PLAN.md "Attacks against measured data").

## Attacks

64. **Only the cello and bass have the new friction.** The cello's μs went from 0.8 to 0.9 and its `pp_attack` from 1.6 to 0.6 (PLAN.md "Attacks against measured data"); the A/B renders sound good (September 2026), with `attack`, `attack_bite`, `grip_attack` and `bite` unchanged. The bass followed (μs 0.9, band recalibrated, `pp_attack` 1.6 → 2.2: its extension's quiet attacks settle late when faster); its A/B renders sound good (September 2026). The violin and viola keep μs 0.8: there is no attack data for them (the Iowa notes item 61 now compares are slow swells, which can't set μs), and a change means recalibrating each band and running its seed sweep.

## Open decisions

The rest is in PLAN.md "Open questions": CC64 vs CC68 for legato, and renaming the crates.

## The body fitted to recordings

65. **All four bodies are fitted to the recorded notes' spectral envelope** (PLAN.md "The body fitted to recordings"). The envelope distance (per-note RMS over sixth octaves, recording − model) fell to 6.5 / 6.5 / 6.3 / 6.7 dB for cello / violin / viola / bass (7.7 / 7.4 / 8.0 / 10.3 before). Still open:
   - **Not heard yet.** The output gains were corrected to keep each instrument's level (cello +2.3 dB, violin +6.2, viola −6.5, bass −10.9), from `compare`'s median sustain level, not by ear.
   - **5.6–5.9 dB of it is note-to-note detail** that a smooth body can't remove (this body's modes against the recorded instrument's, and item 40). Single bins stand out: cello 320 Hz (+7 dB, +16 at pp–mf), violin 400 Hz (+8), viola 200 Hz (+8), bass 63 Hz (+12) and 320 Hz (+7).
   - **Several listed modes' levels hit the fit's 4× limit** (violin B1−, viola A0 and B1+, bass A0, T1 and A2). The dense modes now carry most of those instruments' low end.
   - **One recording per instrument.** The fit includes that recording's microphone position and room. A second set (another player or microphone) would show how much of the envelope is the instrument.
   - **The viola is darker than its recording at mf–ff** in partials 4–7 (−16 against −11 dB; −9 to −10 before).

