//! Floating point that gives the same bits on every platform.
//!
//! IEEE 754 fixes add, subtract, multiply, divide and square root, but not
//! sine, arctangent or power: `f32::sin` and friends call whatever the
//! platform's C library provides, and glam's SIMD paths round differently
//! from its scalar ones, so an x86 and an ARM machine playing the same inputs
//! drift apart within seconds. Peers of a networked game must not.
//!
//! The simulation therefore takes these functions from the pure-Rust `libm`
//! crate, through the `det_` methods here, and glam is built with its `libm`
//! and `scalar-math` features for the same reason. Square root, rounding,
//! `abs` and `%` are exact everywhere and stay as they are.
//!
//! The tests hold the sim to this: one scans its sources for the platform's
//! functions, one pins the results here to known bits.

/// `f32` maths with the same result on every platform.
pub trait Det {
    fn det_sin(self) -> Self;
    fn det_cos(self) -> Self;
    fn det_tan(self) -> Self;
    fn det_asin(self) -> Self;
    fn det_acos(self) -> Self;
    fn det_atan(self) -> Self;
    fn det_atan2(self, x: Self) -> Self;
    fn det_sin_cos(self) -> (Self, Self)
    where
        Self: Sized;
    fn det_hypot(self, y: Self) -> Self;
    fn det_powf(self, e: Self) -> Self;
    fn det_powi(self, e: i32) -> Self;
}

impl Det for f32 {
    fn det_sin(self) -> f32 {
        libm::sinf(self)
    }
    fn det_cos(self) -> f32 {
        libm::cosf(self)
    }
    fn det_tan(self) -> f32 {
        libm::tanf(self)
    }
    fn det_asin(self) -> f32 {
        libm::asinf(self)
    }
    fn det_acos(self) -> f32 {
        libm::acosf(self)
    }
    fn det_atan(self) -> f32 {
        libm::atanf(self)
    }
    fn det_atan2(self, x: f32) -> f32 {
        libm::atan2f(self, x)
    }
    fn det_sin_cos(self) -> (f32, f32) {
        libm::sincosf(self)
    }
    fn det_hypot(self, y: f32) -> f32 {
        libm::hypotf(self, y)
    }
    fn det_powf(self, e: f32) -> f32 {
        libm::powf(self, e)
    }
    /// Square-and-multiply: `f32::powi` is whatever the compiler makes of it.
    fn det_powi(self, e: i32) -> f32 {
        let mut base = self;
        let mut n = e.unsigned_abs();
        let mut out = 1.0f32;
        while n > 0 {
            if n & 1 != 0 {
                out *= base;
            }
            base *= base;
            n >>= 1;
        }
        if e < 0 {
            1.0 / out
        } else {
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Quat, Vec3};

    /// The platform functions the simulation must not call, and the file
    /// allowed to mention them.
    const FORBIDDEN: &[&str] = &[
        ".sin(", ".cos(", ".tan(", ".asin(", ".acos(", ".atan(", ".atan2(", ".sin_cos(",
        ".sinh(", ".cosh(", ".tanh(", ".hypot(", ".powf(", ".powi(", ".exp(", ".exp2(",
        ".exp_m1(", ".ln(", ".ln_1p(", ".log(", ".log2(", ".log10(", ".cbrt(", ".mul_add(",
    ];

    fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                sources(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }

    #[test]
    fn the_sim_calls_no_platform_maths() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        sources(&root.join("src"), &mut files);
        sources(&root.join("../gdl-formats/src"), &mut files);
        let mut found = Vec::new();
        for f in files {
            // Movies are decoded to pictures, not simulated.
            if f.ends_with("gdl-formats/src/movie.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&f).unwrap();
            for (n, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap();
                for bad in FORBIDDEN {
                    if code.contains(bad) && !code.contains("\"") {
                        found.push(format!("{}:{}: {bad}", f.display(), n + 1));
                    }
                }
            }
        }
        assert!(found.is_empty(), "use crate::math::Det: {found:#?}");
    }

    /// The bits of every function over a sweep of inputs, and of the glam
    /// operations the sim leans on. A platform whose result differs fails
    /// here, whatever level would have shown it. The constant changes only if
    /// the libm or glam dependency does; peers on different versions of them
    /// would not agree anyway.
    #[test]
    fn the_maths_gives_the_pinned_bits() {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut put = |v: f32| {
            for b in v.to_bits().to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        let mut x = -40.0f32;
        while x < 40.0 {
            let y = x * 0.37 + 0.11;
            let unit = (x / 40.0).clamp(-1.0, 1.0);
            let (s, c) = x.det_sin_cos();
            for v in [
                x.det_sin(),
                x.det_cos(),
                x.det_tan(),
                unit.det_asin(),
                unit.det_acos(),
                x.det_atan(),
                y.det_atan2(x),
                s,
                c,
                x.det_hypot(y),
                x.abs().det_powf(1.7),
                x.det_powi(3),
                x.det_powi(-2),
            ] {
                put(v);
            }
            let q = Quat::from_rotation_y(x) * Quat::from_axis_angle(Vec3::X, y);
            let m = Mat4::from_rotation_y(x)
                * Mat4::from_rotation_translation(q.slerp(Quat::IDENTITY, 0.3), Vec3::splat(y));
            let v = Mat4::look_at_rh(Vec3::new(x, y, 3.0), Vec3::ZERO, Vec3::Y)
                * Mat4::perspective_rh(1.0, 4.0 / 3.0, 1.0, 1500.0);
            for a in [m.inverse(), v * m] {
                a.to_cols_array().iter().copied().for_each(&mut put);
            }
            let n = Vec3::new(x, y, 1.0).normalize_or_zero();
            put(n.x);
            put(n.angle_between(Vec3::Y));
            x += 0.173;
        }
        assert_eq!(h, PINNED, "got {h:#018x}");
    }

    const PINNED: u64 = 0x2b39_2700_c6e9_bb29;

    #[test]
    fn powers_are_right() {
        assert_eq!(2.0f32.det_powi(10), 1024.0);
        assert_eq!(2.0f32.det_powi(-2), 0.25);
        assert_eq!(5.0f32.det_powi(0), 1.0);
    }
}
