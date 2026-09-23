# Violin Reference Recordings

Real recordings to compare against the physically modeled string library.

Suggested use: Iowa for single notes → Violin Etudes for runs and connected playing.

## TODO

- [ ] **Iowa comparisons need the body filter first.** The Iowa recordings include the instrument body and a microphone, while the model currently outputs raw bridge force. A fair A/B has to wait for the Phase 2 body filter. Until then, compare only attack time, how fast the high harmonics decay, and vibrato.
- [x] **Look for recordings made with a pickup on the bridge** (electric violins, or research recordings made with bridge piezos). They are the same signal the renderer outputs, so they compare directly with the raw model now. See section 3. The best match is the mdw Vienna cello datasets, which need a cello G2 preset to compare against.
- [ ] **Bowing-gestures dataset:** check where the data is hosted, its license, and whether bow *force* was recorded, not only position. If so, the recorded bow speed and force could drive the model directly for a one-to-one comparison. Paper: https://www.frontiersin.org/journals/psychology/articles/10.3389/fpsyg.2019.00344/full
- [ ] **Check each dataset's license before using it.** Keep downloaded audio out of git (the project will be open source); add a download script instead.

## 1. Dry isolated notes (basic model check)

### University of Iowa Musical Instrument Samples
- Link: https://theremin.music.uiowa.edu/MIS.html
- Free recordings made in an anechoic chamber, so there's no room sound.
- Violin notes recorded chromatically, bowed (arco) and plucked (pizzicato), with and without vibrato, at pp / mf / ff.
- Good for: comparing attack, spectrum and decay against the model's output (once it has a body filter; see TODO).
- Limitation: no runs or connected phrases.
- The library also has cello (and viola and double bass) recorded the same way, which fits the cello-first milestone (PLAN.md).
- **Cello arco (2012), in use:** `scripts/fetch-reference-data.sh iowa-cello` fetches the mono 16-bit 44.1 kHz set (110 MB) and converts it to WAV; `strings-render compare` measures the model against it (PLAN.md "Phase 4: comparison with recorded notes"). One file per string (sul C, G, D, A), dynamic (pp, mf, ff) and range: chromatic runs of long notes without vibrato, up two octaves of each string (the G string at ff only to D4). The gaps between notes are digital silence; below 20 Hz there is rumble close to a quiet note's fundamental; the player plays a median 15 cents sharp. The ff C file named C2B2 has one extra segment. License: "may be downloaded and used for any projects, without restrictions".
- Checked, not used: the solo cello in Virtual Playing Orchestra 3 is 10 vibrato notes a minor third apart from No Budget Orchestra (bwv662 on freesound, looped and edited, licenses include CC BY-NC) plus staccato blended from Iowa and NBO samples. Fine for listening, not for measuring.

## 2. Runs, passagework and real phrasing

### Violin Etudes (ISMIR 2022)
- Paper: https://archives.ismir.net/ismir2022/paper/000062.pdf
- GitHub: https://github.com/nctamer/violin-etudes
- Many hours of standard study pieces (e.g. Kreutzer, Wohlfahrt), taken from YouTube, with precise pitch (f0) annotations.
- Good for: fast runs, shifts, slides and vibrato — compare against the model's pitch curve.
- Limitation: audio quality varies between recordings.

### KRAISLER (TISMIR)
- Link: https://transactions.ismir.net/articles/10.5334/tismir.338
- Violin and piano duets, with the violin on its own track.
- Good for: longer phrases with musical expression.

## 3. Bridge force and pickup signals (compare with the raw model now)

These record force at the bridge, or a bridge pickup, with no body or room, so they compare directly with the renderer's output.

Download with `scripts/fetch-reference-data.sh` into `data/reference/` (gitignored). It currently fetches the Guettler waveforms and one Schelleng set (type A, sample 1, T1). To fetch more, add entries to its manifest. Each Schelleng archive is about 15 GB to download and needs about 75 GB free while it's being converted; there are 8 parts with about 3 archives each.

### mdw Vienna: bowed responses of cello strings (Lampis, Chatziioannou, Mayer)
- Zenodo, 8 parts (7–8 extend bow force to 4–12 N); links to parts 1–5: [1](https://doi.org/10.5281/zenodo.17749110), [2](https://doi.org/10.5281/zenodo.17782542), [3](https://doi.org/10.5281/zenodo.17782552), [4](https://zenodo.org/records/17782565), [5](https://doi.org/10.5281/zenodo.17782538)
- License: CC BY 4.0, open access. Each archive is about 15 GB (one string type, tension and sample).
- A UR5e robot arm bows a cello G2 string on a monochord with rigid terminations. Recorded at 50 kHz: bow force, bow velocity, **bridge force** and nut force.
- Sweeps: bow force 0.1–4 N (parts 7–8: 4–12 N at β 0.02–0.07), β 0.02–0.2, bow speed 0.05 / 0.1 / 0.2 m/s. Nominal pitch 98 Hz. Archive layout (differs from the Zenodo description): one folder per bow speed, `*_r_v1` / `v2` / `v3` = v_b 0.05 / 0.1 / 0.2 m/s, each with 2000 points as `beta_N.csv`, `timestamp_N.csv` (steady-state window, in samples) and `whole_N.csv` (no header; bow force N, bow velocity m/s, bridge force N, nut force N). Point numbers are not ordered by bow force. An archive extracts to about 70 GB of CSV, so the fetch script runs `scripts/compact-schelleng.py`, which converts it losslessly to about 5 GB: `vb<speed>/<N>.flac` (4 channels in the column order above, 50 kHz, 24-bit; value = int24 × 16 / 2²³, LSB ≈ 1.9e-6, far below the sensor noise of about 2e-3) plus `index.csv` (file, v_b, N, β, window, and the window's mean bow force, mean bow velocity and bridge-force RMS). Four string constructions (steel/tungsten, stranded steel, nylon/silver, steel/nickel) at two tensions.
- Good for: a **measured Schelleng diagram** to compare with `schelleng`, and bridge-force waveforms at known (F, v, β), including our lower-force-limit gap. The real bow force is recorded, so it can drive the model directly.
- Model comparison: `strings-render measured` uses `presets::reference::MONOCHORD_CELLO_G_A_T1`, built from the PoMA paper's Table 1 (T = 145.31 N, μ = 7.721 g/m, Z = 1.059 kg/s, EI = 3.03e-4 N·m², L = 0.70 m). Results are in PLAN.md, "Measured comparison". The PoMA paper is CC BY 4.0 and is kept in [docs/papers/](papers/). Attribution: A. Lampis, V. Chatziioannou, G. Scavone, Proc. Mtgs. Acoust. 58, 035013 (2025), https://doi.org/10.1121/2.0002111. Related papers: [PoMA 2025, Schelleng diagrams of string types](https://pubs.aip.org/asa/poma/article/58/1/035013/3364284/Experimental-analysis-of-cello-string-types), [JASA-EL 2024, attacks](https://pub.mdw.ac.at/media/content_files/113201_1_10.0034330.pdf).

### mdw Vienna: transients / Guettler diagrams
- [Bridge force waveforms, cello G-string models](https://zenodo.org/records/13374477): CC BY 4.0, 3 GB (4.9 GB extracted, 37k files), 50 kHz mono WAV (multiply by 10 for N). Layout: `waveforms/model_<A-D>_<sample>/<date>/Fb_<N>_a_<m/s²>.wav`. Bow force 1.15–3.5 N × bow acceleration 0.15–3.15 m/s².
- [Data for Experimental Assessment of Bowed-String Transient Playability Limits](https://zenodo.org/records/10946413): CC BY 4.0, 22 GB, 11-channel WAV plus MATLAB code. Paper: [Acta Acustica 2024](https://acta-acustica.edpsciences.org/articles/aacus/full_html/2024/01/aacus240063/aacus240063.html).
- Good for: attack behavior (Phase 2+). The same monochord setup as above.

### MTG QUARTET (UPF Barcelona)
- [Zenodo](https://zenodo.org/records/4774190): 96 string quartet takes. Each player has a **bridge pickup** track, plus ambient mics, motion capture and bowing descriptors.
- License: **non-commercial, restricted** (you have to request access), 226 GB. Real violin playing with a bridge pickup, but we can't redistribute it and it probably can't serve as a reference for an open-source project's defaults. It is unclear whether bow force is included; the descriptors come from Maestre's EMF tracking.

### Checked, no pickup
- [good-sounds](https://zenodo.org/records/4588740) (MTG, CC BY 4.0): violin notes and scales, but only Neumann / AKG / iPhone mics (checked `takes.json`).

### For the Phase 2 body filter
- NBody (Cook & Trueman, Princeton 1998): measured violin body radiation IRs, 12 directions. [Paper](https://www.cs.princeton.edu/~prc/ism98fin.pdf). Where the data is hosted is still unknown.
- Pickup-to-mic deconvolution (Pérez-Carrillo et al., [bowed glissandi method](https://www.researchgate.net/publication/51605704_Method_for_measuring_violin_sound_radiation_based_on_bowed_glissandi_and_its_application_to_sound_synthesis)): the method for getting a bridge→radiated IR. The data doesn't appear to be public.
