struct Globals {
    view_proj: mat4x4<f32>,
    // xyz = direction towards the light, w = ambient
    light: vec4<f32>,
    // rgb = fog colour, w = fog density
    fog: vec4<f32>,
    camera_pos: vec4<f32>,
    // x = relief normal strength, y = parallax depth in texture units
    relief: vec4<f32>,
};

struct Model {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    // x = 1 when the batch has a lightmap, y = 1 when vertex colours are pre-lit
    params: vec4<f32>,
    // Texture scroll record: u * x + y, v * z + w (texmod UV scrolls).
    uv_xf: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(1) @binding(1) var s_diffuse: sampler;
@group(1) @binding(2) var t_light: texture_2d<f32>;
@group(1) @binding(3) var s_light: sampler;
// Relief map: rgb = tangent-space normal (x along +u, y along +v), a = height.
@group(1) @binding(4) var t_bump: texture_2d<f32>;
@group(2) @binding(0) var<uniform> m: Model;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) lm_uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) lm_uv: vec2<f32>,
    @location(4) world: vec3<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    let world = m.transform * vec4<f32>(v.pos, 1.0);
    o.clip = g.view_proj * world;
    o.normal = normalize((m.transform * vec4<f32>(v.normal, 0.0)).xyz);
    o.color = v.color;
    o.uv = vec2<f32>(v.uv.x * m.uv_xf.x + m.uv_xf.y, v.uv.y * m.uv_xf.z + m.uv_xf.w);
    o.lm_uv = v.lm_uv;
    o.world = world.xyz;
    return o;
}

// Tangent frame from the screen-space derivatives of position and uv
// (Schüler's cotangent frame): no vertex tangents needed.
fn cotangent_frame(n: vec3<f32>, p: vec3<f32>, uv: vec2<f32>) -> mat3x3<f32> {
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let invmax = inverseSqrt(max(dot(t, t), dot(b, b)));
    return mat3x3<f32>(t * invmax, b * invmax, n);
}

fn shade(v: VsOut) -> vec4<f32> {
    let n0 = normalize(v.normal);
    let ddx = dpdx(v.uv);
    let ddy = dpdy(v.uv);
    var uv = v.uv;
    var nb = n0;
    if (g.relief.x > 0.0 || g.relief.y > 0.0) {
        let tbn = cotangent_frame(n0, v.world, v.uv);
        if (g.relief.y > 0.0) {
            // Steep parallax: march down through the height field along the
            // view ray until it is under the surface, then interpolate.
            let vw = normalize(g.camera_pos.xyz - v.world);
            let vt = vec3<f32>(dot(tbn[0], vw), dot(tbn[1], vw), dot(n0, vw));
            let steps = 12.0;
            let delta = vt.xy / max(vt.z, 0.25) * g.relief.y / steps;
            var depth = 0.0;
            var h = 1.0 - textureSampleGrad(t_bump, s_diffuse, uv, ddx, ddy).a;
            var i = 0;
            loop {
                if (depth >= h || i >= 12) {
                    break;
                }
                uv -= delta;
                depth += 1.0 / steps;
                h = 1.0 - textureSampleGrad(t_bump, s_diffuse, uv, ddx, ddy).a;
                i += 1;
            }
            if (i > 0) {
                let prev = uv + delta;
                let after = h - depth;
                let before = (1.0 - textureSampleGrad(t_bump, s_diffuse, prev, ddx, ddy).a)
                    - (depth - 1.0 / steps);
                let w = clamp(after / (after - before), 0.0, 1.0);
                uv = mix(uv, prev, w);
            }
        }
        let bn = textureSampleGrad(t_bump, s_diffuse, uv, ddx, ddy).xyz * 2.0 - 1.0;
        nb = normalize(tbn * normalize(vec3<f32>(bn.xy * g.relief.x, bn.z)));
    }
    let tex = textureSampleGrad(t_diffuse, s_diffuse, uv, ddx, ddy);
    let l = normalize(g.light.xyz);
    let lit_flat = g.light.w + (1.0 - g.light.w) * max(dot(n0, l), 0.0);
    let lit_bump = g.light.w + (1.0 - g.light.w) * max(dot(nb, l), 0.0);
    // Baked lighting keeps its level; the relief only shifts it locally.
    let relief = clamp(lit_bump / max(lit_flat, 0.05), 0.4, 1.6);
    var rgb = tex.rgb * m.tint.rgb;
    if (m.params.x > 0.5) {
        // Lightmapped world geometry: the lightmap carries the baked lighting.
        let lm = textureSample(t_light, s_light, v.lm_uv).rgb;
        rgb = rgb * lm * relief;
    } else if (m.params.y > 0.5) {
        rgb = rgb * v.color.rgb * 2.0 * relief;
    } else {
        rgb = rgb * lit_bump;
    }
    let dist = distance(v.world, g.camera_pos.xyz);
    let fog = 1.0 - exp(-g.fog.w * dist);
    rgb = mix(rgb, g.fog.rgb, clamp(fog, 0.0, 1.0));
    return vec4<f32>(rgb, tex.a * m.tint.a);
}

@fragment
fn fs_opaque(v: VsOut) -> @location(0) vec4<f32> {
    let c = shade(v);
    if (c.a < 0.5) {
        discard;
    }
    return vec4<f32>(c.rgb, 1.0);
}

// Multisampled: the alpha test becomes coverage. The alpha is sharpened to
// the width of a pixel so a cut-out's edge stays crisp at every mip level
// and takes its anti-aliasing from the samples.
@fragment
fn fs_opaque_aa(v: VsOut) -> @location(0) vec4<f32> {
    let c = shade(v);
    let a = clamp((c.a - 0.5) / max(fwidth(c.a), 1e-4) + 0.5, 0.0, 1.0);
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(c.rgb, a);
}

@fragment
fn fs_blend(v: VsOut) -> @location(0) vec4<f32> {
    return shade(v);
}

// Particles and streaks: world-space corners, texture x vertex colour
// (GXSetChanMatColor), GX's alpha compare (> 2).
struct SpriteIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct SpriteOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_sprite(v: SpriteIn) -> SpriteOut {
    var o: SpriteOut;
    o.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.uv = v.uv;
    o.color = v.color;
    return o;
}

@fragment
fn fs_sprite(v: SpriteOut) -> @location(0) vec4<f32> {
    let c = textureSample(t_diffuse, s_diffuse, v.uv) * v.color;
    if (c.a <= 2.0 / 255.0) {
        discard;
    }
    return c;
}
