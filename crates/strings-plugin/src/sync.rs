//! Instance sync (docs/SECTIONS.md B1): every LibreStrings instance on this
//! computer with the same stage name sees the others' sections and shares one
//! room, as SWAM's instances do.
//!
//! The stage is a small memory-mapped file under `$XDG_RUNTIME_DIR` (or the
//! temp dir), so it works whether the host runs all its plugins in one process
//! or each in its own. It holds the room and a fixed table of slots, one per
//! instance: its name, instrument, players and placement. Everything in it is
//! an atomic; each group of values is written under a seqlock, whose version
//! also tells a reader that it changed. A writer that died mid-write (a host
//! crash) only delays the next writer.
//!
//! Each instance runs a thread ([`Link::start`]) that keeps its slot and a
//! heartbeat up to date, publishes its own changes, and copies other
//! instances' changes into its [`Layout`] (from where the audio thread reads
//! them). Another instance's editor moves this section by writing into this
//! slot; this instance then takes the move over, so each instance still owns
//! its section. Slots whose heartbeat stops are freed after a few seconds.
//!
//! Nothing here runs on the audio thread.

use std::fs::OpenOptions;
use std::io;
use std::mem::size_of;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use memmap2::MmapMut;
use strings_dsp::{Absorption, Placement, RoomPreset, StageSettings};

use crate::layout::{Layout, absorption_index, default_name, now_ms, room_index};
use crate::params::{InstrumentParam, StringsParams};

/// Marks a ready stage file; change it (and the file's name) with the layout.
const MAGIC: u32 = 0x4c53_5401;
/// Someone is writing the room's defaults into a new file.
const INITIALIZING: u32 = 1;
pub const MAX_SLOTS: usize = 32;
/// A section's name, UTF-8, cut to fit.
const NAME_BYTES: usize = 32;
const INFO_WORDS: usize = NAME_BYTES / 4 + 3;
const ROOM_WORDS: usize = 7;
/// A section whose heartbeat is older than this (ms) is no longer shown…
pub const STALE: u64 = 2_000;
/// …and after this its slot is freed.
const FREE_AFTER: u64 = 5_000;
/// How often the sync thread looks.
const TICK: Duration = Duration::from_millis(40);

/// `N` words under a seqlock. The version is even when no write is under way,
/// and it grows with every write.
#[repr(C)]
struct Seqlock<const N: usize> {
    seq: AtomicU32,
    words: [AtomicU32; N],
}

/// Spins before a reader or writer gives up waiting for a writer that may
/// have died.
const SPINS: u32 = 20_000;

impl<const N: usize> Seqlock<N> {
    fn version(&self) -> u32 {
        self.seq.load(Ordering::Acquire)
    }

    /// The words and their version.
    fn read(&self) -> ([u32; N], u32) {
        let mut spins = 0;
        loop {
            let before = self.seq.load(Ordering::Acquire);
            let words = std::array::from_fn(|i| self.words[i].load(Ordering::Relaxed));
            fence(Ordering::Acquire);
            let after = self.seq.load(Ordering::Relaxed);
            if (before == after && before.is_multiple_of(2)) || spins > SPINS {
                return (words, after);
            }
            spins += 1;
            std::hint::spin_loop();
        }
    }

    /// Writes the words and returns their new version.
    fn write(&self, words: [u32; N]) -> u32 {
        let mut spins = 0;
        let base = loop {
            let s = self.seq.load(Ordering::Relaxed);
            if s.is_multiple_of(2)
                && self
                    .seq
                    .compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed)
                    .is_ok()
            {
                break s;
            }
            spins += 1;
            if spins > SPINS {
                // A writer died mid-write: take its place.
                let base = s & !1;
                self.seq.store(base + 1, Ordering::Relaxed);
                break base;
            }
            std::hint::spin_loop();
        };
        fence(Ordering::Release);
        for (w, value) in self.words.iter().zip(words) {
            w.store(value, Ordering::Relaxed);
        }
        self.seq.store(base + 2, Ordering::Release);
        base + 2
    }
}

#[repr(C)]
struct Slot {
    /// The instance's ID, or 0 for a free slot.
    owner: AtomicU64,
    /// When the owner last looked in (ms since the Unix epoch).
    heartbeat: AtomicU64,
    /// Name, instrument, players, on stage: see [`Info`].
    info: Seqlock<INFO_WORDS>,
    /// x, y, width and depth (f32 bits). The owner writes it, and so does any
    /// editor that moves the section.
    placement: Seqlock<4>,
}

#[repr(C)]
struct Region {
    magic: AtomicU32,
    /// Room, absorption, mic distance, mic x, reflections, and when it was
    /// last changed (two words).
    room: Seqlock<ROOM_WORDS>,
    slots: [Slot; MAX_SLOTS],
}

/// A stage file, mapped.
pub struct Registry {
    map: MmapMut,
    pub path: PathBuf,
}

/// Another instance's section, as its slot has it.
#[derive(Clone, Debug)]
pub struct Section {
    pub slot: usize,
    pub owner: u64,
    pub name: String,
    pub instrument: InstrumentParam,
    pub players: u32,
    /// On the stage (otherwise it plays dry).
    pub staged: bool,
    pub placement: Placement,
}

/// What an instance shows the others about itself.
#[derive(Clone, PartialEq)]
struct Info {
    name: String,
    instrument: InstrumentParam,
    players: u32,
    staged: bool,
}

impl Info {
    fn words(&self) -> [u32; INFO_WORDS] {
        let mut bytes = [0u8; NAME_BYTES];
        let mut end = self.name.len().min(NAME_BYTES);
        while !self.name.is_char_boundary(end) {
            end -= 1;
        }
        bytes[..end].copy_from_slice(&self.name.as_bytes()[..end]);
        let mut words = [0; INFO_WORDS];
        for (w, chunk) in words.iter_mut().zip(bytes.chunks(4)) {
            *w = u32::from_le_bytes(chunk.try_into().unwrap_or_default());
        }
        words[NAME_BYTES / 4] = self.instrument.index() as u32;
        words[NAME_BYTES / 4 + 1] = self.players;
        words[NAME_BYTES / 4 + 2] = u32::from(self.staged);
        words
    }

    fn from_words(words: &[u32; INFO_WORDS]) -> Self {
        let bytes: Vec<u8> = words[..NAME_BYTES / 4]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .take_while(|&b| b != 0)
            .collect();
        let instrument = words[NAME_BYTES / 4] as usize;
        Self {
            name: String::from_utf8_lossy(&bytes).into_owned(),
            instrument: InstrumentParam::ALL[instrument % InstrumentParam::ALL.len()],
            players: words[NAME_BYTES / 4 + 1],
            staged: words[NAME_BYTES / 4 + 2] != 0,
        }
    }
}

fn placement_words(p: Placement) -> [u32; 4] {
    [p.x, p.y, p.width, p.depth].map(f32::to_bits)
}

fn placement_from(w: [u32; 4]) -> Placement {
    let [x, y, width, depth] = w.map(f32::from_bits);
    Placement { x, y, width, depth }
}

fn room_words(s: StageSettings, changed: u64) -> [u32; ROOM_WORDS] {
    [
        room_index(s.room),
        absorption_index(s.absorption),
        s.mic_distance.to_bits(),
        s.mic_x.to_bits(),
        s.reflections.to_bits(),
        changed as u32,
        (changed >> 32) as u32,
    ]
}

fn room_from(w: [u32; ROOM_WORDS]) -> (StageSettings, u64) {
    let settings = StageSettings {
        room: RoomPreset::ALL[w[0] as usize % RoomPreset::ALL.len()],
        absorption: Absorption::ALL[w[1] as usize % Absorption::ALL.len()],
        mic_distance: f32::from_bits(w[2]),
        mic_x: f32::from_bits(w[3]),
        reflections: f32::from_bits(w[4]),
    };
    (settings, u64::from(w[5]) | u64::from(w[6]) << 32)
}

/// The stage's file name: only letters, digits, `-` and `_`, lowercase, so
/// "Main" and "main" are one stage.
fn file_name(stage: &str) -> String {
    let mut name: String = stage
        .trim()
        .chars()
        .take(40)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        name = "main".into();
    }
    format!("librestrings-stage-{name}.v1")
}

impl Registry {
    /// Opens the stage's file, making it if this is the first instance on it.
    pub fn open(stage: &str) -> io::Result<Self> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|d| d.is_dir())
            .unwrap_or_else(std::env::temp_dir);
        let path = dir.join(file_name(stage));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        let size = size_of::<Region>() as u64;
        if file.metadata()?.len() < size {
            file.set_len(size)?;
        }
        // SAFETY: the file is only ever grown, never truncated, by any
        // instance, so the mapping stays valid while it is held.
        let map = unsafe { MmapMut::map_mut(&file)? };
        if (map.len() as u64) < size {
            return Err(io::Error::other("stage file too small"));
        }
        let registry = Self { map, path };
        registry.initialize();
        Ok(registry)
    }

    fn region(&self) -> &Region {
        // SAFETY: the mapping is page-aligned and at least as large as a
        // `Region`, which is all atomics, so any bytes are a valid `Region`
        // and it can be shared with other threads and processes. The mapping
        // lives as long as `self`.
        unsafe { &*self.map.as_ptr().cast::<Region>() }
    }

    /// Writes a new file's room (the defaults, never changed) once.
    fn initialize(&self) {
        let r = self.region();
        for _ in 0..50 {
            match r
                .magic
                .compare_exchange(0, INITIALIZING, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => break,
                Err(MAGIC) => return,
                Err(INITIALIZING) => std::thread::sleep(Duration::from_millis(2)),
                // Not a stage file of this version: start it again.
                Err(_) => break,
            }
        }
        // Here the file is new, or whoever was initializing it died.
        r.room.write(room_words(StageSettings::default(), 0));
        r.magic.store(MAGIC, Ordering::Release);
    }

    /// Every other live section on the stage (not `except`'s).
    pub fn sections(&self, except: u64) -> Vec<Section> {
        let now = now_ms();
        let mut sections = Vec::new();
        for (slot, s) in self.region().slots.iter().enumerate() {
            let owner = s.owner.load(Ordering::Acquire);
            if owner == 0 || owner == except || !live(s, now) {
                continue;
            }
            let info = Info::from_words(&s.info.read().0);
            sections.push(Section {
                slot,
                owner,
                name: info.name,
                instrument: info.instrument,
                players: info.players.max(1),
                staged: info.staged,
                placement: placement_from(s.placement.read().0),
            });
        }
        sections
    }

    /// Moves another instance's section (if it still has that slot); it
    /// takes the move over on its next tick.
    pub fn move_section(&self, slot: usize, owner: u64, placement: Placement) {
        let Some(s) = self.region().slots.get(slot) else {
            return;
        };
        if s.owner.load(Ordering::Acquire) == owner {
            s.placement.write(placement_words(placement));
        }
    }

    /// Takes a free slot (or one whose owner is gone) for `id`.
    fn claim(&self, id: u64) -> Option<usize> {
        let now = now_ms();
        let slots = &self.region().slots;
        let free = |s: &Slot| {
            let owner = s.owner.load(Ordering::Acquire);
            owner == 0 || now.saturating_sub(s.heartbeat.load(Ordering::Acquire)) > FREE_AFTER
        };
        for (i, s) in slots.iter().enumerate() {
            let owner = s.owner.load(Ordering::Acquire);
            if free(s)
                && s.owner
                    .compare_exchange(owner, id, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                s.heartbeat.store(now, Ordering::Release);
                return Some(i);
            }
        }
        None
    }

    fn release(&self, slot: usize, id: u64) {
        let s = &self.region().slots[slot];
        let _ = s
            .owner
            .compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire);
    }

    /// Frees the slots of instances that stopped looking in.
    fn sweep(&self, now: u64) {
        for s in &self.region().slots {
            let owner = s.owner.load(Ordering::Acquire);
            if owner != 0 && now.saturating_sub(s.heartbeat.load(Ordering::Acquire)) > FREE_AFTER {
                let _ = s
                    .owner
                    .compare_exchange(owner, 0, Ordering::AcqRel, Ordering::Acquire);
            }
        }
    }

    /// Live instances on the stage other than `except`.
    fn others(&self, except: u64) -> usize {
        let now = now_ms();
        self.region()
            .slots
            .iter()
            .filter(|s| {
                let owner = s.owner.load(Ordering::Acquire);
                owner != 0 && owner != except && live(s, now)
            })
            .count()
    }
}

fn live(s: &Slot, now: u64) -> bool {
    now.saturating_sub(s.heartbeat.load(Ordering::Acquire)) <= STALE
}

/// This instance's link to the stage, shared by the plugin, its sync thread
/// and its editor.
pub struct Link {
    /// This instance's ID on the stage.
    pub id: u64,
    registry: RwLock<Option<Arc<Registry>>>,
    /// Why the stage isn't shared, if it isn't.
    error: RwLock<Option<String>>,
    stop: AtomicBool,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Default for Link {
    fn default() -> Self {
        Self {
            id: new_id(),
            registry: RwLock::new(None),
            error: RwLock::new(None),
            stop: AtomicBool::new(false),
            thread: Mutex::new(None),
        }
    }
}

impl Link {
    /// The stage, once the sync thread has joined it.
    pub fn registry(&self) -> Option<Arc<Registry>> {
        self.registry.read().ok()?.clone()
    }

    pub fn error(&self) -> Option<String> {
        self.error.read().ok()?.clone()
    }

    fn set_error(&self, error: Option<String>) {
        if let Ok(mut e) = self.error.write() {
            *e = error;
        }
    }

    /// Starts the sync thread, unless it is running.
    pub fn start(self: &Arc<Self>, params: Arc<StringsParams>) {
        let Ok(mut thread) = self.thread.lock() else {
            return;
        };
        if thread.is_some() {
            return;
        }
        let link = self.clone();
        *thread = std::thread::Builder::new()
            .name("LibreStrings stage sync".into())
            .spawn(move || {
                let mut member = Member::default();
                while !link.stop.load(Ordering::Relaxed) {
                    member.tick(&link, &params);
                    std::thread::park_timeout(TICK);
                }
                member.leave(&link);
            })
            .ok();
    }

    /// Stops the sync thread and frees this instance's slot.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let thread = self.thread.lock().ok().and_then(|mut t| t.take());
        if let Some(thread) = thread {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

/// The plugin's link: dropping it (with the plugin) stops the sync thread.
#[derive(Default)]
pub struct LinkOwner(Arc<Link>);

impl std::ops::Deref for LinkOwner {
    type Target = Arc<Link>;

    fn deref(&self) -> &Arc<Link> {
        &self.0
    }
}

impl Drop for LinkOwner {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// A random ID, never 0.
fn new_id() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(now_ms());
    h.write_u32(std::process::id());
    h.finish().max(1)
}

/// The sync thread's own state.
#[derive(Default)]
struct Member {
    registry: Option<Arc<Registry>>,
    /// The stage name the registry is for.
    stage: String,
    /// When opening it last failed (ms).
    failed_at: Option<u64>,
    slot: Option<usize>,
    info: Option<Info>,
    /// `Layout::placement_edits` when last published, and the slot's
    /// placement version since.
    placement_edits: Option<u32>,
    placement_version: u32,
    /// The same for the room.
    room_edits: Option<u32>,
    room_version: u32,
    swept_at: u64,
}

impl Member {
    fn tick(&mut self, link: &Link, params: &StringsParams) {
        let now = now_ms();
        let layout: &Layout = &params.layout;
        let stage = layout.stage();
        if self.registry.is_some() && stage != self.stage {
            self.leave(link);
        }
        if self.registry.is_none() {
            let retry = self.failed_at.is_none_or(|t| now.saturating_sub(t) > 2_000);
            if !retry && stage == self.stage {
                return;
            }
            self.stage = stage.clone();
            match Registry::open(&stage) {
                Ok(r) => {
                    let r = Arc::new(r);
                    self.registry = Some(r.clone());
                    if let Ok(mut registry) = link.registry.write() {
                        *registry = Some(r);
                    }
                    self.failed_at = None;
                    link.set_error(None);
                }
                Err(e) => {
                    self.failed_at = Some(now);
                    link.set_error(Some(format!("Not shared: {e}")));
                    return;
                }
            }
        }
        let Some(registry) = self.registry.clone() else {
            return;
        };
        let region = registry.region();

        // Our slot, claimed again if it was freed while we weren't looking.
        let ours = self
            .slot
            .filter(|&i| region.slots[i].owner.load(Ordering::Acquire) == link.id);
        let slot = match ours {
            Some(i) => i,
            None => {
                let Some(i) = registry.claim(link.id) else {
                    self.slot = None;
                    link.set_error(Some(format!(
                        "The stage is full ({MAX_SLOTS} instances): this one isn't shown"
                    )));
                    return;
                };
                link.set_error(None);
                self.slot = Some(i);
                self.info = None;
                self.placement_edits = None;
                i
            }
        };
        let s = &region.slots[slot];
        s.heartbeat.store(now, Ordering::Release);

        let players = params.players.value().max(1) as u32;
        let instrument = params.instrument.value();
        let name = layout.name();
        let info = Info {
            name: if name.is_empty() {
                default_name(instrument, players)
            } else {
                name
            },
            instrument,
            players,
            staged: params.stage.value(),
        };
        if self.info.as_ref() != Some(&info) {
            s.info.write(info.words());
            self.info = Some(info);
        }

        // The placement: ours to publish, or moved by another editor.
        let edits = layout.placement_edits.load(Ordering::Relaxed);
        if self.placement_edits != Some(edits) {
            self.placement_version = s.placement.write(placement_words(layout.placement()));
            self.placement_edits = Some(edits);
        } else if s.placement.version() != self.placement_version {
            let (words, version) = s.placement.read();
            layout.adopt_placement(placement_from(words));
            self.placement_version = version;
        }

        // The room: the newer of ours and the stage's wins when ours changes
        // (or we join the stage); after that we follow the stage.
        let edits = layout.room_edits.load(Ordering::Relaxed);
        if self.room_edits != Some(edits) {
            let (words, version) = region.room.read();
            let (settings, changed) = room_from(words);
            // Alone, the stage's room is left over from instances gone.
            let alone = self.room_edits.is_none() && registry.others(link.id) == 0;
            self.room_version = if alone || layout.room_changed() > changed {
                region
                    .room
                    .write(room_words(layout.settings(), layout.room_changed()))
            } else {
                layout.adopt_settings(settings, changed);
                version
            };
            self.room_edits = Some(edits);
        } else if region.room.version() != self.room_version {
            let (words, version) = region.room.read();
            let (settings, changed) = room_from(words);
            layout.adopt_settings(settings, changed);
            self.room_version = version;
        }

        if now.saturating_sub(self.swept_at) > 1_000 {
            registry.sweep(now);
            self.swept_at = now;
        }
    }

    /// Frees our slot and forgets the stage.
    fn leave(&mut self, link: &Link) {
        if let (Some(registry), Some(slot)) = (&self.registry, self.slot) {
            registry.release(slot, link.id);
        }
        if let Ok(mut registry) = link.registry.write() {
            *registry = None;
        }
        *self = Self {
            stage: std::mem::take(&mut self.stage),
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two instances on a stage of their own.
    fn pair(stage: &str) -> [(Arc<Link>, Arc<StringsParams>, Member); 2] {
        std::array::from_fn(|_| {
            let params = Arc::new(StringsParams::default());
            params.layout.set_stage(stage);
            (Arc::new(Link::default()), params, Member::default())
        })
    }

    fn unique_stage(test: &str) -> String {
        format!("test-{test}-{}-{}", std::process::id(), new_id())
    }

    #[test]
    fn instances_see_each_other_and_share_the_room() {
        let stage = unique_stage("share");
        let [(la, pa, mut sa), (lb, pb, mut sb)] = pair(&stage);
        pa.layout.set_name("Celli");
        pa.layout.set_placement(Placement::CELLOS);
        sa.tick(&la, &pa);
        sb.tick(&lb, &pb);

        let registry = lb.registry().unwrap();
        let others = registry.sections(lb.id);
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].name, "Celli");
        assert_eq!(others[0].placement, Placement::CELLOS);
        assert_eq!(registry.sections(la.id)[0].name, "Solo cello");

        // A room set in B reaches A.
        let concert = StageSettings {
            room: RoomPreset::ConcertHall,
            mic_distance: 7.0,
            ..StageSettings::default()
        };
        pb.layout.set_settings(concert);
        sb.tick(&lb, &pb);
        sa.tick(&la, &pa);
        assert_eq!(pa.layout.settings(), concert);
        assert_eq!(pa.layout.room_changed(), pb.layout.room_changed());

        // B's editor moves A's section; A takes it over, and keeps it.
        let moved = Placement {
            x: -2.0,
            ..Placement::CELLOS
        };
        registry.move_section(others[0].slot, others[0].owner, moved);
        sa.tick(&la, &pa);
        assert_eq!(pa.layout.placement(), moved);
        sa.tick(&la, &pa);
        assert_eq!(registry.sections(lb.id)[0].placement, moved);

        let path = registry.path.clone();
        sa.leave(&la);
        assert!(registry.sections(lb.id).is_empty());
        sb.leave(&lb);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_loaded_project_brings_its_room_only_if_newer() {
        let stage = unique_stage("recall");
        let [(la, pa, mut sa), (lb, pb, mut sb)] = pair(&stage);
        let studio = StageSettings {
            room: RoomPreset::Studio,
            ..StageSettings::default()
        };
        pa.layout.set_settings(studio);
        sa.tick(&la, &pa);
        sb.tick(&lb, &pb);
        // B is new: it takes the stage's room.
        assert_eq!(pb.layout.settings(), studio);

        // A project saved before A's change doesn't take the stage's room.
        let mut old = pb.layout.settings();
        old.room = RoomPreset::ScoringStage;
        pb.layout.adopt_settings(old, 1_000);
        pb.layout.room_edits.fetch_add(1, Ordering::Relaxed);
        sb.tick(&lb, &pb);
        assert_eq!(pb.layout.settings(), studio);

        let path = la.registry().unwrap().path.clone();
        sa.leave(&la);
        sb.leave(&lb);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn names_fit_their_slot() {
        let info = Info {
            name: "Violoncelli primi, leggio 1–2".into(),
            instrument: InstrumentParam::Cello,
            players: 4,
            staged: true,
        };
        let back = Info::from_words(&info.words());
        assert!(info.name.starts_with(&back.name));
        assert!(back.name.len() <= NAME_BYTES);
        assert_eq!(back.players, 4);
        assert_eq!(file_name(" Main "), file_name("main"));
        assert_eq!(file_name("../x"), "librestrings-stage-___x.v1");
    }
}
