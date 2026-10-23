//! Sumner, the tower's wizard (world/tower.c): his speeches when shards and
//! runestones are brought back, the follow-up window and runestone scenes, the
//! level-up congratulations, the story message boxes, and the trigger cameras
//! they use.
//!
//! Naming follows `Progress`: `runes` are the boss shards (bits 1..11 in world
//! order) and `shards` the 13 runestones, which is how tower.c names them too
//! (its effect names are the other way round).

use crate::fs::GameFs;
use crate::game::{Game, Kind, PlayerState, ScrollBox, CRYSTALS_NEEDED, GARG_NEEDED};
use gdl_formats::text::TextRom;
use gdl_formats::worlds::LocatorType;
use glam::Vec3;

/// The tower lectern trigger's id (item marker 0xF0).
pub const LECTERN: u8 = 0xF0;

/// How the tower was entered (EnterTower / sumnerUpdatePresence).
#[derive(Clone, Copy, Debug, Default)]
pub struct Entry {
    /// gSumnerReady: the session's first tower entry, every player with no
    /// experience. Shows the welcome story and the crystal camera.
    pub welcome: bool,
    /// What tower.c has banked (save.rune_stones2 / rune_stones): new bits in
    /// the party's live masks start a speech.
    pub banked_runes: u16,
    pub banked_shards: u16,
    /// The last level beaten (towerRecordLevelBeaten), for the Garm story and
    /// the missing-13th-runestone speech.
    pub last_beaten: Option<u32>,
}

/// sumnerUpdatePresence: Sumner welcomes the party (gSumnerReady) on the
/// session's first tower entry (lbl_80344C4C == 0) when no active player has
/// experience in any of the 16 class slots of their save image
/// (`save.atts[k].exp`). `exps` is every active player's live experience and
/// every class slot of their hero's save.
///
/// Not in the original: a party that has been welcomed (`welcomed`, kept in
/// the save) or has already collected the orange crystals his welcome drops
/// is not welcomed again. The original would repeat it each time a hero
/// saved with no experience is loaded.
pub fn welcome_due(
    progress: &crate::progress::Progress,
    exps: impl IntoIterator<Item = u32>,
) -> bool {
    !progress.entered_tower
        && !progress.welcomed
        && progress.crystals[1] >= 0
        && exps.into_iter().all(|e| e == 0)
}

/// TriggerCameraActivate: a fixed camera from a type-9 locator.
#[derive(Clone, Copy, Debug)]
pub struct TriggerCam {
    pub eye: Vec3,
    pub dir: Vec3,
    /// Main timer in ticks (BC); -1 runs until ended.
    main: i32,
    /// Hold timer that runs once the main one is out (C0).
    hold: i32,
}

impl TriggerCam {
    /// "Cam done": both timers (nearly) out.
    fn done(&self) -> bool {
        self.main >= 0 && self.main <= 4 && self.hold <= 4
    }
}

/// The tower's waypoints (LookoutParams): its type-8 and type-10 locators,
/// numbered by their subtype (stored in `delay`). Waypoint 0 is Sumner's place
/// at the lectern; 1..11 are by the realms' portal rings.
pub fn waypoints(g: &Game) -> impl Iterator<Item = &gdl_formats::worlds::Locator> {
    g.level
        .worlds
        .locators
        .iter()
        .filter(|l| matches!(l.kind, LocatorType::Sentry | LocatorType::Event))
}

/// A waypoint's facing: add_arrow turns lookout matrices round by pi.
fn lookout_yaw(l: &gdl_formats::worlds::Locator) -> f32 {
    l.rot[1] + std::f32::consts::PI
}

/// The type-9 locator `id` (eye = pos; angles = pitch, yaw, roll). Its view is
/// Yaw(pi - yaw) then Pitch(pitch) (bosscam.c:348).
pub fn locator_cam(g: &Game, id: u16) -> Option<(Vec3, Vec3)> {
    let l = g
        .level
        .worlds
        .locators
        .iter()
        .find(|l| l.kind == LocatorType::TriggerCamera && l.next == id)?;
    let (pitch, yaw) = (l.rot[0], l.rot[1]);
    let dir = Vec3::new(
        yaw.sin() * pitch.cos(),
        -pitch.sin(),
        yaw.cos() * pitch.cos(),
    );
    Some((Vec3::from(l.pos), dir))
}

/// CaptionText's typewriter: the text revealed by `budget` (ticks / 2). Each
/// character costs 1.75, `,` and `.` 2 more, a tab 5 (a pause, not drawn) and
/// `\r` 30, which also starts a new page. True once all of it is shown.
pub fn typewrite(text: &str, budget: f32) -> (String, bool) {
    let mut cost = 0.0;
    let mut out = String::new();
    for c in text.chars() {
        let k = match c {
            ',' | '.' => 3.75,
            '\t' => 5.0,
            '\r' => 30.0,
            _ => 1.75,
        };
        if cost + k > budget {
            return (out, false);
        }
        cost += k;
        match c {
            '\r' => out.clear(),
            '\t' => {}
            c => out.push(c),
        }
    }
    (out, true)
}

/// A speech's caption: one line, or every line in turn with 60 ticks between.
#[derive(Clone, Debug, Default)]
struct Caption {
    lines: Vec<String>,
    line: usize,
    /// Speech tick the current line started on.
    start: i32,
    /// Ticks waited since the line was fully shown.
    wait: i32,
}

/// Story message boxes (SCROLL_E): sounds per crystal realm and gargoyle gate.
const CRYSTAL_VOICE: [&str; 9] = [
    "",
    "S_CRYS4TWN",
    "S_CRYS4MNT",
    "S_CRYS4CST",
    "S_CRYS4SKY",
    "S_CRYS4FOR",
    "S_CRYS4DES",
    "S_CRYS4ICE",
    "S_CRYS4DRM",
];
const SECTION_VOICE: [&str; 3] = ["S_FNGS4WST", "S_FTHS4WST", "S_CLWS4BTL"];
/// AudioShardSpeech(n), n = 1..8 in world order.
const SHARD_VOICE: [&str; 9] = [
    "",
    "S_SHRD4TWN",
    "S_SHRD4MNT",
    "S_SHRD4CST",
    "S_SHRD4SKY",
    "S_SHRD4FOR",
    "S_SHRD4DES",
    "S_SHRD4ICE",
    "S_SHRD4DRM",
];

/// Portal waves the follow-up scenes reveal.
const TEMPLE_PORTAL: u32 = 0x500;
const HELL_PORTAL: u32 = 0x600;
const CITADEL_PORTAL: u32 = 0x803;

pub struct Sumner {
    /// The resident Sumner at the lectern, and his idle sequence (lbl_80344C90).
    gwiz: Option<u64>,
    gwiz_seq: usize,
    /// The speech Sumner, summoned beside the party.
    wizard: Option<u64>,
    /// C6C speech id, C68 hold after the text (s), C70 ticks, C74 camera started,
    /// E58 voice queued.
    speech: i32,
    hold: f32,
    ticks: i32,
    cam_started: bool,
    voiced: bool,
    caption: Caption,
    /// The caption to draw at y 312, if any.
    pub text: Option<String>,
    /// C78: no speech in the first 60 ticks.
    delay: i32,
    /// C7C follow-up step, C80 fade ticks and the objects fading in.
    follow: i32,
    fade: i32,
    fade_targets: Vec<u64>,
    /// E4C: the level-up check (>= 0 pending), the players to congratulate and
    /// the one being congratulated.
    level_check: i32,
    level_ups: Vec<usize>,
    level_up_voiced: bool,
    pub cam: Option<TriggerCam>,
    banked_runes: u16,
    banked_shards: u16,
    last_beaten: Option<u32>,
    welcome: bool,
    /// The welcome / Garm stories wait for the first frame (do_players).
    story_pending: bool,
    after_welcome: bool,
    /// C5C: when the orange crystals started to fade in.
    reveal_start: Option<f32>,
    /// C48: the last unlock message, and each barrier's last "need" message.
    msg_time: f32,
    need_time: [f32; 12],
    /// The lectern: its 0x10 latch, and when the hint menu opens (C58).
    latch: bool,
    hint_at: Option<f32>,
    /// The player the hint menu should open for (HintMenu).
    pub hint_request: Option<usize>,
    english: Option<TextRom>,
}

impl Sumner {
    /// SumnerInit + the EnterTower decorations: shards on the stained-glass
    /// window, runestones on their place, Sumner at his lectern.
    pub fn new(fs: &GameFs, g: &mut Game, entry: Entry) -> Sumner {
        let english = fs
            .read("TEXT/ENGLISH.ROM")
            .ok()
            .and_then(|d| TextRom::parse(&d).ok());
        let mut s = Sumner {
            gwiz: None,
            gwiz_seq: 0,
            wizard: None,
            speech: -1,
            hold: 0.0,
            ticks: 0,
            cam_started: false,
            voiced: false,
            caption: Caption::default(),
            text: None,
            delay: 60,
            follow: 0,
            fade: 0,
            fade_targets: Vec::new(),
            level_check: 0,
            level_ups: Vec::new(),
            level_up_voiced: false,
            cam: None,
            banked_runes: entry.banked_runes,
            banked_shards: entry.banked_shards,
            last_beaten: entry.last_beaten,
            welcome: entry.welcome,
            story_pending: true,
            after_welcome: false,
            reveal_start: None,
            msg_time: g.time + 1.0,
            need_time: [f32::MIN; 12],
            latch: false,
            hint_at: None,
            hint_request: None,
            english,
        };
        // GWIZ stands at waypoint 0 (FindLookoutParam(0)), behind the lectern.
        let lectern = waypoints(g).find(|l| l.delay == 0).cloned();
        if let Some(w) = lectern {
            if let Some(m) = g.model_named("GWIZ") {
                let id = g.spawn(Kind::Prop, Some(m), Vec3::from(w.pos), lookout_yaw(&w));
                s.gwiz = Some(id);
                if let Some(e) = g.entity_mut(id) {
                    e.animator.force_loop = false;
                }
            }
        }
        for n in 1..=8 {
            if s.banked_runes & (1 << n) != 0 {
                s.effect(g, &format!("SHARD{n}"), "L1WINDOWFRAME", true);
            }
        }
        for j in 0..12 {
            if s.banked_shards & (1 << j) != 0 {
                s.effect(g, &format!("RUNE{}", j + 1), "L1RUNEPLACE", true);
            }
        }
        if s.banked_shards & 0x1000 != 0 {
            s.effect(g, "RUNE13", "L1RUNE13", true);
        }
        if s.banked_runes & 0x1FE != 0x1FE {
            g.hide_object("L1XPLIGHTRAY01");
        }
        // While Sumner welcomes the party, the tower's orange crystals are hidden.
        if s.welcome {
            for e in &mut g.entities {
                if is_orange(&e.kind) {
                    e.fade = 0.0;
                }
            }
        }
        s
    }

    /// What tower.c has banked, for the app to keep.
    pub fn banked(&self) -> (u16, u16) {
        (self.banked_runes, self.banked_shards)
    }

    pub fn last_beaten(&self) -> Option<u32> {
        self.last_beaten
    }

    /// DisablePlayerControls: during speeches and trigger cameras.
    pub fn controls_locked(&self) -> bool {
        self.cam.is_some() || self.hold > 0.0
    }

    /// One 60 Hz tower frame.
    pub fn update(&mut self, g: &mut Game) {
        // do_players: the story on the first frame in the tower.
        if self.story_pending {
            self.story_pending = false;
            if self.welcome {
                self.story(g, "WELCOMEMESSAGE", None, None);
                self.after_welcome = true;
                return;
            }
            if self.last_beaten == Some(0x601) {
                self.story(g, "GARMMESSAGE", None, None);
                return;
            }
        }
        if self.after_welcome {
            self.after_welcome = false;
            self.gwiz_seq = 6;
            self.restart_gwiz(g);
            // CrystalCamActivate: locator 198 for 50 x 6 ticks.
            self.cam = locator_cam(g, 198).map(|(eye, dir)| TriggerCam {
                eye,
                dir,
                main: 300,
                hold: 0,
            });
            self.reveal_start = Some(g.time);
        }
        self.animate_gwiz(g);
        self.reveal_crystals(g);
        if let Some(c) = &mut self.cam {
            if c.main > 0 {
                c.main -= 1;
            } else if c.main == 0 && c.hold > 0 {
                c.hold -= 1;
            }
            if c.main == 0 && c.hold <= 0 && self.hold <= 0.0 {
                self.cam = None;
            }
        }
        self.follow_up(g);
        self.check_messages(g);
        self.do_speech(g);
        self.lectern(g);
        self.need_messages(g);
    }

    fn story(&mut self, g: &mut Game, name: &str, line: Option<usize>, sound: Option<&str>) {
        let Some(m) = g.scroll_rom.as_ref().and_then(|r| r.named(name)) else {
            return;
        };
        let pages = match line {
            Some(l) => m.lines.get(l).cloned().into_iter().collect(),
            None => m.lines.clone(),
        };
        g.scroll_box = Some(ScrollBox::new(None, pages, m.scale));
        if let Some(s) = sound {
            g.sfx.push(s.into());
        }
    }

    /// Start a decoration (custom effect `name` at world object `at`); `done`
    /// shows it as already played (EnterTower).
    fn effect(&mut self, g: &mut Game, name: &str, at: &str, done: bool) {
        let Some(pos) = g.object_pos(at) else {
            log::warn!("tower: no object {at}");
            return;
        };
        let Some(m) = g.model_named(name) else {
            log::warn!("tower: no effect {name}");
            return;
        };
        let id = g.spawn(Kind::Prop, Some(m), pos, 0.0);
        let frames = g.models[m]
            .skeleton
            .sequences
            .first()
            .map_or(0, |s| s.frame_count);
        if let Some(e) = g.entity_mut(id) {
            e.animator.force_loop = false;
            if done {
                e.animator.frame = frames.saturating_sub(1) as f32;
                e.animator.finished = true;
            }
        }
    }

    fn restart_gwiz(&mut self, g: &mut Game) {
        if let Some(e) = self.gwiz.and_then(|id| g.entity_mut(id)) {
            e.animator.restart(self.gwiz_seq);
        }
    }

    /// Idle READY -> READING -> THINKING; after a one-shot, READY.
    fn animate_gwiz(&mut self, g: &mut Game) {
        let Some(e) = self.gwiz.and_then(|id| g.entity_mut(id)) else {
            return;
        };
        if e.animator.finished {
            self.gwiz_seq = if self.gwiz_seq >= 2 {
                0
            } else {
                self.gwiz_seq + 1
            };
            e.animator.restart(self.gwiz_seq);
        }
    }

    /// From C5C on, each orange crystal within 15 x (1.75 + seconds) of the
    /// tower's centre fades in (alpha -8 a frame).
    fn reveal_crystals(&mut self, g: &mut Game) {
        let Some(t0) = self.reveal_start else { return };
        let r = 15.0 * (1.75 + (g.time - t0));
        let mut hidden = false;
        for e in &mut g.entities {
            if is_orange(&e.kind) && e.fade < 1.0 {
                if Vec3::new(e.pos.x, 0.0, e.pos.z).length() <= r {
                    e.fade = (e.fade + 8.0 / 255.0).min(1.0);
                }
                hidden = true;
            }
        }
        if !hidden {
            self.reveal_start = None;
        }
    }

    fn portal(g: &Game, wave: u32) -> Option<u64> {
        g.entities.iter().find_map(|e| match e.kind {
            Kind::Exit { dest: Some(w), .. } if w == wave => Some(e.id),
            _ => None,
        })
    }

    fn set_fade(g: &mut Game, id: Option<u64>, fade: f32) {
        if let Some(e) = id.and_then(|id| g.entity_mut(id)) {
            e.fade = fade;
        }
    }

    fn trigger_cam(&mut self, g: &Game, id: u16, hold: i32) {
        if let Some((eye, dir)) = locator_cam(g, id) {
            // A duration of 0 runs the main timer for 40 ticks.
            self.cam = Some(TriggerCam {
                eye,
                dir,
                main: 40,
                hold,
            });
        }
    }

    fn cam_done(&self) -> bool {
        self.cam.as_ref().is_none_or(|c| c.done())
    }

    fn hold_ticks(&self) -> i32 {
        self.cam.as_ref().map_or(0, |c| c.hold)
    }

    fn start_speech(&mut self, id: i32, hold: f32) {
        self.speech = id;
        self.hold = hold;
        self.ticks = 0;
        self.cam_started = false;
        self.voiced = false;
        self.caption = Caption::default();
        self.text = None;
    }

    /// The follow-up sequencer C7C (tower.c:1045-1170).
    fn follow_up(&mut self, g: &mut Game) {
        match self.follow {
            10 | 14 if self.hold_ticks() < 250 => {
                g.sfx.push("S_STNDGLASS".into());
                self.follow += 1;
            }
            11 if self.cam_done() => {
                self.trigger_cam(g, 204, 300);
                let portal = Self::portal(g, TEMPLE_PORTAL);
                Self::set_fade(g, portal, 0.0);
                self.fade_targets = portal.into_iter().collect();
                if g.show_object("L1XPLIGHTRAY01") {
                    g.world_changed = true;
                }
                self.fade = 180;
                self.follow = 116;
            }
            15 | 16 if self.cam_done() => {
                self.start_speech(self.follow, 2.0);
                self.follow = 0;
            }
            20 | 22 | 24 if self.hold_ticks() < 250 => {
                g.sfx.push("S_RUNEHIT".into());
                self.follow += 1;
            }
            21 => {
                let portal = Self::portal(g, HELL_PORTAL);
                Self::set_fade(g, portal, 0.0);
                self.fade_targets = portal.into_iter().collect();
                if self.cam_done() {
                    self.trigger_cam(g, 201, 300);
                    self.fade = 180;
                    self.follow = 126;
                }
            }
            23 | 33 => self.follow = 0,
            25 | 26 | 36 if self.cam_done() => {
                self.start_speech(self.follow + 100, 2.0);
                self.follow = 0;
            }
            30 | 32 if self.hold_ticks() < 220 => {
                g.sfx.push("S_RUNEHIT".into());
                self.follow += 1;
            }
            31 if self.cam_done() => {
                self.trigger_cam(g, 205, 300);
                g.activate_trigger(255);
                let portal = Self::portal(g, CITADEL_PORTAL);
                Self::set_fade(g, portal, 0.0);
                self.fade_targets = portal.into_iter().collect();
                self.fade = 180;
                self.follow = 136;
            }
            f if f >= 100 => {
                self.fade -= 1;
                let a = 1.0 - self.fade.max(0) as f32 / 180.0;
                for id in self.fade_targets.clone() {
                    Self::set_fade(g, Some(id), a);
                }
                if self.fade <= 0 {
                    self.follow -= 100;
                }
            }
            _ => {}
        }
    }

    /// TowerCheckMessages(1): level-ups, banking, and the unlock messages.
    fn check_messages(&mut self, g: &mut Game) {
        if self.delay > 0 {
            self.delay -= 1;
            return;
        }
        if self.cam.is_some() || self.hold > 0.0 || g.scroll_box.is_some() {
            return;
        }
        self.check_level_up(g);
        if self.hold > 0.0 {
            return;
        }
        // Banking: what the party carries back joins the tower's record.
        let new_runes = g.runes & !self.banked_runes;
        let new_shards = g.shards & !self.banked_shards;
        let had_shards = self.banked_shards;
        self.banked_runes |= g.runes;
        self.banked_shards |= g.shards;
        if let Some(n) = (1..=8).find(|n| new_runes & (1 << n) != 0) {
            self.start_speech(n, 3.0);
        }
        if let Some(s) = (0..13).find(|s| new_shards & (1 << s) != 0) {
            self.start_speech(100 + s, 3.0);
        } else if self.last_beaten == Some(0x802) && self.banked_shards & 0x1000 == 0 {
            self.last_beaten = None;
            self.start_speech(113, 3.0);
        } else if had_shards & 0xFFF == 0xFFF && new_runes & 0x200 != 0 {
            self.speech = 114;
            self.speech_end(g);
        }
        if self.hold > 0.0 {
            return;
        }
        // UnlockLevel / UnlockSection: 4 s after entering, then 3 s apart.
        if g.time - self.msg_time < 3.0 {
            return;
        }
        if let Some(j) = (1..9).find(|&j| g.crystals[j] == CRYSTALS_NEEDED[j]) {
            g.crystals[j] = -1;
            self.story(g, "UNLOCKLEVEL", Some(j), Some(CRYSTAL_VOICE[j]));
            self.msg_time = g.time;
        } else if let Some(j) = (0..3).find(|&j| g.garg_items[j] == GARG_NEEDED[j]) {
            g.garg_items[j] = -1;
            self.story(g, "UNLOCKSECTION", Some(j), Some(SECTION_VOICE[j]));
            self.msg_time = g.time;
        }
    }

    /// sumnerCheckLevelUp: who crossed a multiple of ten levels (or reached 99)
    /// since the last check.
    fn check_level_up(&mut self, g: &mut Game) {
        if self.level_check < 0 {
            return;
        }
        let mut ups = Vec::new();
        for (i, p) in g.players.iter_mut().enumerate() {
            if !p.state.active() {
                continue;
            }
            let old = crate::stats::exp_to_level(p.sumner_exp);
            let new = crate::stats::exp_to_level(p.exp);
            if (new >= 99 && old < 99) || old / 10 != new / 10 {
                ups.push(i);
            }
            p.sumner_exp = p.exp;
        }
        if !ups.is_empty() {
            self.start_speech(-1, 0.5);
            self.level_ups = ups;
            self.level_up_voiced = false;
        }
    }

    /// SumnerDoSpeech.
    fn do_speech(&mut self, g: &mut Game) {
        if self.hold <= 0.0 {
            return;
        }
        if !self.cam_started {
            self.cam_started = true;
            self.summon(g);
        }
        self.ticks += 1;
        let elapsed = self.ticks - 120;
        if elapsed < 0 {
            return;
        }
        let done = if !self.level_ups.is_empty() {
            self.level_up(g, elapsed);
            false
        } else {
            self.speech_text(g, elapsed)
        };
        if done {
            self.hold -= 1.0 / 60.0;
            if self.hold <= 0.0 {
                self.speech_end(g);
            }
        }
    }

    /// towerSummonWizard + SumnerCamActivate: Sumner appears at the waypoint
    /// nearest the party, seen from that waypoint's camera of the set.
    fn summon(&mut self, g: &mut Game) {
        let alive: Vec<Vec3> = g
            .players
            .iter()
            .filter(|p| !matches!(p.state, PlayerState::InTower | PlayerState::Exited))
            .map(|p| p.pos)
            .collect();
        let avg = alive.iter().copied().sum::<Vec3>() / alive.len().max(1) as f32;
        let Some(w) = waypoints(g)
            .min_by(|a, b| {
                let d = |l: &gdl_formats::worlds::Locator| (Vec3::from(l.pos) - avg).length();
                d(a).total_cmp(&d(b))
            })
            .cloned()
        else {
            return;
        };
        if let Some(m) = g.model_named("WIZARD") {
            self.wizard = Some(g.spawn(Kind::Prop, Some(m), Vec3::from(w.pos), lookout_yaw(&w)));
            // At the lectern the speech Sumner takes the resident's place.
            if w.delay == 0 {
                Self::set_fade(g, self.gwiz, 0.0);
            }
        }
        let base = if !self.level_ups.is_empty() {
            240
        } else if self.speech >= 100 {
            170
        } else {
            220
        };
        let wp = w.delay as u16;
        let id = base + wp;
        let id = if locator_cam(g, id).is_some() {
            id
        } else {
            220 + wp
        };
        if let Some((eye, dir)) = locator_cam(g, id) {
            self.cam = Some(TriggerCam {
                eye,
                dir,
                main: -1,
                hold: 0,
            });
        }
    }

    /// The speech's text and voice; true once the caption is done.
    fn speech_text(&mut self, g: &mut Game, elapsed: i32) -> bool {
        let (name, line, voice) = match self.speech {
            n @ 1..=8 => ("NEWSHARDS", Some(n as usize), SHARD_VOICE[n as usize]),
            15 => ("MORESHARDS", None, "S_CONTINUEVOX"),
            16 => ("ALLSHARDS", None, "S_4KEYVOX"),
            100..=112 => ("NEWRUNES", Some(0), "S_FNDRUNEYOU"),
            113 => ("RUNE13NO", None, "S_RUNE13NO"),
            125 => ("ALL12RUNESNO", None, "S_12RUNENO"),
            126 => ("ALL12RUNESYES", None, "S_12RUNEYES"),
            136 => ("RUNE13YES", None, "S_RUNE13YES"),
            _ => return true,
        };
        if self.caption.lines.is_empty() {
            let Some(m) = g.scroll_rom.as_ref().and_then(|r| r.named(name)) else {
                return true;
            };
            self.caption.lines = match line {
                Some(l) => m.raw.get(l).cloned().into_iter().collect(),
                None => m.raw.clone(),
            };
            self.caption.start = elapsed;
        }
        if !self.voiced {
            self.voiced = true;
            g.sfx.push(voice.into());
        }
        let c = &mut self.caption;
        let Some(text) = c.lines.get(c.line) else {
            return true;
        };
        let (shown, full) = typewrite(text, ((elapsed - c.start) >> 1) as f32);
        self.text = Some(shown);
        if !full {
            return false;
        }
        if c.line + 1 >= c.lines.len() {
            return true;
        }
        // caption_timer: 60 ticks, then the next line.
        c.wait += 1;
        if c.wait >= 60 {
            c.wait = 0;
            c.line += 1;
            c.start = elapsed;
        }
        false
    }

    /// The level-up congratulations, one player after another.
    fn level_up(&mut self, g: &mut Game, elapsed: i32) {
        let Some(&pi) = self.level_ups.first() else {
            return;
        };
        if self.caption.lines.is_empty() {
            self.caption.lines = vec![self.level_up_line(g, pi)];
            self.caption.start = elapsed;
            self.level_up_voiced = false;
        }
        let t = elapsed - self.caption.start;
        if !self.level_up_voiced {
            self.level_up_voiced = true;
            if let Some(p) = g.players.get(pi) {
                let lv = p.level.min(99);
                g.sfx.push(if lv >= 99 {
                    "S_EXP99ALL".into()
                } else {
                    format!("S_EXP{}{}", lv / 10 * 10, p.class)
                });
            }
        }
        let (shown, full) = typewrite(&self.caption.lines[0], (t >> 1) as f32);
        self.text = Some(shown);
        // (StartLevelUpFX at 240 and StartGemFX at 270 have no effects yet.)
        if full && t >= 360 {
            self.level_ups.remove(0);
            self.caption = Caption::default();
            if self.level_ups.is_empty() {
                self.level_check = -1;
                self.text = None;
            }
        }
    }

    /// "%s %s is now\na level %d %s!\n" with the colour, class and rank.
    fn level_up_line(&self, g: &Game, pi: usize) -> String {
        let p = &g.players[pi];
        let rom = self.english.as_ref();
        let get = |msg: &str, line: usize| -> String {
            rom.and_then(|r| r.named(msg))
                .and_then(|m| m.lines.get(line).cloned())
                .unwrap_or_default()
        };
        let colour = match p.color.as_str() {
            "YEL" => 0,
            "BLU" => 1,
            "RED" => 2,
            _ => 3,
        };
        let lv = p.level.min(99);
        let rank = if lv >= 99 {
            get("LEGEND", 0)
        } else {
            // String list 0 names each character's rank message.
            rom.and_then(|r| {
                let msg = *r.lists.first()?.get(p.class_id)?;
                r.messages
                    .get(msg as usize)?
                    .lines
                    .get(((lv / 10) >> 1) as usize)
                    .cloned()
            })
            .unwrap_or_default()
        };
        format!(
            "{} {} is now\na level {} {}!\n",
            get("PLAYER_COLOR_LC", colour),
            get("PLAYER_CLASS_LC", p.class_id),
            lv,
            rank
        )
    }

    /// SumnerSpeechEnd: Sumner leaves, and the shard or runestone takes its place.
    fn speech_end(&mut self, g: &mut Game) {
        if let Some(e) = self.wizard.take().and_then(|id| g.entity_mut(id)) {
            e.alive = false;
        }
        Self::set_fade(g, self.gwiz, 1.0);
        self.cam = None;
        self.hold = 0.0;
        self.text = None;
        self.level_ups.clear();
        match self.speech {
            n @ 1..=8 => {
                self.effect(g, &format!("SHARD{n}"), "L1WINDOWFRAME", false);
                self.trigger_cam(g, 202, 300);
                if g.hide_object("L1XPLIGHTRAY01") {
                    g.world_changed = true;
                }
                if self.banked_runes & 0x1FE == 0x1FE {
                    let portal = Self::portal(g, TEMPLE_PORTAL);
                    Self::set_fade(g, portal, 0.0);
                    self.follow = 10;
                    g.sfx.push("S_SHRDS127".into());
                } else {
                    self.follow = 14;
                    g.sfx.push("S_SHRD8".into());
                }
            }
            s @ 100..=111 => {
                self.effect(g, &format!("RUNE{}", s - 99), "L1RUNEPLACE", false);
                self.trigger_cam(g, 201, 300);
                g.sfx.push("S_RUNEFALL".into());
                self.follow =
                    if self.banked_runes & 0x3FE == 0x3FE && self.banked_shards & 0xFFF == 0xFFF {
                        20
                    } else if self.banked_shards & 0xFFF == 0xFFF {
                        24
                    } else {
                        22
                    };
                let portal = Self::portal(g, HELL_PORTAL);
                Self::set_fade(g, portal, 0.0);
            }
            112 => {
                self.effect(g, "RUNE13", "L1RUNE13", false);
                self.trigger_cam(g, 203, 300);
                let portal = Self::portal(g, CITADEL_PORTAL);
                Self::set_fade(g, portal, 0.0);
                self.follow = if self.banked_shards & 0x1FFF == 0x1FFF {
                    30
                } else {
                    32
                };
            }
            // Runes 13-14 place nothing; the next step is as for the others
            // (20 becomes 21, or none once all eleven runes are in), and any
            // step fades the hell portal (fn_8005B558(0x600)).
            113 | 114 => {
                let shards = self.banked_shards & 0xFFF == 0xFFF;
                self.follow = if self.banked_runes & 0x3FE == 0x3FE && shards {
                    if self.banked_runes & 0x7FE != 0x7FE {
                        21
                    } else {
                        0
                    }
                } else if shards {
                    24
                } else {
                    22
                };
                if self.follow != 0 {
                    let portal = Self::portal(g, HELL_PORTAL);
                    Self::set_fade(g, portal, 0.0);
                }
            }
            _ => {}
        }
        self.speech = -1;
    }

    /// SumnerAnimate: a player on the lectern gets a WELCOME, then the hint menu.
    fn lectern(&mut self, g: &mut Game) {
        let on = g.entities.iter().find_map(|e| match e.kind {
            Kind::Trigger {
                id: LECTERN,
                radius,
                ..
            } => g.players.iter().position(|p| {
                p.state.active()
                    && glam::Vec2::new(p.pos.x - e.pos.x, p.pos.z - e.pos.z).length() < radius + 1.5
                    && (p.pos.y - e.pos.y).abs() < 4.0
            }),
            _ => None,
        });
        let Some(player) = on else {
            self.latch = false;
            self.hint_at = None;
            return;
        };
        if self.latch || self.controls_locked() {
            return;
        }
        match self.hint_at {
            None => {
                self.gwiz_seq = 3;
                self.restart_gwiz(g);
                self.hint_at = Some(g.time + 2.0);
            }
            Some(t) if g.time >= t => {
                self.hint_at = None;
                self.latch = true;
                self.hint_request = Some(player);
            }
            _ => {}
        }
    }

    /// The hint menu was closed with Back: GOAWAY.
    pub fn hints_closed(&mut self, g: &mut Game) {
        self.gwiz_seq = 4;
        self.restart_gwiz(g);
    }

    /// TowerNeedCrystalsMsg / TowerNeedGargItemsMsg: 10 s apart per barrier.
    fn need_messages(&mut self, g: &mut Game) {
        let touches = std::mem::take(&mut g.barrier_touches);
        if self.controls_locked() || g.scroll_box.is_some() {
            return;
        }
        for gate in touches {
            let (slot, name, line) = match gate {
                101..=103 => (8 + gate - 100, "NEEDGARGITEMS", gate - 101),
                c => (c, "NEEDCRYSTALS", c),
            };
            let Some(t) = self.need_time.get_mut(slot) else {
                continue;
            };
            if g.time - *t < 10.0 {
                continue;
            }
            *t = g.time;
            self.story(g, name, Some(line), None);
            break;
        }
    }
}

fn is_orange(k: &Kind) -> bool {
    matches!(
        k,
        Kind::Pickup {
            kind: crate::game::PickupKind::Crystal(1),
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tower() -> Option<(GameFs, Game)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let fs = GameFs::open(&root).ok()?;
        let g = Game::new(&fs, "levelL1", &[("WAR", "RED")]).ok()?;
        Some((fs, g))
    }

    fn run(g: &mut Game, ticks: u32) {
        for _ in 0..ticks {
            if let Some(b) = &mut g.scroll_box {
                let inp = crate::input::PlayerInput {
                    b_pressed: b.ticks >= 20,
                    ..Default::default()
                };
                if b.update(&[inp]) {
                    g.scroll_box = None;
                }
                continue;
            }
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
        }
    }

    #[test]
    fn a_shard_brought_back_is_banked_with_a_speech() {
        let Some((fs, mut g)) = tower() else { return };
        g.runes = 0x2;
        let s = Sumner::new(&fs, &mut g, Entry::default());
        assert!(s.gwiz.is_some(), "Sumner stands at his lectern");
        g.sumner = Some(Box::new(s));
        run(&mut g, 61 + 120 + 30);
        let s = g.sumner.as_ref().unwrap();
        assert_eq!(s.speech, 1);
        assert!(s.controls_locked() && s.wizard.is_some() && s.cam.is_some());
        assert!(s.text.as_deref().is_some_and(|t| t.starts_with("Congrat")));
        // Typing, 3 s, then the window scene and "Continue now on your quest".
        run(&mut g, 2000);
        let s = g.sumner.as_ref().unwrap();
        assert_eq!(s.banked(), (0x2, 0));
        assert_eq!((s.speech, s.follow), (-1, 0));
        assert!(!s.controls_locked());
        assert!(g
            .model_named("SHARD1")
            .is_some_and(|m| g.entities.iter().any(|e| e.model == Some(m) && e.alive)));
    }

    /// After a reload, what was banked stays banked: only a newly brought
    /// rune starts a speech.
    #[test]
    fn reloaded_banks_only_speak_about_new_runes() {
        let Some((fs, mut g)) = tower() else { return };
        g.runes = 0x2;
        g.shards = 0x1;
        let entry = Entry {
            banked_runes: 0x2,
            banked_shards: 0x1,
            ..Entry::default()
        };
        g.sumner = Some(Box::new(Sumner::new(&fs, &mut g, entry)));
        run(&mut g, 61 + 400);
        let s = g.sumner.as_ref().unwrap();
        assert_eq!(s.speech, -1, "nothing new to bank");
        assert!(g.scroll_box.is_none() && !s.controls_locked());
        // A second rune brought back now: its speech plays.
        g.runes |= 0x4;
        run(&mut g, 30);
        let s = g.sumner.as_ref().unwrap();
        assert_eq!(s.speech, 2);
        assert_eq!(s.banked(), (0x6, 0x1));
    }

    #[test]
    fn the_lectern_opens_the_hint_menu() {
        let Some((fs, mut g)) = tower() else { return };
        let s = Sumner::new(&fs, &mut g, Entry::default());
        g.sumner = Some(Box::new(s));
        let pad = g
            .entities
            .iter()
            .find(|e| matches!(e.kind, Kind::Trigger { id: LECTERN, .. }))
            .map(|e| e.pos)
            .unwrap();
        g.players[0].pos = pad;
        run(&mut g, 30);
        assert_eq!(g.sumner.as_ref().unwrap().gwiz_seq, 3, "WELCOME");
        run(&mut g, 100);
        assert_eq!(g.sumner.as_mut().unwrap().hint_request.take(), Some(0));
        // Once only, until the player steps off.
        run(&mut g, 200);
        assert_eq!(g.sumner.as_ref().unwrap().hint_request, None);
    }

    #[test]
    fn met_crystals_unlock_with_a_message() {
        let Some((fs, mut g)) = tower() else { return };
        g.crystals[1] = CRYSTALS_NEEDED[1];
        let s = Sumner::new(&fs, &mut g, Entry::default());
        g.sumner = Some(Box::new(s));
        run(&mut g, 60 * 3);
        assert!(g.scroll_box.is_none());
        run(&mut g, 70);
        assert_eq!(g.crystals[1], -1);
    }

    /// SumnerSpeechEnd for speeches 113/114: the follow step runs whether
    /// or not every rune is in.
    #[test]
    fn late_rune_speeches_pick_their_follow_step() {
        let Some((fs, mut g)) = tower() else { return };
        let mut s = Sumner::new(&fs, &mut g, Entry::default());
        for (runes, shards, follow) in [
            (0x7FE, 0xFFF, 0),
            (0x3FE, 0xFFF, 21),
            (0x002, 0xFFF, 24),
            (0x3FE, 0x001, 22),
        ] {
            s.banked_runes = runes;
            s.banked_shards = shards;
            s.speech = 113;
            s.speech_end(&mut g);
            assert_eq!((s.speech, s.follow), (-1, follow), "{runes:#x} {shards:#x}");
        }
    }

    #[test]
    fn typewriter_costs() {
        assert_eq!(typewrite("ab", 3.5), ("ab".into(), true));
        assert_eq!(typewrite("a.b", 3.5).0, "a");
        // A page break clears what was shown and costs 30.
        assert_eq!(
            typewrite("Hi\r\nYou", 3.5 + 30.0 + 1.75),
            ("\n".into(), false)
        );
        assert!(typewrite("Hi\tthere", 100.0).0 == "Hithere");
    }

    /// The podium's stairs (L1STAIRS_PODIUM, flag 8) are climbed, and the
    /// faded-out lit podium (a trigger target that also has a world
    /// animation) has no collision: a hero walks from the tower floor up to
    /// the lectern and Sumner welcomes them.
    #[test]
    fn a_hero_walks_up_the_podium_stairs_to_the_lectern() {
        let Some((fs, mut g)) = tower() else { return };
        let s = Sumner::new(&fs, &mut g, Entry::default());
        g.sumner = Some(Box::new(s));
        g.players[0].pos = Vec3::new(3.3, -0.27, -32.0);
        let mut top = f32::MIN;
        for _ in 0..240 {
            let stick = g.stick_toward(glam::Vec2::new(0.0, -1.0));
            let inp = crate::input::PlayerInput {
                stick,
                ..Default::default()
            };
            g.update(1.0 / 60.0, &[inp]);
            top = top.max(g.players[0].pos.y);
            if g.sumner.as_ref().unwrap().gwiz_seq == 3 {
                break;
            }
        }
        let p = g.players[0].pos;
        assert!(top > 1.2 && p.z < -48.0, "up the stairs: {p:?}");
        assert_eq!(g.sumner.as_ref().unwrap().gwiz_seq, 3, "WELCOME at {p:?}");
    }
}
