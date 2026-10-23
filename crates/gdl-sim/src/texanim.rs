//! Texture animation timing: the game's `TEXMOD` records (auxanim.c / atree.c
//! in the GameCube decompilation).
//!
//! Two drivers run texmods:
//!
//! - `DoTexMods` (atree.c) runs a model's *global* texmods (`flag == -1`) once
//!   per game frame: the level's from `DoWorldAnimation` (gauntworld.c),
//!   enemies' from `enemy_update`, players', critters' and so on. Each one
//!   (`DoTexModSub`) advances its counter every `rate` frames of `InfFrame`
//!   (clock.c; one per un-paused game frame) and then either points the
//!   texture at frame `source + counter` (`MBSetRomTexture`) or, for the scroll
//!   modes, sets the texture's UV offset to `counter / frames`.
//! - `DoSeqTexMods` / `AnimateNode` run *sequence* texmods and texmod nodes
//!   with an animation's frame (`DoTexModSeqSub`): a texture swap on the node
//!   tree (`MBTreeSetAltTex`), a UV scale/offset (`MBTreeSetUVScaleAdd`), or an
//!   alpha fade (`MBTreeSetAlpha`).
//!
//! The engine runs at the game's 60 Hz, so `InfFrame` is the game's tick count.

use gdl_formats::anim::TexMod;

/// What a global texmod shows at a given `InfFrame`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlobalState {
    /// Frame `n` of the flipbook (the texture shows bitmap `source + n`).
    Frame(usize),
    /// U offset of the texture's scroll record (`y0` in the 16-byte record).
    ScrollU(f32),
    /// V offset of the texture's scroll record (`y1`).
    ScrollV(f32),
    /// Nothing to animate (mip blend, or a mode a global texmod cannot use).
    None,
}

/// Number of `DoTexModSub` steps after `inf_frame` frames: the counter advances
/// when `rate <= 0 || InfFrame % rate == 0`, InfFrame being 1, 2, ... on
/// successive frames.
pub fn steps(rate: i32, inf_frame: u32) -> u32 {
    if rate <= 0 {
        inf_frame
    } else {
        inf_frame / rate as u32
    }
}

/// The texmod's counter after `inf_frame` frames, or None before its first step
/// (when `InitTexMod`'s state still shows).
///
/// `DoTexModSub`: `if (++counter >= abs(frames)) counter = 0;`
pub fn global_counter(tm: &TexMod, inf_frame: u32) -> Option<i32> {
    let mut left = steps(tm.rate, inf_frame);
    if left == 0 {
        return None;
    }
    let m = (tm.frames as i32).abs();
    let mut c = tm.frame;
    // Step one at a time until the counter is in range, then in closed form.
    while left > 0 && !(0..m).contains(&c) {
        c += 1;
        if c >= m {
            c = 0;
        }
        left -= 1;
    }
    if m > 0 {
        c = ((c as i64 + left as i64) % m as i64) as i32;
    }
    Some(c)
}

/// What global texmod `tm` shows after `inf_frame` game frames.
pub fn global_state(tm: &TexMod, inf_frame: u32) -> GlobalState {
    let counter = global_counter(tm, inf_frame);
    match tm.source {
        TexMod::SCROLL_U | TexMod::SCROLL_V => {
            // InitTexMod zeroes the scroll record; DoTexScroll2Sub then stores
            // mul * CalcTexScroll(counter - startframe, 0, |frames|, counter).
            let off = counter.map_or(0.0, |c| {
                let (n, mul) = if tm.frames < 0 {
                    (-(tm.frames as f32), -1.0)
                } else {
                    (tm.frames as f32, 1.0)
                };
                mul * calc_tex_scroll((c - tm.start_frame as i32) as f32, 0.0, n, c).0
            });
            if tm.source == TexMod::SCROLL_U {
                GlobalState::ScrollU(off)
            } else {
                GlobalState::ScrollV(off)
            }
        }
        TexMod::FADE_IN | TexMod::FADE_OUT | TexMod::MIPBLEND => GlobalState::None,
        // A flipbook (source >= 0) or an external source (resolved to an index
        // by InitTexMod, then animated the same way). Before the first step
        // InitTexMod has already shown frame 0.
        _ => GlobalState::Frame(counter.unwrap_or(0).max(0) as usize),
    }
}

/// `CalcTexScroll(t, lo, hi, frame, &scale)`: returns (scroll, scale).
///
/// With `lo == 0` it is a plain wrap, `(frame % (int)hi) / hi` (scale 0); otherwise
/// a ramp in over `lo` frames, a hold until `hi`, and a ramp out.
pub fn calc_tex_scroll(t: f32, lo: f32, hi: f32, frame: i32) -> (f32, f32) {
    let (mut lo, mut hi) = (lo, hi);
    let mut gt = false;
    if lo > hi {
        lo = hi;
        gt = true;
    }
    if lo == 0.0 {
        let ih = hi as i32;
        if ih == 0 {
            // The game would divide by zero; no data does this.
            return (0.0, 0.0);
        }
        return (((frame % ih) as f32) / hi, 0.0);
    }
    let (ret, d5): (f32, f32);
    if t as f64 <= 0.0 {
        ret = 0.0;
        d5 = 0.0;
    } else if t < lo {
        d5 = 1.0;
        lo = (1.0 / lo as f64) as f32;
        hi = (hi * lo) as f32;
        let hi = hi as f64;
        ret = (((t * lo) as f64) * ((1.0 - hi) - 2.0 * -hi) + 2.0 * -hi) as f32;
    } else if gt {
        ret = 0.0;
        d5 = 1.0;
    } else if t < hi {
        let t = t - lo;
        let span = hi - lo;
        let lo = (1.0 / lo as f64) as f32;
        let scaled = (hi * lo) as f64;
        let d4 = (t / span) as f64;
        let dret = 1.0 - scaled;
        ret = (d4 * -dret + dret) as f32;
        d5 = (d4 * (scaled - 1.0) + 1.0) as f32;
    } else if t < hi + lo {
        let lo = (1.0 / lo as f64) as f32;
        ret = lo * (t - hi);
        d5 = hi * lo;
    } else {
        ret = 1.0;
        d5 = hi * (1.0 / lo as f64) as f32;
    }
    (ret, d5 - ret)
}

/// What a sequence texmod / texmod node does at animation frame `frame`
/// (`DoTexModSeqSub`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeqState {
    /// `MBTreeSetAltTex`: draw texture `tex` as flipbook frame `n` (bitmap `source + n`).
    Frame(usize),
    /// `MBTreeSetUVScaleAdd(uScale, uAdd, vScale, vAdd)`.
    Uv([f32; 4]),
    /// `MBTreeSetAlpha`: node opacity 0..=255.
    Alpha(u8),
    None,
}

pub fn seq_state(tm: &TexMod, frame: i32) -> SeqState {
    let start = tm.start_frame as i32;
    match tm.source {
        TexMod::SCROLL_U | TexMod::SCROLL_V => {
            // DoTexScrollSub: CalcTexScroll(frame - start, rate, frames, frame, &scale).
            let (scroll, scale) = calc_tex_scroll(
                (frame - start) as f32,
                tm.rate as f32,
                tm.frames as f32,
                frame,
            );
            SeqState::Uv(if tm.source == TexMod::SCROLL_U {
                [scale, scroll, 1.0, 0.0]
            } else {
                [1.0, 0.0, scale, scroll]
            })
        }
        TexMod::FADE_IN | TexMod::FADE_OUT => {
            // DoTexFadeSub: -4 passes fadeout = 1, -5 passes 0 (as decompiled).
            let f = (frame - start) as f32;
            let n = tm.frames as f32;
            let mut x = if f <= 0.0 || n <= 0.0 {
                0.0
            } else if f >= n {
                1.0
            } else {
                f / n
            };
            if tm.source == TexMod::FADE_IN {
                x = (1.0 - x as f64) as f32;
            }
            SeqState::Alpha((x as f64 * 255.0) as i32 as u8)
        }
        TexMod::MIPBLEND => SeqState::None,
        _ => {
            let d = frame - start;
            let f = if d < 0 {
                0
            } else {
                let f = if tm.rate > 0 { d / tm.rate } else { d };
                f.min(tm.frames as i32 - 1)
            };
            SeqState::Frame(f.max(0) as usize)
        }
    }
}

/// `DoSeqTexMods`: the states of a sequence's texmods at animation frame
/// `frame`. Like the game, a repeating sequence wraps the frame by each
/// texmod's period (`frames * rate`) in turn, and the wrapped frame carries on
/// to the following texmods.
pub fn seq_states(texmods: &[TexMod], repeat: bool, mut frame: i32) -> Vec<SeqState> {
    texmods
        .iter()
        .map(|tm| {
            let period = tm.frames as i32 * tm.rate;
            if frame > period && repeat && period > 1 {
                frame %= period;
            }
            seq_state(tm, frame)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tm(source: i32, frames: i16, rate: i32, frame: i32) -> TexMod {
        TexMod {
            atree: -1,
            seq: -1,
            name: "T".into(),
            source_name: "T00".into(),
            tex: 6,
            source,
            frames,
            start_frame: 0,
            rate,
            frame,
        }
    }

    /// Step-by-step model of DoTexModSub, to check the closed form against.
    fn simulate(t: &TexMod, n: u32) -> Option<i32> {
        let mut c = t.frame;
        let mut stepped = false;
        for inf in 1..=n {
            if t.rate <= 0 || inf % t.rate as u32 == 0 {
                c += 1;
                if c >= (t.frames as i32).abs() {
                    c = 0;
                }
                stepped = true;
            }
        }
        stepped.then_some(c)
    }

    #[test]
    fn counter_matches_step_by_step() {
        for t in [
            tm(7, 30, 4, 0),
            tm(-1, 15, 2, 5),
            tm(-1, 15, 2, 10),
            tm(-2, 1250, 0, 0),
            tm(-3, -60, 0, 0),
            tm(3, 4, 1, 7),
            tm(3, 0, 1, 0),
        ] {
            for n in 0..400 {
                assert_eq!(global_counter(&t, n), simulate(&t, n), "{t:?} n={n}");
            }
        }
    }

    #[test]
    fn lava_flipbook_timing() {
        // levelB6's LAVA <- LAVA00: 30 frames, a new one every 4 game frames.
        let t = tm(7, 30, 4, 0);
        assert_eq!(global_state(&t, 0), GlobalState::Frame(0));
        assert_eq!(global_state(&t, 3), GlobalState::Frame(0));
        assert_eq!(global_state(&t, 4), GlobalState::Frame(1));
        assert_eq!(global_state(&t, 119), GlobalState::Frame(29));
        // Wraps after 30 steps = 120 frames (2 s at 60 Hz).
        assert_eq!(global_state(&t, 120), GlobalState::Frame(0));
        assert_eq!(global_state(&t, 124), GlobalState::Frame(1));
    }

    #[test]
    fn torch_phases() {
        // TORCHA/B/C share TORCH00's 15 frames, staggered by their initial counters.
        let a = tm(-1, 15, 2, 0);
        let b = tm(-1, 15, 2, 5);
        assert_eq!(global_state(&b, 1), GlobalState::Frame(0));
        assert_eq!(global_state(&a, 2), GlobalState::Frame(1));
        assert_eq!(global_state(&b, 2), GlobalState::Frame(6));
        assert_eq!(global_state(&b, 20), GlobalState::Frame(0));
    }

    #[test]
    fn scroll_timing() {
        // levelB6's SKY: scroll U by 1/1250 every frame.
        let t = tm(-2, 1250, 0, 0);
        assert_eq!(global_state(&t, 0), GlobalState::ScrollU(0.0));
        assert_eq!(global_state(&t, 1), GlobalState::ScrollU(1.0 / 1250.0));
        assert_eq!(global_state(&t, 625), GlobalState::ScrollU(0.5));
        assert_eq!(global_state(&t, 1250), GlobalState::ScrollU(0.0));
        // Negative frames scroll the other way.
        let v = tm(-3, -60, 0, 0);
        assert_eq!(global_state(&v, 15), GlobalState::ScrollV(-0.25));
    }

    #[test]
    fn calc_tex_scroll_ramps() {
        // lo == 0: plain wrap.
        assert_eq!(calc_tex_scroll(0.0, 0.0, 8.0, 10), (0.25, 0.0));
        // Before the start: nothing.
        assert_eq!(calc_tex_scroll(0.0, 4.0, 16.0, 0), (0.0, 0.0));
        // After hi + lo: fully scrolled.
        let (s, scale) = calc_tex_scroll(30.0, 4.0, 16.0, 30);
        assert_eq!(s, 1.0);
        assert_eq!(scale, 4.0 - 1.0);
        // Start of the middle segment: scroll 1 - hi/lo, scale 1 - scroll.
        assert_eq!(calc_tex_scroll(4.0, 4.0, 16.0, 4), (-3.0, 4.0));
    }

    #[test]
    fn seq_flip_and_wrap() {
        let mut t = tm(166, 29, 1, 0);
        t.atree = 3;
        t.seq = 0;
        assert_eq!(seq_state(&t, -3), SeqState::Frame(0));
        assert_eq!(seq_state(&t, 5), SeqState::Frame(5));
        assert_eq!(seq_state(&t, 40), SeqState::Frame(28));
        // A repeating sequence wraps by the texmod's period.
        assert_eq!(seq_states(&[t.clone()], true, 30), vec![SeqState::Frame(1)]);
        assert_eq!(seq_states(&[t], false, 30), vec![SeqState::Frame(28)]);
        // Start frame and rate.
        let mut s = tm(115, 16, 2, 0);
        s.start_frame = 4;
        assert_eq!(seq_state(&s, 3), SeqState::Frame(0));
        assert_eq!(seq_state(&s, 9), SeqState::Frame(2));
    }

    #[test]
    fn seq_fades() {
        let mut t = tm(TexMod::FADE_OUT, 10, 0, 0);
        assert_eq!(seq_state(&t, 0), SeqState::Alpha(0));
        assert_eq!(seq_state(&t, 5), SeqState::Alpha(127));
        assert_eq!(seq_state(&t, 10), SeqState::Alpha(255));
        t.source = TexMod::FADE_IN;
        assert_eq!(seq_state(&t, 0), SeqState::Alpha(255));
        assert_eq!(seq_state(&t, 10), SeqState::Alpha(0));
    }
}
