# Vision

High quality, realistic, expressive and highly playable string instrument system.

## Plugin GUI

Has: settings toggles, status, performance info, interactive keyboard mapping one octave to keys qw3er5t6y7uio0 with transpose up/down buttons (the keyboard mapping).

- maybe status / performance etc has a status row at top, small text.
- below instrument and section selection as dropdowns.
- in the center, a simple drawing/visual of the instrument and the bow maybe and some data
- for debugging, might include more info
- bottom: keyboard plus faders for all the dynamics. makes it easy to test without connected midi hardware.


## Plugin playing

Taking notes from how Audio Modeling's SWAM works.

### Mono/Polyphony & bends

We need polyphony modes, a mode that allows playing two notes at the same time (on different strings).

Currently, seems like bitch bends / slides occur from one string to the next, which isn't physically possible?

### dyanmics

We should bind Expression CC to Dynamics, we don't really need a separate volume.
We can bind regular modulation wheel to vibrato (amount).
velocity can control attack/accent.

bow pressure goes from flautando to scratch. the deafult is middle.

### Fingering Mode

SWAM provides three fingering modes, we should probably do the same:
- Mid Position
- Near the Bridge
- near the nut & Open

### Remove staccatto / spiccato playing modes

we should remove the separate staccato and spicatto modes in favor of one normal play mode which work like like this:

Detached notes = staccato, velocity controls attack
Connected notes = legato, velocity of landing note controls portamento time. pressing hard should give a almost-instant transition, whereas a soft press should be slow.

SWAM also has a Bow Lift toggle, can be "on string" or "off string".

These parameters then can essentially provide most articulations, like Martelè with bow lift "On string" and high velocity and high expression at note on.

SWAM also seems to alternate bow direction automatically - not sure if it matters in terms of our modeling but their visual does that.