//! Levers, pressure pads and the world geometry they drive (items.c: trigger items,
//! RegisterItemWobj and the per-frame world-object update). A trigger's target is a
//! world object that slides up or down between two heights, or fades in and out.

use crate::collision::Collision;
use crate::level::Level;
use glam::Vec3;

/// Rate moving world objects slide at (lbl_803470F0, units per second).
const SLIDE_SPEED: f32 = 4.0;
/// Toggle triggers wait this long before they can flip their target again (0x78 ticks).
const TOGGLE_IDLE: f32 = 2.0;

/// How a trigger drives its target (triggerdata.flags & 7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TriggerMode {
    /// flags & 1: deactivates the target while held.
    Release,
    /// flags & 2: activates once and stays.
    Latch,
    /// flags & 4: each press flips the target.
    Toggle,
    /// Otherwise: activates the target.
    Set,
}

/// Trigger behaviour from an item subtype and its instance flags (the table in the
/// item constructor, items.c case 5). Returns the mode and the target's flag byte.
pub fn trigger_flags(subtype: i32, params_flags: u16) -> (TriggerMode, u8) {
    let base: u16 = match subtype {
        20 => 0x10,  // bridge_pad
        21 => 8,     // door_pad
        22 => 0x12,  // bridge_switch
        23 => 10,    // door_switch
        25 => 0x804, // elevator_pad
        26 => 2,     // elevator_switch
        27 => 0x80C, // lift_pad
        28 => 9,     // lift_start
        29 => 10,    // lift_end
        _ => params_flags | 8,
    };
    let flags = base | (params_flags & !0xFF);
    let mode = if flags & 1 != 0 {
        TriggerMode::Release
    } else if flags & 2 != 0 {
        TriggerMode::Latch
    } else if flags & 4 != 0 {
        TriggerMode::Toggle
    } else {
        TriggerMode::Set
    };
    (mode, (flags & 0xFF) as u8)
}

/// A world object (with its children) moved by triggers.
pub struct Mover {
    pub root: usize,
    pub objects: Vec<usize>,
    /// Collision triangles that move with it, and their resting vertices.
    tris: Vec<(usize, [Vec3; 3])>,
    /// Current vertical offset from the modelled position.
    pub offset: f32,
    off: f32,
    on: f32,
    /// Triggered (triggerstate bit 0x20).
    pub active: bool,
    /// A player is holding a trigger for it this frame (triggerstate low bits).
    pub held: bool,
    /// Target flag byte from the trigger subtype.
    flags8: u8,
    pub moving: bool,
    /// Drawn (fading objects disappear when not triggered).
    pub shown: bool,
    /// Where it is, for sounds.
    pub pos: Vec3,
}

impl Mover {
    pub fn new(
        level: &Level,
        collision: &Collision,
        root: usize,
        flags8: u8,
        off: f32,
        on: f32,
    ) -> Mover {
        let objects = level.subtree(root);
        let tris = collision
            .tris
            .iter()
            .enumerate()
            .filter(|(_, t)| t.owner >= 0 && objects.contains(&(t.owner as usize)))
            .map(|(i, t)| (i, t.v))
            .collect();
        Mover {
            root,
            objects,
            tris,
            offset: off,
            off,
            on,
            active: false,
            held: false,
            flags8,
            moving: false,
            shown: flags8 & 0x10 == 0,
            pos: Vec3::from(level.world_pos[root]),
        }
    }

    /// Fades in and out instead of sliding (target flag 0x10).
    pub fn fades(&self) -> bool {
        self.flags8 & 0x10 != 0
    }

    /// Momentary targets forget their state every frame unless a trigger holds them.
    fn momentary(&self) -> bool {
        self.flags8 & 0x47 == 0
    }

    /// Apply a trigger touched (or not) this frame. Returns true if it changed the target.
    pub fn trigger(&mut self, mode: TriggerMode, pressed: bool, idle: &mut f32) -> bool {
        let was = self.active;
        if pressed {
            self.held = true;
        }
        match mode {
            TriggerMode::Release => {
                if pressed {
                    self.active = false;
                }
            }
            TriggerMode::Latch | TriggerMode::Set => {
                if pressed {
                    self.active = true;
                }
            }
            TriggerMode::Toggle => {
                if pressed {
                    if self.moving {
                        *idle = TOGGLE_IDLE;
                    } else if *idle <= 0.0 {
                        self.active = !self.active;
                        *idle = TOGGLE_IDLE;
                    }
                }
            }
        }
        self.active != was
    }

    /// Slide toward the active/inactive height and move the collision with it.
    /// Returns (started, stopped) moving this frame.
    pub fn update(&mut self, collision: &mut Collision, dt: f32) -> (bool, bool) {
        let was_moving = self.moving;
        if self.fades() {
            self.shown = self.active || self.held;
            for &(i, _) in &self.tris {
                collision.enabled[i] = self.shown;
            }
            self.moving = false;
        } else {
            let target = if self.active { self.on } else { self.off };
            let delta = target - self.offset;
            let step = SLIDE_SPEED * dt;
            self.moving = delta.abs() > 0.001;
            if self.moving {
                self.offset += delta.clamp(-step, step);
                for &(i, rest) in &self.tris {
                    collision.tris[i].v = rest.map(|v| v + Vec3::Y * self.offset);
                }
            } else if self.flags8 & 0x20 != 0 {
                // Ping-pong targets turn around at each end.
                self.active = !self.active;
            }
        }
        if self.momentary() {
            self.active = false;
        }
        self.held = false;
        (self.moving && !was_moving, was_moving && !self.moving)
    }

    /// Put the collision at the starting offset.
    pub fn place(&self, collision: &mut Collision) {
        for &(i, rest) in &self.tris {
            collision.tris[i].v = rest.map(|v| v + Vec3::Y * self.offset);
        }
    }
}
