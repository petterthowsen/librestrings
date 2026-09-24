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

### dyanmics

We should bind Expression CC to Dynamics, we don't really need a separate volume.
We can bind regular modulation wheel to vibrato (amount).
velocity can control attack/accent.

bow pressure goes from flautando to scratch. the deafult is middle.

### Fingering Mode

Three fingering modes, we should probably do the same:
- Mid Position
- Near the Bridge
- near the nut & Open


---

### Prformer Status

In the center, below the instrument visualization, we can show the articulations the performer is playing, plus the 2 previous ones.

Can be a vertical list of events in a semi-transparent black background, each text horizontally centered. The recent one on the top, older below. Recent can be higher font and white font, recent lower font size and more a bit more gray color.

What I've gleaned from swam, they show these events: "Bow stop", "Staccato Attack", "Portamento" etc. 


besides the status box, show current/last bow direction as well.


### bow/pizz position knob
Bow/Pizz position knob, SWAM has this.

### Play accuracy knob (for sections)

default to 0.5. At 1, no humanization is done

### Pizzicato Mode