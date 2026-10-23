//! sfx.c's effect table and the Start* family: InitEffects' EffectInfo rows
//! (lbl_80122118, then the monster, item and wizard trees), StartFXSub /
//! StartFXTree with each row's zmod and alpha, the morph chains
//! (SfxSetMorph, ChangeEffect) and the ProcessEffects motion of what is
//! started here: monster hit and death bursts (fn_800945D0), generator hits
//! (StartGenHitFx), spawn puffs (StartGenFX), Death's drain (StartDeathFX),
//! block sparks (StartBlockFX), Garm's attack and death effects
//! (StartEnemyAtkFX, StartEnemyDeathFX), the level entry (StartEnterFX),
//! missile hit morphs (ProcessEffects' fxhit / hitmorph), world and suicide
//! explosions (WorldExplosion, SuicideExplosion).
//!
//! A child of `game`, so it reaches the game's private state.

use super::*;

/// fx_type values used here (effect.h).
pub(crate) const FX_EXPLODE: usize = 22;
pub(crate) const FX_EXPPOISON: usize = 25;
pub(crate) const FX_EXPPOISON2: usize = 26;
pub(crate) const FX_EXPPOISON3: usize = 27;
pub(crate) const FX_EXPRING: usize = 28;
pub(crate) const FX_GENDEST: usize = 30;
/// lbl_80122E60: StartLevelUpFX's rows by class_id (the colour): YEL, BLU,
/// RED, GRE.
pub(crate) const FX_LEVELUP: usize = 57;
pub(crate) const FX_BLOCK: usize = 66;
pub(crate) const FX_ENTER: usize = 67;
pub(crate) const FX_SUICIDEEXP: usize = 80;
pub(crate) const FX_GENFX1: usize = 81;
pub(crate) const FX_ALTHIT: usize = 84;
pub(crate) const FX_ALTDIE: usize = 85;
pub(crate) const FX_ENEATK1: usize = 86;
pub(crate) const FX_ENEDEATH1: usize = 88;
pub(crate) const FX_ENEDEATH2: usize = 89;
pub(crate) const FX_LEGEND1: usize = 90;
pub(crate) const FX_LEGEND2: usize = 91;
pub(crate) const FX_LEGENDHLD: usize = 92;
pub(crate) const FX_LEGENDPRJ: usize = 93;
pub(crate) const FX_WORLD_EXP: usize = 94;
pub(crate) const FX_DEATH_HEALTH: usize = 95;
pub(crate) const FX_DEATH_EXP: usize = 96;
pub(crate) const FX_CUSTOM1: usize = 97;
/// MAXEFFECTTYPES.
pub(crate) const MAX_FX: usize = 218;
/// InitCustomEffectSub's limit on custom rows.
const MAX_CUSTOM: usize = 121;

/// lbl_80122118: InitEffects' 80 authoring rows (EFFECTINFO desc, zmod,
/// alpha), matched in WEAPONS, else POWERUPS.
const ROWS: [(&str, i32, i32); 80] = [
    ("NULLFX", 0, 0),
    ("SPARKS", -128, 96),
    ("HITCOL", -128, 96),
    ("HITDIE", -128, 96),
    ("BLOODHIT", -128, 96),
    ("BLOODDIE", -128, 96),
    ("BLOODFX1", -128, 96),
    ("BLOODFX1", -128, 96),
    ("BLOODFX2", -128, 96),
    ("BLOODFX2", -128, 96),
    ("FIREHIT", -128, 96),
    ("FIREHIT", -128, 96),
    ("FIREDIE", -128, 96),
    ("HITCOL", -128, 96),
    ("HITCOL", -128, 96),
    ("ELECDIE", -128, 96),
    ("HITCOL", -128, 96),
    ("HITCOL", -128, 96),
    ("LIGHTDIE", -128, 96),
    ("HITCOL", -128, 96),
    ("HITCOL", -128, 96),
    ("ACIDDIE", -128, 96),
    ("EXPLOSION", -128, 0),
    ("EXPSMALL", -128, 0),
    ("EXPLOSION", -128, 0),
    ("POISONEXP1", -128, 0),
    ("POISONEXP2", -128, 0),
    ("POISONEXP3", -128, 0),
    ("EXPRING", -128, 0),
    ("EXPCHEST", -128, 0),
    ("GENDEST", -128, 0),
    ("CHESTDEST", -128, 0),
    ("CHESTDEST", -128, 0),
    ("DESTSMOKE", -128, 0),
    ("MAGICFX", 0, 0),
    ("MP_FIRE", -128, 0),
    ("MP_ELEC", -128, 0),
    ("MP_LIGHT", -128, 0),
    ("MP_ACID", -128, 0),
    ("MS_FIRE", -128, 0),
    ("MS_ELEC", -128, 0),
    ("MS_LIGHT", -128, 0),
    ("MS_ACID", -128, 0),
    ("POT_RED_TW", -128, 0),
    ("POT_BLU_TW", -128, 0),
    ("POT_YEL_TW", -128, 0),
    ("POT_GRE_TW", -128, 0),
    ("LEVELUP_RED", -128, 0),
    ("LEVELUP_YEL", -128, 0),
    ("LEVELUP_BLU", -128, 0),
    ("LEVELUP_GRE", -128, 0),
    ("MAGICHEALTH", -128, 0),
    ("FIREBREATHE", 0, 0),
    ("ACIDBREATHE", 0, 0),
    ("ELECBREATHE", 0, 0),
    ("L_SHLD_ACTIVE", 0, 0),
    ("BOSS_BREATHE", 0, 0),
    ("LEVELUP_YEL", -512, 0),
    ("LEVELUP_BLU", -512, 0),
    ("LEVELUP_RED", -512, 0),
    ("LEVELUP_GRE", -512, 0),
    ("COMBO_SPH", -512, 0),
    ("COMBO_YEL", -512, 0),
    ("COMBO_BLU", -512, 0),
    ("COMBO_RED", -512, 0),
    ("COMBO_GRN", -512, 0),
    ("BLOCKFX", -512, 0),
    ("STARTFX", -512, 0),
    ("BAG_THROW", 0, 0),
    ("BAG_HIT", 0, 0),
    ("GETGEMORANGE", -512, 0),
    ("GETGEMRED", -512, 0),
    ("GETGEMPURPLE", -512, 0),
    ("GETGEMBLUE", -512, 0),
    ("GETGEMGREEN", -512, 0),
    ("GETGEMYELLOW", -512, 0),
    ("GETGEMWHITE", -512, 0),
    ("GETGEMBLACK", -512, 0),
    ("GETGARG", -512, 0),
    ("GETRUNE", -512, 0),
];

/// A hero's class_id (colour index) by colour code: YEL 0, BLU 1, RED 2, GRE 3.
pub(crate) fn colour_index(color: &str) -> usize {
    match color {
        "YEL" => 0,
        "BLU" => 1,
        "RED" => 2,
        _ => 3,
    }
}

/// MagicView.hitmorph (lbl_80122D98): what a missile bursting on a wall
/// becomes, by element (SPARKS, FIREHIT, then HITCOL).
pub(crate) const HITMORPH: [usize; 5] = [1, 10, 13, 16, 19];
/// MagicView.kindidA..F (0x80122DAC..0x80122E10): fn_800945D0's hit and die
/// rows by element — A/B without blood (golems, knights), C/D with it (4 and
/// 5 alternate between the BLOODFX pairs), E/F for treefolk and acid blobs.
const KIND_A: [usize; 5] = [2, 11, 14, 17, 20];
const KIND_B: [usize; 5] = [3, 12, 15, 18, 21];
const KIND_C: [usize; 5] = [4, 11, 14, 17, 20];
const KIND_D: [usize; 5] = [5, 12, 15, 18, 21];
const KIND_E: [usize; 5] = [84, 11, 14, 17, 20];
const KIND_F: [usize; 5] = [85, 12, 15, 18, 21];
/// lbl_8011A178 (0xFFFF00, 0xFF, 0xFF0000, 0xFF00 in main.dol): the tint
/// by Player.class_id, which in the original (+0x4) is the costume colour
/// (save.last_color; reset_players sets it to the slot) — yellow, blue,
/// red, green — not this port's class_id (the character).
pub(crate) const COLOUR_TINT: [u32; 4] = [0xFF_FF00, 0x00_00FF, 0xFF_0000, 0x00_FF00];

/// ene_attn / ene_coll (0x8011B768 / 0x8011B7F0): each monster type's
/// attention and collision heights.
const ENE_ATTN: [f32; 34] = [
    2.0, 3.8, 3.8, 2.0, 3.8, 3.8, 2.0, 3.8, 3.8, 2.0, 3.8, 3.8, 2.0, 3.8, 3.8, 2.0, 3.8, 3.8, 2.0,
    3.8, 3.8, 0.5, 0.5, 3.8, 3.8, 3.8, 3.8, 3.8, -1.0, 5.0, 3.0, 3.0, 5.0, 4.0,
];
const ENE_COLL: [f32; 34] = [
    1.5, 3.0, 3.0, 1.5, 3.0, 3.0, 1.5, 3.0, 3.0, 1.5, 3.0, 3.0, 1.5, 3.0, 3.0, 1.5, 3.0, 3.0, 1.5,
    3.0, 3.0, 1.5, 1.5, 3.0, 3.0, 3.0, 3.0, 4.0, -1.0, 4.0, 3.0, 3.0, 3.0, 3.0,
];
/// DMG_NOHITFX.
const DMG_NOHITFX: u32 = 0x100_0000;
/// enemy_dies: the suicide grunt (algorithm 18) blows up for 50 x ene_damage.
const SUICIDE_DAMAGE: f32 = 50.0;

/// sMusicTrackHi 9 and 11: WorldExplosion's FX_WORLD_EXP (poison gas, radius
/// 6), else EXPLOSION (fire and knockdown, radius 5, x1.5 wide); 50 damage
/// (lbl_80346BE0..BF4).
const WORLD_EXP_DAMAGE: f32 = 50.0;
/// StartEnemyDeathFX: Garm's death shot flies at 20 along its facing with a
/// colrad of 3, doing 50 (SUPER | KNOCKDOWN), and becomes DEATHFX2 after 3 s.
const ENEDEATH_SPEED: f32 = 20.0;
const ENEDEATH_COLRAD: f32 = 3.0;
const ENEDEATH_DAMAGE: f32 = 50.0;
const ENEDEATH_TIME: f32 = 3.0;
const ENEDEATH_TYPE: u32 = 0x10_0020;
/// StartBagFX's launch: up at 20, pitching at pi/12 + Random(pi/3), falling
/// at 50 (lbl_803480FC), colrad 1, for at most 3 s.
pub(crate) const BAG_SPEED: f32 = 20.0;
pub(crate) const BAG_WEIGHT: f32 = 50.0;
pub(crate) const BAG_TIME: f32 = 3.0;

/// PORT ADDITION, not in the original (which only shows FX_BLOCK as a
/// clank on a blocked hit taking off more than 2): keep FX_BLOCK on a hero
/// for as long as they hold a block (the DEFEND1 / DEFEND2 / directional
/// defend poses: hud DEFEND1 or BLOCK). Tunable.
pub(crate) const BLOCK_SHIELD_HELD: bool = true;
/// The held shield's MBTreeSetAlpha (0 opaque .. 255 clear); StartBlockFX
/// uses 0x40. Tunable.
pub(crate) const BLOCK_SHIELD_ALPHA: i32 = 0x40;
/// The held shield is kept at least this long ahead (s) each tick, so it
/// lasts while the block does and goes within a few ticks if left behind.
const BLOCK_SHIELD_KEEP: f32 = 0.1;

/// A resolved EffectInfo row (EffectHeader): its tree's model (None when it
/// has no geometry), z bias, alpha and animation length (nframes x rate /
/// 900; 30 frames when it has none).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FxDef {
    pub model: Option<usize>,
    pub zmod: i32,
    pub alpha: i32,
    pub time: f32,
}

/// How a started effect is drawn beyond its entity: MBTreeSetColor's tint
/// and ScaleFX's (possibly uneven) scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxLook {
    pub tint: Option<[f32; 3]>,
    pub scale: Option<Vec3>,
}

/// What an effect hangs from (SfxSetParent / MBNodeSetParent): it keeps its
/// place in that frame and goes with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FxParent {
    Player(usize),
    Entity(u64),
}

/// One live effect this module moves (an Effects[] slot): its motion, morph
/// chain and the damage it carries.
#[derive(Clone, Debug)]
pub(crate) struct LiveFx {
    pub id: u64,
    pub ty: usize,
    /// endtime - now.
    pub left: f32,
    /// minendtime - now: DeleteEffect(mode 0) waits for it.
    hold: f32,
    vel: Vec3,
    /// pyrvel: pitch rate (rad/s).
    pitch_rate: f32,
    weight: f32,
    colrad: f32,
    /// SfxSetMorph: fxmorph, fxmorph2 and morphtime (0: the new row's length).
    morph: Vec<(usize, f32)>,
    /// Damage to heroes it passes (flags 1; SfxSetDamage).
    damage: f32,
    dtype: u32,
    parent: Option<(FxParent, Vec3)>,
    /// fxfade: it clears over its last this many seconds.
    fade: f32,
}

/// The effect system's state (EffectInfo, the live pool's bookkeeping).
#[derive(Clone)]
pub(crate) struct Effects {
    pub defs: Vec<Option<FxDef>>,
    /// EnemyFxPage.hitfx / deathfx: GENHIT / GENDIE custom rows by monster type.
    pub hitfx: Vec<i32>,
    pub deathfx: Vec<i32>,
    /// lbl_80251148: a type whose generated monsters get StartGenFX.
    pub genfx: Vec<bool>,
    customs: usize,
    /// lbl_80344BD0: alternates the BLOODFX rows.
    blood: u32,
    pub live: Vec<LiveFx>,
    pub looks: HashMap<u64, FxLook>,
    /// Player.timer_1FE: block sparks wait this long (s).
    block_timer: [f32; 4],
    /// The held block shield on each hero (BLOCK_SHIELD_HELD; port addition).
    pub block_shield: [Option<u64>; 4],
    /// lbl_803447B8 == 2: StartEnterFX has run.
    entered: bool,
    /// Enemy.specialfx / Player.death_effect: Death's drain arc.
    pub death_fx: HashMap<u64, u64>,
    pub player_death_fx: [Option<u64>; 4],
    /// Players Death drained this frame (field_128 & 1).
    pub drained: [bool; 4],
    /// A started missile's fxhit (SfxSetHit) and damage type, by its id.
    pub missiles: HashMap<u64, (i32, u32)>,
    /// The damage type of the hit hit_entity is about to deal (else the
    /// player's weapon element or the potion's).
    pub hit_dtype: Option<u32>,
    /// A melee hit's upward direction (PlayerMotion_DamageTarget: 0.05 x
    /// the damage, at most 2), for the monster's push.
    pub hit_lift: Option<f32>,
}

impl Default for Effects {
    fn default() -> Self {
        Effects {
            defs: vec![None; MAX_FX],
            hitfx: vec![-1; 45],
            deathfx: vec![-1; 45],
            genfx: vec![false; 45],
            customs: 0,
            blood: 0,
            live: Vec::new(),
            looks: HashMap::new(),
            block_timer: [0.0; 4],
            block_shield: [None; 4],
            entered: false,
            death_fx: HashMap::new(),
            player_death_fx: [None; 4],
            drained: [false; 4],
            missiles: HashMap::new(),
            hit_dtype: None,
            hit_lift: None,
        }
    }
}

/// AtreeMatch in one asset: the tree's model and length, or None when the
/// asset has no tree of that name.
fn match_tree(
    g: &mut Game,
    asset: Option<&AssetFile>,
    known: &mut HashMap<(usize, String), Option<(Option<usize>, f32)>>,
    key: usize,
    name: &str,
) -> Option<(Option<usize>, f32)> {
    if name.is_empty() {
        return None;
    }
    let k = (key, name.to_ascii_uppercase());
    if let Some(r) = known.get(&k) {
        return *r;
    }
    let r = (|| {
        let asset = asset?;
        let anim = asset.anim.as_ref()?;
        let tree = anim
            .atrees
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))?;
        let time = tree_time(tree);
        let model = Model::from_anim(&asset.objects, &asset.textures, anim, Some(&tree.name))
            .ok()
            .filter(|m| !m.mesh.indices.is_empty() || !m.psys.is_empty())
            .map(|m| g.add_model(m));
        Some((model, time))
    })();
    known.insert(k, r);
    r
}

/// StartFXTree's endtime without a time: nframes x rate x 0.00111111 (30
/// frames when the sequence has none).
fn tree_time(tree: &gdl_formats::anim::ATree) -> f32 {
    tree.sequences.first().map_or(1.0, |s| {
        let n = if s.frame_count == 0 {
            30.0
        } else {
            s.frame_count as f32
        };
        let rate = if s.frame_rate > 0 {
            s.frame_rate as f32
        } else {
            30.0
        };
        n * rate * 0.001_111_11
    })
}

/// An RGB colour (0xRRGGBB) as a tint.
fn tint_of(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
    ]
}

impl Game {
    /// InitEffects: resolve the 80 rows against WEAPONS then POWERUPS, the
    /// monster rows from the loaded monster directories (first found wins),
    /// each type's GENHIT / GENDIE, the realm 5-6 wizard's, the boss rows
    /// (ITEMS/<level>), SAFEREXP in B6 and I5, and WORLD_EXP.
    pub(crate) fn init_effects(
        &mut self,
        fs: &GameFs,
        cache: &mut HashMap<String, Option<AssetFile>>,
    ) {
        let letter = self.world_letter();
        let boss = self.boss.is_some();
        let mut dirs: Vec<String> = vec![
            "WEAPONS".into(),
            "POWERUPS".into(),
            // sGoodWizObj (only off boss levels) and sItemFile1Buf.
            if boss {
                String::new()
            } else {
                format!("ITEMS/level{letter}")
            },
            format!("ITEMS/{}", self.level.name),
        ];
        let mut wads: Vec<(usize, String)> = self
            .classes
            .iter()
            .filter(|c| c.etype < 45)
            .map(|c| (c.etype, format!("MONSTERS/{}", c.name)))
            .collect();
        wads.sort();
        wads.dedup_by_key(|w| w.0);
        dirs.extend(wads.iter().map(|w| w.1.clone()));
        for d in &dirs {
            if !d.is_empty() && !cache.contains_key(d) {
                let a = load_asset(fs, d);
                cache.insert(d.clone(), a);
            }
        }
        let asset = |i: usize| cache.get(&dirs[i]).and_then(|a| a.as_ref());
        let mut known = HashMap::new();
        let mut fx = Effects::default();
        let def = |t: Option<(Option<usize>, f32)>, zmod: i32, alpha: i32| {
            t.map(|(model, time)| FxDef {
                model,
                zmod,
                alpha,
                time,
            })
        };
        for (i, &(name, zmod, alpha)) in ROWS.iter().enumerate() {
            let t = match_tree(self, asset(0), &mut known, 0, name)
                .or_else(|| match_tree(self, asset(1), &mut known, 1, name));
            fx.defs[i] = def(t, zmod, alpha);
        }
        // InitCustomEffectSub: the given tree, else WEAPONS, POWERUPS, the
        // wizard's and the level's items; a new FX_CUSTOM row each time.
        let custom = |g: &mut Game,
                      fx: &mut Effects,
                      known: &mut HashMap<_, _>,
                      first: Option<usize>,
                      name: &str|
         -> i32 {
            let t = first
                .into_iter()
                .chain(0..4)
                .find_map(|k| match_tree(g, asset(k), known, k, name));
            match t {
                Some(t) if fx.customs < MAX_CUSTOM => {
                    let i = FX_CUSTOM1 + fx.customs;
                    fx.customs += 1;
                    fx.defs[i] = def(Some(t), 0, 0);
                    i as i32
                }
                _ => -1,
            }
        };
        for (k, (ety, dir)) in wads.iter().enumerate() {
            let w = 4 + k;
            let a = asset(w);
            if a.is_none() {
                continue;
            }
            let mut tree = |g: &mut Game, name: &str| match_tree(g, a, &mut known, w, name);
            if !dir.ends_with("AUX") && fx.defs[FX_SUICIDEEXP].is_none() {
                fx.defs[FX_SUICIDEEXP] = def(tree(self, "SUICIDEEXP"), -512, 0);
            }
            for (n, name) in ["GENFX1", "GENFX2", "GENFX3"].iter().enumerate() {
                if fx.defs[FX_GENFX1 + n].is_none() {
                    fx.defs[FX_GENFX1 + n] = def(tree(self, name), -512, 0);
                    fx.genfx[*ety] = true;
                }
            }
            if *ety == 11 || *ety == 21 {
                fx.defs[FX_ALTHIT] = def(tree(self, "TREEHIT"), -512, 0);
                fx.defs[FX_ALTDIE] = def(tree(self, "TREEDIE"), -512, 0);
            }
            if *ety == 27 {
                fx.defs[FX_ENEATK1] = def(tree(self, "ATK1FX"), -512, 0);
                fx.defs[FX_ENEATK1 + 1] = def(tree(self, "ATK2FX"), -512, 0);
                fx.defs[FX_ENEDEATH1] = def(tree(self, "DEATHFX1"), -512, 0);
                fx.defs[FX_ENEDEATH2] = def(tree(self, "DEATHFX2"), -512, 0);
            }
            if *ety == crate::enemies::DEATH {
                fx.defs[FX_DEATH_HEALTH] = def(tree(self, "DEATH_ARC"), -512, 0);
                fx.defs[FX_DEATH_EXP] = def(tree(self, "DEATH_EXP"), -512, 0);
            }
            fx.hitfx[*ety] = custom(self, &mut fx, &mut known, Some(w), "GENHIT");
            fx.deathfx[*ety] = custom(self, &mut fx, &mut known, Some(w), "GENDIE");
        }
        let realm = self.wave.map_or(0, |w| w >> 8);
        if (5..=6).contains(&realm) && !boss {
            fx.hitfx[0] = custom(self, &mut fx, &mut known, Some(2), "GENHIT");
            fx.deathfx[0] = custom(self, &mut fx, &mut known, Some(2), "GENDIE");
        }
        if boss {
            for (ty, name) in [
                (FX_LEGEND1, "LEGENDFX"),
                (FX_LEGEND2, "LEGENDFX2"),
                (FX_LEGENDPRJ, "LEGENDPRJ"),
                (FX_LEGENDHLD, "LEGENDHLD"),
            ] {
                fx.defs[ty] = def(match_tree(self, asset(3), &mut known, 3, name), 0, 0);
            }
        }
        for n in 1..3 {
            if fx.defs[FX_GENFX1 + n].is_none() {
                fx.defs[FX_GENFX1 + n] = fx.defs[FX_GENFX1];
            }
        }
        let lvl = self.level.name.to_ascii_uppercase();
        if lvl.ends_with("B6") || lvl.ends_with("I5") {
            fx.defs[FX_GENDEST] = def(match_tree(self, asset(3), &mut known, 3, "SAFEREXP"), 0, 0);
        }
        let t = match_tree(self, asset(2), &mut known, 2, "WORLD_EXP")
            .or_else(|| match_tree(self, asset(3), &mut known, 3, "WORLD_EXP"));
        fx.defs[FX_WORLD_EXP] = def(t, -512, 0);
        self.effects = fx;
    }

    /// An EffectInfo row, if its tree was found.
    pub(crate) fn fx_def(&self, ty: usize) -> Option<FxDef> {
        self.effects.defs.get(ty).copied().flatten()
    }

    /// A row's animation length (its time when started without one).
    pub(crate) fn fx_time(&self, ty: usize) -> f32 {
        self.fx_def(ty).map_or(1.0, |d| d.time)
    }

    /// StartFXSub: show row `ty` at `pos` for `time` seconds (0: its
    /// animation's length), at its alpha (MBTreeSetAlpha: 255 is clear).
    /// None when the row has no tree.
    pub(crate) fn start_fx(&mut self, ty: usize, pos: Vec3, yaw: f32, time: f32) -> Option<u64> {
        let d = self.fx_def(ty)?;
        let life = if time > 0.0 { time } else { d.time };
        let id = self.spawn(Kind::Effect { life }, d.model, pos, yaw);
        self.set_fx_alpha(id, d.alpha);
        Some(id)
    }

    /// MBTreeSetAlpha on an effect: 0 opaque .. 255 clear.
    fn set_fx_alpha(&mut self, id: u64, alpha: i32) {
        if let Some(e) = self.entities.iter_mut().rev().find(|e| e.id == id) {
            e.fade = (1.0 - alpha.clamp(0, 255) as f32 / 255.0).max(0.0);
        }
    }

    /// ScaleFX: the effect's node scale.
    pub(crate) fn scale_fx(&mut self, id: u64, s: Vec3) {
        if s.x == s.y && s.y == s.z {
            if let Some(e) = self.entities.iter_mut().rev().find(|e| e.id == id) {
                e.scale = s.x;
            }
            if let Some(l) = self.effects.looks.get_mut(&id) {
                l.scale = None;
            }
        } else {
            let l = self.effects.looks.entry(id).or_insert(FxLook {
                tint: None,
                scale: None,
            });
            l.scale = Some(s);
        }
    }

    /// MBTreeSetColor: tint an effect (0xRRGGBB).
    fn tint_fx(&mut self, id: u64, colour: u32) {
        let l = self.effects.looks.entry(id).or_insert(FxLook {
            tint: None,
            scale: None,
        });
        l.tint = Some(tint_of(colour));
    }

    /// Track a started effect for motion, parenting and morphs.
    fn live_fx(&mut self, id: u64, ty: usize, left: f32) -> &mut LiveFx {
        self.effects.live.push(LiveFx {
            id,
            ty,
            left,
            hold: 0.0,
            vel: Vec3::ZERO,
            pitch_rate: 0.0,
            weight: 0.0,
            colrad: 1.0,
            morph: Vec::new(),
            damage: 0.0,
            dtype: 0,
            parent: None,
            fade: 0.0,
        });
        self.effects.live.last_mut().unwrap()
    }

    /// Where a parent is now.
    fn parent_pos(&self, p: FxParent) -> Option<(Vec3, f32)> {
        match p {
            FxParent::Player(pi) => self
                .players
                .get(pi)
                .filter(|p| p.state.active())
                .map(|p| (p.pos, p.yaw)),
            FxParent::Entity(id) => self
                .entities
                .iter()
                .find(|e| e.id == id && e.alive)
                .map(|e| (e.pos, e.yaw)),
        }
    }

    /// SfxSetParent: hang effect `id` from `p` at its origin.
    fn parent_fx(&mut self, id: u64, ty: usize, left: f32, p: FxParent) {
        let at = self.parent_pos(p);
        let l = self.live_fx(id, ty, left);
        l.parent = Some((p, Vec3::ZERO));
        if let Some((pos, yaw)) = at {
            if let Some(e) = self.entities.iter_mut().rev().find(|e| e.id == id) {
                e.pos = pos;
                e.yaw = yaw;
            }
        }
    }

    /// DeleteEffect: the effect is gone (mode 0 waits for its minendtime).
    pub(crate) fn delete_fx(&mut self, id: u64, wait: bool) -> bool {
        if let Some(l) = self.effects.live.iter_mut().find(|l| l.id == id) {
            if wait && l.hold > 0.0 {
                return false;
            }
            l.left = -1.0;
        }
        if let Some(e) = self.entities.iter_mut().find(|e| e.id == id) {
            e.alive = false;
        }
        self.effects.looks.remove(&id);
        true
    }

    /// fn_800945D0: a monster's hit (`die` false) or death burst by the
    /// hit's element, at `pos`, scaled by its half height: treefolk and acid
    /// blobs have their own, golems and knights a bloodless one (at size 1),
    /// the rest blood (alternating BLOODFX pairs).
    pub(crate) fn start_hit_fx(
        &mut self,
        pos: Vec3,
        yaw: f32,
        dtype: u32,
        die: bool,
        kind: usize,
        hht: f32,
    ) -> Option<u64> {
        let t = (dtype & 0xF) as usize;
        let t = t.min(4);
        let (mut tid, rad) = match kind {
            11 | 21 => (if die { KIND_F[t] } else { KIND_E[t] }, 0.5 * hht),
            29 | 5 => (if die { KIND_B[t] } else { KIND_A[t] }, 1.0),
            _ => (if die { KIND_D[t] } else { KIND_C[t] }, 0.5 * hht),
        };
        if tid == 4 || tid == 5 {
            let base = if tid == 4 { 6 } else { 8 };
            tid = (self.effects.blood & 1) as usize + base;
            self.effects.blood = self.effects.blood.wrapping_add(1);
        }
        let id = self.start_fx(tid, pos, yaw, 0.0)?;
        self.scale_fx(id, Vec3::splat(rad));
        Some(id)
    }

    /// damage_enemy's effects on monster entity `t` (not Death or IT): a
    /// suicide grunt killed blows up (enemy_dies); unless DMG_NOHITFX, the
    /// hit or death burst at its attention point (a tall one's hit where it
    /// landed).
    pub(crate) fn enemy_hit_fx(&mut self, t: usize, dtype: u32, killed: bool) {
        let e = &self.entities[t];
        let Kind::Enemy { etype, ai, .. } = &e.kind else {
            return;
        };
        let (etype, ai, pos, yaw) = (*etype, *ai, e.pos, e.yaw);
        if etype == crate::enemies::DEATH || etype == crate::enemies::IT {
            return;
        }
        let ty = etype.min(33);
        if killed && ai == 18 {
            let at = pos + Vec3::Y * ENE_COLL[ty];
            self.suicide_explosion(at, SUICIDE_DAMAGE * self.tuning.damage);
        }
        if dtype & DMG_NOHITFX != 0 {
            return;
        }
        let hht = 0.5 * crate::enemies::TYPES[ty].height;
        let at = match self.hit_from {
            Some(h) if !killed && hht >= 4.0 => h,
            _ => pos + Vec3::Y * ENE_ATTN[ty],
        };
        self.start_hit_fx(at, yaw, dtype, killed, etype, hht);
    }

    /// items.c's generator damage: a tier lost or the generator destroyed
    /// shows StartGenHitFx where it stands.
    pub(crate) fn generator_fx(&mut self, t: usize, death: bool) {
        let e = &self.entities[t];
        if let Kind::Generator { etype, .. } = &e.kind {
            let (etype, pos, yaw) = (*etype, e.pos, e.yaw);
            self.start_gen_hit_fx(pos, yaw, etype, death);
        }
    }

    /// StartGenHitFx: a generator dropping a tier (`death` false) or
    /// destroyed, by its monster type's GENHIT / GENDIE (GENDEST for
    /// destroyed troll and grunt generators without one).
    pub(crate) fn start_gen_hit_fx(
        &mut self,
        pos: Vec3,
        yaw: f32,
        ene: usize,
        death: bool,
    ) -> Option<u64> {
        let ene = ene.min(44);
        let mut ty = if death {
            self.effects.deathfx[ene]
        } else {
            self.effects.hitfx[ene]
        };
        if death && ty < 0 && (ene == 1 || ene == 4) {
            ty = FX_GENDEST as i32;
        }
        let ty = usize::try_from(ty).ok()?;
        self.start_fx(ty, pos, yaw, 0.0)
    }

    /// StartGenFX: the puff a generated monster of tier 1-3 comes out of.
    pub(crate) fn start_gen_fx(&mut self, pos: Vec3, level: usize) -> Option<u64> {
        if !(1..=3).contains(&level) {
            return None;
        }
        self.fx_def(FX_GENFX1 + level - 1)?;
        self.start_fx(FX_GENFX1 + level - 1, pos, 0.0, 0.0)
    }

    /// StartEnemyAtkFX: Garm's ATK1FX (swing) or ATK2FX (power attack) on
    /// the monster.
    pub(crate) fn start_enemy_atk_fx(&mut self, n: usize, parent: u64) -> Option<u64> {
        let ty = FX_ENEATK1 + n.min(1);
        let id = self.start_fx(ty, Vec3::ZERO, 0.0, 0.0)?;
        let t = self.fx_time(ty);
        self.parent_fx(id, ty, t, FxParent::Entity(parent));
        Some(id)
    }

    /// StartEnemyDeathFX (fn_8004F1DC): Garm's DEATHFX1 flies from `pos`
    /// along `dir` at 20, hurting heroes it passes for 50, and becomes
    /// DEATHFX2 after 3 s.
    pub(crate) fn start_enemy_death_fx(&mut self, pos: Vec3, dir: Vec3) -> Option<u64> {
        let d = self.fx_def(FX_ENEDEATH1)?;
        let id = self.spawn(
            Kind::Effect { life: d.time },
            d.model,
            pos,
            dir.x.atan2(dir.z),
        );
        self.set_fx_alpha(id, d.alpha);
        let damage = ENEDEATH_DAMAGE * self.tuning.damage;
        let l = self.live_fx(id, FX_ENEDEATH1, d.time);
        l.vel = dir.normalize_or_zero() * ENEDEATH_SPEED;
        l.colrad = ENEDEATH_COLRAD;
        l.damage = damage;
        l.dtype = ENEDEATH_TYPE;
        l.morph = vec![(FX_ENEDEATH2, ENEDEATH_TIME)];
        Some(id)
    }

    /// fn_8004F1DC: Garm's pup gone, its death shot heads for the nearest
    /// hero in play (Enemy.closest), if any.
    pub(crate) fn garm_death_fx(&mut self, at: Vec3) {
        let target = self
            .players
            .iter()
            .filter(|p| p.state.active())
            .map(|p| p.pos)
            .min_by(|a, b| a.distance(at).total_cmp(&b.distance(at)));
        if let Some(t) = target {
            self.start_enemy_death_fx(at, (t - at).normalize_or_zero());
        }
    }

    /// StartDeathFX: DEATH_ARC (DEATH_EXP for kind 2) on what Death drains,
    /// for 10 s, held at least half a second.
    pub(crate) fn start_death_fx(&mut self, parent: FxParent, kind: i32) -> Option<u64> {
        let ty = if kind == 2 {
            FX_DEATH_EXP
        } else {
            FX_DEATH_HEALTH
        };
        let id = self.start_fx(ty, Vec3::ZERO, 0.0, 10.0)?;
        self.parent_fx(id, ty, 10.0, parent);
        if let Some(l) = self.effects.live.last_mut() {
            l.hold = 0.5;
        }
        Some(id)
    }

    /// StartGemFX: a pickup sparkle — GETRUNE (0x400), GETGARG (0x100), else
    /// row 69 + `sel` (the GETGEM colours).
    pub(crate) fn start_gem_fx(&mut self, pos: Vec3, sel: i32) -> Option<u64> {
        let ty = match sel {
            0x400 => 79,
            0x100 => 78,
            s => usize::try_from(s + 69).ok()?,
        };
        self.start_fx(ty, pos, 0.0, 0.0)
    }

    /// StartEnterFX: STARTFX where the party starts.
    pub(crate) fn start_enter_fx(&mut self, pos: Vec3) -> Option<u64> {
        self.start_fx(FX_ENTER, pos, 0.0, 0.0)
    }

    /// AddExp's level-up: StartLevelUpFX(0, class_id), the hero's colour's
    /// LEVELUP row for its animation's length, riding the hero.
    pub(crate) fn start_level_up_fx(&mut self, pi: usize) -> Option<u64> {
        let ty = FX_LEVELUP + colour_index(&self.players.get(pi)?.color);
        let time = self.fx_time(ty);
        let id = self.start_fx(ty, Vec3::ZERO, 0.0, time)?;
        self.parent_fx(id, ty, time, FxParent::Player(pi));
        Some(id)
    }

    /// damage_player's block clank: StartBlockFX for `time` (0.333-1 s) on
    /// the hero, in their colour at alpha 0x40, then not again for 60 x
    /// `time` ticks. Over a held shield it adds a second, brightening it.
    pub(crate) fn start_block_fx(&mut self, pi: usize, reduced: f32) -> Option<u64> {
        if self.effects.block_timer.get(pi).is_none_or(|&t| t > 0.0) {
            return None;
        }
        let time = (0.01 * reduced).clamp(0.333, 1.0);
        self.effects.block_timer[pi] = time;
        self.block_fx_on(pi, time, 0x40)
    }

    /// StartBlockFX's body: FX_BLOCK riding hero `pi` for `time`, tinted
    /// lbl_8011A178[class_id] (the costume colour), at `alpha`.
    fn block_fx_on(&mut self, pi: usize, time: f32, alpha: i32) -> Option<u64> {
        let colour = colour_index(&self.players.get(pi)?.color);
        let id = self.start_fx(FX_BLOCK, Vec3::ZERO, 0.0, time)?;
        self.tint_fx(id, COLOUR_TINT[colour]);
        self.set_fx_alpha(id, alpha);
        self.parent_fx(id, FX_BLOCK, time, FxParent::Player(pi));
        Some(id)
    }

    /// PORT ADDITION (BLOCK_SHIELD_HELD; not in the original): FX_BLOCK
    /// shows on a hero while they hold a block — started when it begins,
    /// kept alive while held, ended when released.
    pub(crate) fn update_block_shield(&mut self) {
        for pi in 0..self.effects.block_shield.len() {
            let held = BLOCK_SHIELD_HELD
                && self.players.get(pi).is_some_and(|p| {
                    p.state.active()
                        && p.act.hud & (crate::paction::hud::DEFEND1 | crate::paction::hud::BLOCK)
                            != 0
                });
            let cur = self.effects.block_shield[pi].and_then(|id| {
                self.effects
                    .live
                    .iter()
                    .position(|l| l.id == id && l.left > 0.0)
                    .map(|i| (id, i))
            });
            match (held, cur) {
                (true, Some((_, i))) => {
                    let l = &mut self.effects.live[i];
                    l.left = l.left.max(BLOCK_SHIELD_KEEP);
                }
                (true, None) => {
                    let time = self.fx_time(FX_BLOCK).max(BLOCK_SHIELD_KEEP);
                    self.effects.block_shield[pi] = self.block_fx_on(pi, time, BLOCK_SHIELD_ALPHA);
                }
                (false, Some((id, _))) => {
                    self.delete_fx(id, false);
                    self.effects.block_shield[pi] = None;
                }
                (false, None) => self.effects.block_shield[pi] = None,
            }
        }
    }

    /// ProcessEffects' missile hit: a hero's missile that hit a wall (`wall`)
    /// becomes its fxhit, else its element's hitmorph; one that hit
    /// something becomes its fxhit if it has one. Scaled as the missile was.
    pub(crate) fn missile_burst(&mut self, id: u64, pos: Vec3, yaw: f32, scale: f32, wall: bool) {
        let Some((fxhit, dtype)) = self.effects.missiles.remove(&id) else {
            return;
        };
        let morph = if fxhit > 0 {
            fxhit as usize
        } else if wall {
            HITMORPH[((dtype & 0xF) as usize).min(4)]
        } else {
            return;
        };
        if morph >= MAX_FX {
            return;
        }
        if let Some(fx) = self.start_fx(morph, pos, yaw, 0.0) {
            if scale != 1.0 {
                self.scale_fx(fx, Vec3::splat(scale));
            }
        }
    }

    /// WorldExplosion (WorldObjectExplode): an expanding blast (flags 0x2B)
    /// of 50 — WORLD_EXP of poison gas over 6 in realms 9 and 11, else
    /// EXPLOSION of fire and knockdown over 5, 1.5 wide — and its sound.
    pub(crate) fn world_explosion(&mut self, at: Vec3) {
        let realm = self.wave.map_or(0, |w| w >> 8);
        let (mut ty, mut dtype, radius, wide) = match realm {
            9 | 11 => (FX_WORLD_EXP, DmgType::POISON_GAS, 6.0, 1.0),
            _ => (usize::MAX, 0, 5.0, 1.5),
        };
        if ty == usize::MAX || self.fx_def(ty).is_none() {
            ty = FX_EXPLODE;
            dtype = 33;
        }
        let fx = self.start_fx(ty, at, 0.0, 0.0);
        if let Some(id) = fx {
            self.scale_fx(id, Vec3::new(wide, 1.0, wide));
        }
        let maxtime = self.fx_time(ty);
        self.items.blasts.push(owner_less_blast(
            at,
            WORLD_EXP_DAMAGE,
            dtype,
            radius,
            0x2B,
            maxtime,
            Vec::new(),
            fx,
        ));
    }

    /// SuicideExplosion: in realms 7 and 11 a POISONEXP of gas over 7.5 (2.5
    /// wide, 1 down) that becomes POISONEXP2 for 2 s then POISONEXP3; else an
    /// EXPLOSION of fire, knockdown and blast over 6 hitting heroes and
    /// monsters, with EXPRING at 1.2; then SUICIDEEXP fading over 0.5 s;
    /// and its sound.
    pub(crate) fn suicide_explosion(&mut self, pos: Vec3, dmg: f32) {
        // fn_8009DAC8.
        self.sfx.push("S_EXPLODE1|S_SUICIDEEXP".into());
        let realm = self.wave.map_or(0, |w| w >> 8);
        if realm == 7 || realm == 11 {
            let at = pos - Vec3::Y;
            let fx = self.start_fx(FX_EXPPOISON, at, 0.0, 0.0);
            if let Some(id) = fx {
                self.scale_fx(id, Vec3::new(2.5, 1.0, 2.5));
            }
            let maxtime = self.fx_time(FX_EXPPOISON);
            self.items.blasts.push(owner_less_blast(
                at,
                dmg,
                DmgType::POISON_GAS,
                7.5,
                0x2B,
                maxtime,
                vec![(FX_EXPPOISON2, 2.0), (FX_EXPPOISON3, 0.0)],
                fx,
            ));
        } else {
            let fx = self.start_fx(FX_EXPLODE, pos, 0.0, 0.0);
            let maxtime = self.fx_time(FX_EXPLODE);
            self.items.blasts.push(owner_less_blast(
                pos,
                dmg,
                0x421,
                6.0,
                0x29,
                maxtime,
                Vec::new(),
                fx,
            ));
            if let Some(id) = self.start_fx(FX_EXPRING, pos, 0.0, 0.0) {
                self.scale_fx(id, Vec3::splat(1.2));
            }
        }
        if let Some(id) = self.start_fx(FX_SUICIDEEXP, pos, 0.0, 0.0) {
            let t = self.fx_time(FX_SUICIDEEXP);
            let l = self.live_fx(id, FX_SUICIDEEXP, t);
            l.fade = 0.5;
        }
    }

    /// StartEnterFX once the level-start camera has been up 0.1 s
    /// (gauntworld.c:1131), at the party's middle.
    fn check_enter_fx(&mut self) {
        if self.effects.entered || !self.camera.intro || self.time < 0.1 {
            return;
        }
        self.effects.entered = true;
        let active: Vec<Vec3> = self
            .players
            .iter()
            .filter(|p| p.state.active())
            .map(|p| p.pos)
            .collect();
        if active.is_empty() {
            return;
        }
        let (lo, hi) = active
            .iter()
            .fold((Vec3::splat(1e20), Vec3::splat(-1e20)), |(lo, hi), &p| {
                (lo.min(p), hi.max(p))
            });
        self.start_enter_fx((lo + hi) * 0.5);
    }

    /// The drain arcs: Death keeps its specialfx until it dies (enemy.c:5317
    /// deletes it as the exit starts); a drained hero has theirs only while
    /// drained (DeleteEffect mode 0 after).
    fn update_death_fx(&mut self) {
        let leaving = |e: &Entity| {
            matches!(
                e.kind,
                Kind::Enemy {
                    state: crate::game::EnemyState::Dying { .. },
                    ..
                }
            )
        };
        let gone: Vec<u64> = self
            .effects
            .death_fx
            .keys()
            .copied()
            .filter(|&id| {
                !self
                    .entities
                    .iter()
                    .any(|e| e.id == id && e.alive && !leaving(e))
            })
            .collect();
        for id in gone {
            if let Some(fx) = self.effects.death_fx.remove(&id) {
                self.delete_fx(fx, false);
            }
        }
        for pi in 0..self.players.len().min(4) {
            let drained = std::mem::take(&mut self.effects.drained[pi]);
            let cur = self.effects.player_death_fx[pi]
                .filter(|&id| self.entities.iter().any(|e| e.id == id && e.alive));
            self.effects.player_death_fx[pi] = cur;
            if drained {
                if cur.is_none() {
                    self.effects.player_death_fx[pi] = self.start_death_fx(FxParent::Player(pi), 1);
                }
            } else if let Some(id) = cur {
                if self.delete_fx(id, true) {
                    self.effects.player_death_fx[pi] = None;
                }
            }
        }
    }

    /// Death drains `pi` this frame (Death at `death`): its arc and theirs.
    pub(crate) fn note_drain(&mut self, death: u64, pi: usize) {
        if let Some(d) = self.effects.drained.get_mut(pi) {
            *d = true;
        }
        if !self.effects.death_fx.contains_key(&death) {
            if let Some(id) = self.start_death_fx(FxParent::Entity(death), 1) {
                self.effects.death_fx.insert(death, id);
            }
        }
    }

    /// ProcessEffects for the effects started here: parents carry theirs,
    /// the rest fly and fall (a hero in a damaging one's colrad is hit, once
    /// a quarter second), fade (fxfade), and at their end morph to the next
    /// row of their chain or go.
    pub(crate) fn update_effects(&mut self, dt: f32) {
        self.check_enter_fx();
        for t in &mut self.effects.block_timer {
            *t -= dt;
        }
        self.update_block_shield();
        self.update_death_fx();
        let mut live = std::mem::take(&mut self.effects.live);
        let mut hits: Vec<(usize, f32, u32, Vec3)> = Vec::new();
        for l in &mut live {
            l.left -= dt;
            l.hold -= dt;
            let Some(ei) = self.entities.iter().position(|e| e.id == l.id && e.alive) else {
                l.left = -1.0;
                continue;
            };
            if let Some((p, local)) = l.parent {
                match self.parent_pos(p) {
                    Some((pos, yaw)) => {
                        let e = &mut self.entities[ei];
                        e.pos = pos + glam::Quat::from_rotation_y(yaw) * local;
                        e.yaw = yaw;
                    }
                    None => {
                        // SfxDeleteParented: it goes with its parent.
                        l.left = -1.0;
                    }
                }
            } else if l.vel != Vec3::ZERO || l.weight != 0.0 {
                let from = self.entities[ei].pos;
                let to = from + l.vel * dt;
                l.vel.y -= l.weight * dt;
                let e = &mut self.entities[ei];
                e.pos = to;
                e.pitch += l.pitch_rate * dt;
                if l.damage > 0.0 {
                    for (pi, p) in self.players.iter().enumerate() {
                        if !p.state.active() || self.time < p.fxhittime {
                            continue;
                        }
                        let at = p.pos + Vec3::Y * (PLAYER_HEIGHT * 0.5);
                        if seg_point_dist(from, to, at) < l.colrad + PLAYER_RADIUS {
                            hits.push((pi, l.damage, l.dtype, from));
                        }
                    }
                }
            }
            // fxfade: MBTreeSetAlpha(255 x (1 - remaining / fxfade)).
            if l.fade > 0.0 && l.left < l.fade {
                self.entities[ei].fade = (l.left / l.fade).clamp(0.0, 1.0);
            }
            let e = &mut self.entities[ei];
            if let Kind::Effect { life } = &mut e.kind {
                *life = l.left.max(0.0) + 1.0;
            }
            if l.left <= 0.033_32 && l.left > -1.0 {
                if l.morph.is_empty() {
                    l.left = -1.0;
                } else {
                    // ChangeEffect: the next row, its time or the morphtime.
                    let (ty, t) = l.morph.remove(0);
                    let d = self.effects.defs.get(ty).copied().flatten();
                    match d {
                        Some(d) => {
                            l.ty = ty;
                            l.left = if t > 0.0 { t } else { d.time };
                            let e = &mut self.entities[ei];
                            e.model = d.model;
                            e.animator = crate::character::Animator::new(0);
                            e.fade = (1.0 - d.alpha.clamp(0, 255) as f32 / 255.0).max(0.0);
                        }
                        None => l.left = -1.0,
                    }
                }
            }
            if l.left <= -1.0 {
                self.entities[ei].alive = false;
            }
        }
        live.retain(|l| l.left > -1.0);
        live.append(&mut self.effects.live);
        self.effects.live = live;
        for (pi, dmg, dtype, from) in hits {
            self.players[pi].fxhittime = self.time + 0.25;
            self.damage_from = Some(from);
            self.damage_player(pi, dmg, dtype);
            self.damage_from = None;
        }
        let alive: HashSet<u64> = self
            .entities
            .iter()
            .filter(|e| e.alive)
            .map(|e| e.id)
            .collect();
        self.effects.looks.retain(|id, _| alive.contains(id));
        self.effects.missiles.retain(|id, _| alive.contains(id));
    }

    /// What the renderer needs of an effect beyond its entity.
    pub fn fx_look(&self, id: u64) -> Option<FxLook> {
        self.effects.looks.get(&id).copied()
    }
}

/// An owner-less expanding effect (flags 0x20) over `maxtime`, with its
/// morph chain.
#[allow(clippy::too_many_arguments)]
fn owner_less_blast(
    pos: Vec3,
    damage: f32,
    dtype: u32,
    radius: f32,
    flags: u32,
    maxtime: f32,
    morph: Vec<(usize, f32)>,
    fx: Option<u64>,
) -> items::Blast {
    let mut b = items::Blast::new(pos, damage, dtype, radius, flags, None, maxtime, 0.0, fx);
    b.morph = morph;
    b
}

/// The distance from `p` to the segment `a`-`b`.
fn seg_point_dist(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-8)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    /// A level with the hero settled and nothing else placed.
    fn empty(fs: &GameFs, level: &str) -> Game {
        let mut g = Game::new(fs, level, &[("WAR", "RED")]).unwrap();
        g.entities.clear();
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        g.entities
            .retain(|e| !matches!(e.kind, Kind::Effect { .. }));
        g
    }

    fn effects(g: &Game) -> Vec<&Entity> {
        g.entities
            .iter()
            .filter(|e| e.alive && matches!(e.kind, Kind::Effect { .. }))
            .collect()
    }

    #[test]
    fn effect_rows_resolve_in_every_realm() {
        let Some(fs) = data() else { return };
        // The WEAPONS rows with geometry (BLOODFX, DESTSMOKE, the breaths and
        // the gem pickups are particle trees).
        let shown = [
            1, 2, 3, 4, 5, 10, 11, 12, 15, 18, 21, 22, 23, 24, 25, 26, 27, 28, 29, 31, 32, 34, 35,
            39, 43, 47, 51, 57, 61, 66, 67, 68, 69,
        ];
        for lv in [
            "levelA1", "levelB1", "levelC1", "levelD1", "levelE1", "levelF1", "levelG1", "levelH1",
            "levelI1", "levelJ1", "levelK1", "levelL1", "levelS1", "levelT1",
        ] {
            let g = Game::new(&fs, lv, &[("WAR", "RED")]).unwrap();
            for &r in &shown {
                let d = g.fx_def(r);
                assert!(
                    d.is_some_and(|d| d.model.is_some()),
                    "{lv}: row {r} {:?}",
                    ROWS[r]
                );
            }
            assert_eq!(g.fx_def(1).unwrap().alpha, 96);
            assert_eq!(g.fx_def(66).unwrap().zmod, -512);
            for t in [6, 7, 8, 9, 33] {
                assert!(g.fx_def(t).is_some(), "{lv}: row {t} (particles)");
            }
            // Death walks every level without a boss: its drain arcs.
            if g.classes.iter().any(|c| c.etype == crate::enemies::DEATH) {
                assert!(g.fx_def(FX_DEATH_HEALTH).is_some(), "{lv}: DEATH_ARC");
                assert!(g.fx_def(FX_DEATH_EXP).is_some(), "{lv}: DEATH_EXP");
            }
            // Each monster type's GENHIT / GENDIE took a custom row.
            for c in &g.classes {
                let (h, d) = (g.effects.hitfx[c.etype], g.effects.deathfx[c.etype]);
                if h >= 0 {
                    assert!(h as usize >= FX_CUSTOM1 && g.fx_def(h as usize).is_some());
                }
                if d >= 0 {
                    assert!(d as usize >= FX_CUSTOM1 && g.fx_def(d as usize).is_some());
                }
            }
        }
        // The treefolk realm has TREEHIT / TREEDIE; realm 9's WORLD_EXP.
        let k1 = Game::new(&fs, "levelK1", &[("WAR", "RED")]).unwrap();
        assert!(k1.fx_def(FX_ALTHIT).is_some() && k1.fx_def(FX_ALTDIE).is_some());
        let i1 = Game::new(&fs, "levelI1", &[("WAR", "RED")]).unwrap();
        assert!(i1.fx_def(FX_WORLD_EXP).is_some());
    }

    #[test]
    fn a_monster_hit_and_death_show_their_bursts() {
        let Some(fs) = data() else { return };
        let mut g = empty(&fs, "levelA1");
        let class = g.classes.iter().position(|c| c.name == "GRU").unwrap();
        let model = g.classes[class].model(1);
        let mut kind = g.new_enemy(class, 1, EnemyState::Idle);
        if let Kind::Enemy { cooldown, hp, .. } = &mut kind {
            *cooldown = 99.0;
            *hp = 100.0;
        }
        let at = g.players[0].pos + Vec3::new(0.0, 0.0, 6.0);
        let id = g.spawn(kind, model, at, 0.0);
        let t = g.entities.iter().position(|e| e.id == id).unwrap();
        // A fire hit: FIREHIT (kindidC[1]) at its attention point (3.8 up),
        // at half its half height.
        g.effects.hit_dtype = Some(1);
        g.hit_entity(0, t, 10.0, Vec2::Y, false);
        let fx = effects(&g);
        assert_eq!(fx.len(), 1);
        assert_eq!(fx[0].model, g.fx_def(11).unwrap().model);
        assert!((fx[0].pos - (at + Vec3::Y * 3.8)).length() < 1e-3);
        assert!((fx[0].scale - 1.5).abs() < 1e-5);
        // 0x60 of alpha.
        assert!((fx[0].fade - (1.0 - 96.0 / 255.0)).abs() < 1e-3);
        // The killing blow: FIREDIE.
        g.effects.hit_dtype = Some(1);
        g.hit_entity(0, t, 1000.0, Vec2::Y, false);
        let fx = effects(&g);
        assert_eq!(fx.len(), 2);
        assert_eq!(fx[1].model, g.fx_def(12).unwrap().model);
        // A plain blow on a grunt is blood: kindidC's 4 alternates between
        // the two BLOODFX1 rows (a particle tree).
        let id2 = g.spawn(g.new_enemy(class, 1, EnemyState::Idle), model, at, 0.0);
        let t2 = g.entities.iter().position(|e| e.id == id2).unwrap();
        let n = effects(&g).len();
        g.hit_entity(0, t2, 1.0, Vec2::Y, false);
        assert_eq!(effects(&g).len(), n + 1);
        assert_eq!(effects(&g)[n].model, g.fx_def(6).unwrap().model);
        assert_eq!(g.effects.blood, 1);
    }

    #[test]
    fn a_missile_on_the_floor_becomes_its_hit_row() {
        let Some(fs) = data() else { return };
        let mut g = empty(&fs, "levelA1");
        let from = g.players[0].pos + Vec3::new(0.0, 3.0, 4.0);
        let shoot = |g: &mut Game, dtype: u32| {
            let id = g.spawn(
                Kind::Projectile {
                    owner: 0,
                    vel: Vec3::new(0.0, -20.0, 5.0),
                    life: 2.0,
                    damage: 1.0,
                    weight: 0.0,
                    radius: 1.0,
                    tumble: 0.0,
                    reflect: false,
                    pierced: None,
                    wall_sound: WallSound::None,
                    hold: 0.0,
                    morph: None,
                },
                None,
                from,
                0.0,
            );
            g.effects.missiles.insert(id, (0, dtype));
            for _ in 0..60 {
                g.update(1.0 / 60.0, &[PlayerInput::default()]);
                if !g.entities.iter().any(|e| e.id == id && e.alive) {
                    break;
                }
            }
            effects(g).last().map(|e| (e.model, e.pos))
        };
        // Bursting on the world: its element's hitmorph — SPARKS, FIREHIT.
        let (m, at) = shoot(&mut g, 0).expect("a burst");
        assert_eq!(m, g.fx_def(1).unwrap().model);
        assert!(at.y < from.y - 1.0, "where it hit");
        let (m, _) = shoot(&mut g, 1).expect("a burst");
        assert_eq!(m, g.fx_def(10).unwrap().model);
    }

    #[test]
    fn a_block_clanks_in_the_heros_colour() {
        let Some(fs) = data() else { return };
        let mut g = empty(&fs, "levelA1");
        g.players[0].act.hud |= crate::paction::hud::BLOCK;
        g.damage_player(0, 20.0, 0);
        let fx = effects(&g);
        assert_eq!(fx.len(), 1);
        assert_eq!(fx[0].model, g.fx_def(FX_BLOCK).unwrap().model);
        let id = fx[0].id;
        let look = g.fx_look(id).unwrap();
        assert_eq!(look.tint, Some([1.0, 0.0, 0.0]), "red");
        // timer_1FE: not again for 0.333 s; it rides the hero.
        g.damage_player(0, 20.0, 0);
        assert_eq!(effects(&g).len(), 1);
        g.players[0].pos += Vec3::X;
        g.update(1.0 / 60.0, &[PlayerInput::default()]);
        let e = g.entities.iter().find(|e| e.id == id).unwrap();
        assert!((e.pos - g.players[0].pos).length() < 0.2);
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        assert!(!g.entities.iter().any(|e| e.id == id && e.alive), "gone");
    }

    #[test]
    fn a_held_block_shows_the_shield_until_released() {
        let Some(fs) = data() else { return };
        let mut g = empty(&fs, "levelA1");
        let blocking = |g: &Game| -> Vec<u64> {
            effects(g)
                .iter()
                .filter(|e| Some(e.model) == Some(g.fx_def(FX_BLOCK).unwrap().model))
                .map(|e| e.id)
                .collect()
        };
        // Start: the block begins (DEFEND1, then BLOCK).
        g.players[0].act.hud |= crate::paction::hud::DEFEND1;
        g.update_block_shield();
        let shown = blocking(&g);
        assert_eq!(shown.len(), 1);
        let id = shown[0];
        assert_eq!(g.effects.block_shield[0], Some(id));
        let look = g.fx_look(id).unwrap();
        assert_eq!(look.tint, Some([1.0, 0.0, 0.0]), "RED's lbl_8011A178 tint");
        let e = g.entities.iter().find(|e| e.id == id).unwrap();
        let want = 1.0 - BLOCK_SHIELD_ALPHA as f32 / 255.0;
        assert!((e.fade - want).abs() < 1e-4, "alpha 0x40");
        // Keep: held well past the row's length, the same effect stays,
        // riding the hero.
        g.players[0].act.hud = crate::paction::hud::BLOCK;
        for _ in 0..240 {
            g.players[0].pos += Vec3::X * 0.01;
            g.update_effects(1.0 / 60.0);
        }
        assert_eq!(blocking(&g), vec![id]);
        let e = g.entities.iter().find(|e| e.id == id).unwrap();
        assert!((e.pos - g.players[0].pos).length() < 0.2);
        // The clank still comes on a blocked hit, as a second one on top.
        g.damage_player(0, 20.0, 0);
        assert_eq!(blocking(&g).len(), 2);
        // Stop: released, the shield goes at once.
        g.players[0].act.hud = 0;
        g.update_effects(1.0 / 60.0);
        assert!(!g.entities.iter().any(|e| e.id == id && e.alive), "gone");
        assert_eq!(g.effects.block_shield[0], None);
    }

    #[test]
    fn a_block_held_by_the_button_raises_the_shield() {
        let Some(fs) = data() else { return };
        let mut g = empty(&fs, "levelA1");
        let hold = PlayerInput {
            turbo: true,
            ..Default::default()
        };
        let mut seen = false;
        for f in 0..20 {
            let b_pressed = f == 0;
            g.update(1.0 / 60.0, &[PlayerInput { b_pressed, ..hold }]);
            seen |= g.effects.block_shield[0].is_some();
        }
        assert!(seen, "the shield showed while blocking");
        for _ in 0..240 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        assert_eq!(g.effects.block_shield[0], None, "and went after");
    }

    #[test]
    fn world_explosions_blast_and_show_their_row() {
        let Some(fs) = data() else { return };
        let mut g = empty(&fs, "levelA1");
        let at = g.players[0].pos + Vec3::new(0.0, 1.0, 3.0);
        g.world_explosion(at);
        let fx = effects(&g);
        assert_eq!(fx.len(), 1);
        assert_eq!(fx[0].model, g.fx_def(FX_EXPLODE).unwrap().model);
        assert_eq!(
            g.fx_look(fx[0].id).unwrap().scale,
            Some(Vec3::new(1.5, 1.0, 1.5))
        );
        let b = g.items.blasts.last().unwrap();
        assert_eq!((b.dtype, b.radius, b.flags), (33, 5.0, 0x2B));
        let hp = g.players[0].health;
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        assert!(g.players[0].health < hp, "it reached the hero");
    }

    #[test]
    fn garms_death_shot_flies_hurts_and_morphs() {
        let Some(fs) = data() else { return };
        let mut g = empty(&fs, "levelA1");
        // Stand-ins for GARM2's DEATHFX1 / DEATHFX2 (loaded on its level).
        g.effects.defs[FX_ENEDEATH1] = g.fx_def(FX_EXPLODE);
        g.effects.defs[FX_ENEDEATH2] = g.fx_def(23);
        let p = g.players[0].pos;
        let from = p + Vec3::new(0.0, 2.5, -8.0);
        g.garm_death_fx(from);
        let id = effects(&g)[0].id;
        let first = g.fx_time(FX_EXPLODE);
        let hp = g.players[0].health;
        let mut morphed = None;
        for f in 0..600 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            let Some(e) = g.entities.iter().find(|e| e.id == id && e.alive) else {
                break;
            };
            if morphed.is_none() && e.model == g.fx_def(23).unwrap().model {
                morphed = Some(f as f32 / 60.0);
            }
        }
        let t = morphed.expect("it became DEATHFX2");
        assert!((t - first).abs() < 0.1, "{t} vs {first}");
        assert!(g.players[0].health < hp, "it went through the hero");
        assert!(
            !g.entities.iter().any(|e| e.id == id && e.alive),
            "and ended"
        );
    }

    #[test]
    fn the_level_entry_effect_plays_once() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let m = g.fx_def(FX_ENTER).unwrap().model;
        let mut seen = 0;
        for _ in 0..30 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
            seen += g
                .entities
                .iter()
                .filter(|e| e.alive && e.model == m && matches!(e.kind, Kind::Effect { .. }))
                .count()
                .min(1);
        }
        assert!(seen > 0);
        let n = g.entities.iter().filter(|e| e.model == m).count();
        assert_eq!(n, 1);
    }
}
