struct Globals {
    view_proj: mat4x4<f32>,
    // xyz = direction towards the light, w = ambient
    light: vec4<f32>,
    // rgb = fog colour, w = fog density
    fog: vec4<f32>,
    camera_pos: vec4<f32>,
};

struct Model {
    transform: mat4x4<f32>,
    tint: vec4<f32>,
    // x = 1 when the batch has a lightmap, y = 1 when vertex colours are pre-lit
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var t_diffuse: texture_2d<f32>;
@group(1) @binding(1) var s_diffuse: sampler;
@group(1) @binding(2) var t_light: texture_2d<f32>;
@group(1) @binding(3) var s_light: sampler;
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
    o.uv = v.uv;
    o.lm_uv = v.lm_uv;
    o.world = world.xyz;
    return o;
}

fn shade(v: VsOut) -> vec4<f32> {
    let tex = textureSample(t_diffuse, s_diffuse, v.uv);
    var rgb = tex.rgb * m.tint.rgb;
    if (m.params.x > 0.5) {
        // Lightmapped world geometry: the lightmap carries the baked lighting.
        let lm = textureSample(t_light, s_light, v.lm_uv).rgb;
        rgb = rgb * lm * 2.0;
    } else if (m.params.y > 0.5) {
        rgb = rgb * v.color.rgb * 2.0;
    } else {
        let ndl = max(dot(normalize(v.normal), normalize(g.light.xyz)), 0.0);
        rgb = rgb * (g.light.w + (1.0 - g.light.w) * ndl);
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

@fragment
fn fs_blend(v: VsOut) -> @location(0) vec4<f32> {
    return shade(v);
}
