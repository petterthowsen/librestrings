# Violin Reference Recordings

Real recordings to compare against the physically modeled string library.

Suggested use: Iowa for single notes → Violin Etudes for runs and connected playing.

## TODO

- [ ] **Iowa comparisons need the body filter first.** The Iowa recordings include the instrument body and a microphone, while the model currently outputs raw bridge force. A fair A/B has to wait for the Phase 2 body filter. Until then, compare only attack time, how fast the high harmonics decay, and vibrato.
- [ ] **Look for recordings made with a pickup on the bridge** (electric violins, or research recordings made with bridge piezos). They are the same signal the renderer outputs, so they compare directly with the raw model now.
- [ ] **Bowing-gestures dataset:** check where the data is hosted, its license, and whether bow *force* was recorded, not only position. If so, the recorded bow speed and force could drive the model directly for a one-to-one comparison. Paper: https://www.frontiersin.org/journals/psychology/articles/10.3389/fpsyg.2019.00344/full
- [ ] **Check each dataset's license before using it.** Keep downloaded audio out of git (the project will be open source); add a download script instead.

## 1. Dry isolated notes (basic model check)

### University of Iowa Musical Instrument Samples
- Link: https://theremin.music.uiowa.edu/MIS.html
- Free recordings made in an anechoic chamber, so there's no room sound.
- Violin notes recorded chromatically, bowed (arco) and plucked (pizzicato), with and without vibrato, at pp / mf / ff.
- Good for: comparing attack, spectrum and decay against the model's output (once it has a body filter; see TODO).
- Limitation: no runs or connected phrases.

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
