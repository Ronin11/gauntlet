//! The whole simulation state as a digest, for finding a desync.
//!
//! `Game::fingerprint` is cheap and covers the hot state, enough to notice
//! that two peers disagree. This one covers everything that decides what
//! happens next, section by section, so a test or a desync report can name
//! what diverged first. It is slower: use it in tests and occasionally in
//! play, not every tick.
//!
//! Sections hash each value's `Debug` text. Rust prints floats exactly (the
//! shortest text that reads back to the same bits) and a NaN just as "NaN",
//! so this is as strict as hashing the bits without the platforms' NaN
//! differences. Everything inside is ordered: `BTreeMap`s, `Vec`s.
//!
//! The destructuring in `state_sections` names every field of `Game`, so a
//! new field won't compile until someone decides whether it is state.

use super::*;
use std::fmt::Write;

/// FNV-1a as a formatting sink, so nothing is allocated for the text.
struct Fnv(u64);

impl Write for Fnv {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        for &b in s.as_bytes() {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
        Ok(())
    }
}

fn hash(v: &dyn std::fmt::Debug) -> u64 {
    let mut h = Fnv(0xcbf29ce484222325);
    let _ = write!(h, "{v:?}");
    h.0
}

impl Game {
    /// A digest of each part of the state, in a fixed order.
    pub fn state_sections(&self) -> Vec<(&'static str, u64)> {
        let Game {
            // Loaded once from the level's files, which peers have agreed on
            // (see `compat`), and not changed by play.
            level: _,
            obj_flags: _,
            player_seqs: _,
            monster_seqs: _,
            shadow_models: _,
            classes: _,
            worlds: _,
            scrolls: _,
            scroll_scale: _,
            scroll_rom: _,
            music_items: _,
            ambient_items: _,
            tuning: _,
            ai_paths: _,
            magic_models: _,
            worn_models: _,
            action_seqs: _,
            pdata: _,
            psfx: _,
            psys_banks: _,
            // Grown as play needs them: what matters is that the indices agree.
            models,
            critter_wads,
            item_models,
            wizard_model,
            // Written for the engine to sound and show, read by nothing here.
            sfx: _,
            world_changed: _,
            fixed_camera: _,
            // The state.
            collision,
            players,
            entities,
            movers,
            next_id,
            cam_focus,
            cam_yaw,
            cam_pitch,
            camera,
            shake,
            start_index,
            time,
            frame,
            rng,
            wave,
            scroll_box,
            sumner,
            trigger_cam,
            controls,
            barrier_touches,
            pending_enemies,
            damage_from,
            music_area,
            runes,
            deaths,
            roster,
            max_enemies,
            it_holder,
            special_counter,
            magic,
            worn_fx,
            items,
            randpottype,
            shield_cooldown,
            damg_cooldown,
            rules,
            difficulty,
            friendly_fire,
            exit_state,
            fx_follow,
            secret_timer,
            visible_coins,
            shards,
            garg_items,
            welcome,
            crystals,
            crystal_shown,
            hidden,
            world_anim,
            transport,
            level_end,
            msgs,
            boss,
            boss_runes,
            legend_held,
            debug_god,
            snd,
            spews,
            crit_waves,
            crit_hit_until,
            crit_ntargets,
            hit_from,
            magic_hit,
            gen_streak,
            nag,
            effects,
            particles,
            placed,
        } = self;
        let loaded = (
            models.len(),
            critter_wads.keys().collect::<Vec<_>>(),
            item_models,
            wizard_model,
        );
        vec![
            ("clock", hash(&(frame, time, rng.0, next_id))),
            ("players", hash(players)),
            ("entities", hash(entities)),
            ("movers", hash(movers)),
            ("collision", hash(collision)),
            ("world_anim", hash(world_anim)),
            ("transport", hash(transport)),
            (
                "camera",
                hash(&(cam_focus, cam_yaw, cam_pitch, camera, shake)),
            ),
            ("triggers", hash(&(trigger_cam, barrier_touches, hidden))),
            (
                "progress",
                hash(&(
                    runes,
                    shards,
                    garg_items,
                    crystals,
                    crystal_shown,
                    boss_runes,
                    legend_held,
                )),
            ),
            (
                "session",
                hash(&(
                    wave,
                    start_index,
                    rules.digest(),
                    difficulty,
                    friendly_fire,
                    debug_god,
                    controls,
                )),
            ),
            (
                "spawning",
                hash(&(
                    pending_enemies,
                    roster,
                    max_enemies,
                    gen_streak,
                    special_counter,
                )),
            ),
            (
                "magic",
                hash(&(magic, magic_hit, worn_fx, randpottype, fx_follow)),
            ),
            ("items", hash(items)),
            (
                "hits",
                hash(&(
                    shield_cooldown,
                    damg_cooldown,
                    damage_from,
                    hit_from,
                    placed,
                )),
            ),
            (
                "level_flow",
                hash(&(
                    exit_state,
                    level_end,
                    secret_timer,
                    visible_coins,
                    welcome,
                    deaths,
                )),
            ),
            (
                "boss",
                hash(&(boss, spews, crit_waves, crit_hit_until, crit_ntargets)),
            ),
            ("it", hash(&(it_holder, nag))),
            ("story", hash(&(scroll_box, sumner))),
            ("messages", hash(msgs)),
            ("effects", hash(effects)),
            ("particles", hash(particles)),
            ("sound_state", hash(&snd.simulated())),
            ("music", hash(music_area)),
            ("loaded", hash(&loaded)),
        ]
    }

    /// Every section folded into one value.
    pub fn state_digest(&self) -> u64 {
        hash(&self.state_sections())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    const PARTY: [(&str, &str); 4] = [
        ("WAR", "RED"),
        ("VAL", "BLU"),
        ("WIZ", "YEL"),
        ("ARC", "GRE"),
    ];

    fn step(g: &mut Game) -> Vec<PlayerInput> {
        let inputs: Vec<_> = (0..g.players.len()).map(|i| g.bot_input(i)).collect();
        g.update(&inputs);
        inputs
    }

    /// The first section two games disagree on, for the failure message.
    fn first_difference(a: &Game, b: &Game) -> Option<&'static str> {
        a.state_sections()
            .into_iter()
            .zip(b.state_sections())
            .find(|(x, y)| x != y)
            .map(|(x, _)| x.0)
    }

    /// What rollback does: go back to a saved state and play the same inputs
    /// again. Anything a clone fails to carry shows up here.
    #[test]
    fn going_back_and_replaying_lands_in_the_same_state() {
        let Some(fs) = data() else { return };
        for level in ["levelA1", "levelL1"] {
            let mut g = Game::new(&fs, level, &PARTY).unwrap();
            for _ in 0..300 {
                step(&mut g);
            }
            let saved = g.clone();
            assert_eq!(
                saved.state_digest(),
                g.state_digest(),
                "{level}: a clone matches"
            );
            let inputs: Vec<_> = (0..120).map(|_| step(&mut g)).collect();
            let mut again = saved;
            for i in &inputs {
                again.update(i);
            }
            assert_eq!(
                first_difference(&g, &again),
                None,
                "{level}: the replay diverged"
            );
        }
    }

    #[test]
    fn two_games_agree_on_the_whole_state() {
        let Some(fs) = data() else { return };
        let mut a = Game::new(&fs, "levelA1", &PARTY).unwrap();
        let mut b = Game::new(&fs, "levelA1", &PARTY).unwrap();
        assert_eq!(first_difference(&a, &b), None, "fresh games");
        for tick in 0..600 {
            let inputs = step(&mut a);
            b.update(&inputs);
            if tick % 100 == 99 {
                assert_eq!(first_difference(&a, &b), None, "tick {tick}");
            }
        }
    }

    /// Deep state the cheap fingerprint can't see still counts here.
    #[test]
    fn the_digest_sees_what_the_fingerprint_does_not() {
        let Some(fs) = data() else { return };
        let mut a = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        for _ in 0..60 {
            step(&mut a);
        }
        let mut b = a.clone();
        b.damg_cooldown.insert((0, 1), 1.0);
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_eq!(first_difference(&a, &b), Some("hits"));
        assert_ne!(a.state_digest(), b.state_digest());
    }

    /// Local presentation settings must not read as a desync.
    #[test]
    fn what_the_engine_sets_for_itself_is_not_state() {
        let Some(fs) = data() else { return };
        let a = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let mut b = a.clone();
        b.snd.show_compass = !b.snd.show_compass;
        b.sfx.push("S_ANY".into());
        b.world_changed = !b.world_changed;
        assert_eq!(first_difference(&a, &b), None);
    }

    /// Every level on the disc, the whole state: slow, so asked for by name
    /// (`cargo test -p gdl-sim --release -- --ignored every_level`).
    #[test]
    #[ignore]
    fn every_level_plays_the_same_twice() {
        let Some(fs) = data() else { return };
        for level in fs.list_dirs("LEVELS") {
            let Ok(mut a) = Game::new(&fs, &level, &PARTY) else {
                continue;
            };
            let mut b = Game::new(&fs, &level, &PARTY).unwrap();
            for _ in 0..1800 {
                let inputs = step(&mut a);
                b.update(&inputs);
            }
            assert_eq!(first_difference(&a, &b), None, "{level}");
        }
    }
}
