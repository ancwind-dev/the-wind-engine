// Шейдеры мира: воксельные меши (карта, солдаты, эффекты), земля, карта теней.
//
// Цвета в вершинах — sRGB. Свет считаем в линейном пространстве и обратно
// переводим в sRGB: egui рисует в обычную (не sRGB) поверхность.

struct Point {
    // xy — центр, z — радиус, w — не используется.
    center_radius: vec4<f32>,
    // rgb — цвет (sRGB), a — сила заливки внутри.
    color: vec4<f32>,
};

struct Globals {
    view_proj: mat4x4<f32>,
    light_view_proj: mat4x4<f32>,
    // Направление НА солнце.
    sun_dir: vec4<f32>,
    // rgb — цвет солнца (линейный), w — сила тени (0..1).
    sun_color: vec4<f32>,
    ambient_sky: vec4<f32>,
    // rgb — свет снизу, w — начало тумана.
    ambient_ground: vec4<f32>,
    // rgb — цвет тумана (sRGB), w — плотность.
    fog: vec4<f32>,
    eye: vec4<f32>,
    // x — размер карты, y — время, z — число точек, w — размер текселя тени в UV.
    params: vec4<f32>,
    points: array<Point, 32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

fn shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    let p = g.light_view_proj * vec4<f32>(world + n * 0.04, 1.0);
    let ndc = p.xyz / p.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }
    // 3×3 выборки — мягкий край тени.
    let t = g.params.w;
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            lit += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(f32(x), f32(y)) * t, ndc.z - 0.0008);
        }
    }
    return mix(1.0, lit / 9.0, g.sun_color.w);
}

fn light(albedo_srgb: vec3<f32>, n: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let albedo = to_linear(albedo_srgb);
    let ndl = max(dot(n, g.sun_dir.xyz), 0.0);
    let sky = mix(g.ambient_ground.rgb, g.ambient_sky.rgb, n.y * 0.5 + 0.5);
    let color = albedo * (sky + g.sun_color.rgb * ndl * shadow(world, n));
    return to_srgb(color);
}

fn fogged(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    // Туман начинается за точкой, на которую смотрит камера (ambient_ground.w) —
    // так он не зависит от того, насколько камера отдалена.
    let d = max(distance(world, g.eye.xyz) - g.ambient_ground.w, 0.0);
    let f = 1.0 - exp(-d * g.fog.w);
    return mix(color, g.fog.rgb, clamp(f, 0.0, 1.0));
}

// --- Воксельные меши --------------------------------------------------------

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
    // rgb — цвет команды или множитель цвета, a — режим (см. vs_voxel).
    @location(7) tint: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) emissive: f32,
};

@vertex
fn vs_voxel(v: VIn) -> VOut {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    let world = model * vec4<f32>(v.pos, 1.0);
    var out: VOut;
    out.clip = g.view_proj * world;
    out.world = world.xyz;
    out.normal = normalize((model * vec4<f32>(v.normal.xyz, 0.0)).xyz);
    // tint.a: 0 — солдат (воксели с альфой 0 красятся в цвет команды),
    // 0.5 — освещённый, цвет умножается на tint, 1 — светится сам.
    if (v.color.a < 0.5) {
        out.color = v.tint.rgb;
    } else if (v.tint.a > 0.25) {
        out.color = v.color.rgb * v.tint.rgb;
    } else {
        out.color = v.color.rgb;
    }
    out.emissive = step(0.75, v.tint.a);
    return out;
}

@fragment
fn fs_voxel(in: VOut) -> @location(0) vec4<f32> {
    var c: vec3<f32>;
    if (in.emissive > 0.5) {
        c = in.color;
    } else {
        c = light(in.color, normalize(in.normal), in.world);
    }
    return vec4<f32>(fogged(c, in.world), 1.0);
}

@vertex
fn vs_shadow(v: VIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    return g.light_view_proj * model * vec4<f32>(v.pos, 1.0);
}

// --- Земля -----------------------------------------------------------------

struct GOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_ground(@builtin(vertex_index) i: u32) -> GOut {
    // Прямоугольник с запасом вокруг карты.
    let size = g.params.x;
    let lo = -size;
    let hi = size * 2.0;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(lo, lo), vec2<f32>(lo, hi), vec2<f32>(hi, hi),
        vec2<f32>(lo, lo), vec2<f32>(hi, hi), vec2<f32>(hi, lo),
    );
    let c = corners[i];
    var out: GOut;
    out.world = vec3<f32>(c.x, -0.05, c.y);
    out.clip = g.view_proj * vec4<f32>(out.world, 1.0);
    return out;
}

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

// Плавный шум: значения в узлах сетки, между ними — сглаженная интерполяция.
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (vec2<f32>(3.0) - 2.0 * f);
    let a = mix(hash2(i), hash2(i + vec2<f32>(1.0, 0.0)), u.x);
    let b = mix(hash2(i + vec2<f32>(0.0, 1.0)), hash2(i + vec2<f32>(1.0, 1.0)), u.x);
    return mix(a, b, u.y);
}

// Покрытие земли: номер материала на клетку (см. GROUND_NAMES в wind_sim::map).
@group(1) @binding(0) var ground_tex: texture_2d<u32>;

fn material(m: u32, p: vec2<f32>) -> vec3<f32> {
    var c: vec3<f32>;
    switch m {
        case 1u: { c = vec3<f32>(0.52, 0.50, 0.33); }  // сухая трава
        case 2u: { c = vec3<f32>(0.47, 0.40, 0.30); }  // земля
        case 3u: { c = vec3<f32>(0.74, 0.68, 0.50); }  // песок
        case 4u: { c = vec3<f32>(0.30, 0.30, 0.31); }  // асфальт
        case 5u: { c = vec3<f32>(0.62, 0.61, 0.58); }  // бетон
        default: { c = vec3<f32>(0.42, 0.48, 0.28); }  // трава
    }
    // Лёгкая разница по клеткам (воксельный вид) и мягкие пятна внутри материала.
    return c * (0.95 + 0.05 * hash2(floor(p))) * (0.94 + 0.08 * noise(p / 4.0));
}

// Зоны захвата, метка своего солдата, кисть редактора.
fn zones(albedo_in: vec3<f32>, p: vec2<f32>) -> vec3<f32> {
    var albedo = albedo_in;
    let count = i32(g.params.z);
    for (var k = 0; k < count; k++) {
        let pt = g.points[k];
        let d = distance(p, pt.center_radius.xy);
        let r = pt.center_radius.z;
        let ring = 1.0 - smoothstep(0.05, 0.25, abs(d - r));
        if (d < r) {
            albedo = mix(albedo, pt.color.rgb, pt.color.a);
        }
        albedo = mix(albedo, pt.color.rgb, ring * 0.9);
    }
    return albedo;
}

@fragment
fn fs_terrain(in: VOut) -> @location(0) vec4<f32> {
    let p = in.world.xz;
    let size = vec2<i32>(textureDimensions(ground_tex));
    // Берём материал со сдвигом по шуму — границы получаются неровные, «рисованные».
    let jitter = vec2<f32>(noise(p * 0.35) - 0.5, noise(p * 0.35 + vec2<f32>(31.0, 17.0)) - 0.5) * 1.4;
    let cell = clamp(vec2<i32>(floor(p + jitter)), vec2<i32>(0), size - vec2<i32>(1));
    let m = textureLoad(ground_tex, cell, 0).r;
    let albedo = zones(material(m, p), p);
    let c = light(albedo, normalize(in.normal), in.world);
    return vec4<f32>(fogged(c, in.world), 1.0);
}

// За краем карты — тёмная подложка; внутри её закрывает рельеф.
@fragment
fn fs_ground(in: GOut) -> @location(0) vec4<f32> {
    let p = in.world.xz;
    let size = g.params.x;
    let albedo = vec3<f32>(0.42, 0.48, 0.28) * 0.45 * (0.95 + 0.05 * hash2(floor(p)));
    let c = light(albedo, vec3<f32>(0.0, 1.0, 0.0), in.world);
    return vec4<f32>(fogged(c, in.world), 1.0);
}

// --- Свои солдаты ------------------------------------------------------------
// Цвет вокселя — из 3D-атласа: вершина несёт координату угла вокселя в атласе,
// а фрагмент берёт воксель «за» гранью (сдвиг на полвокселя против нормали).

@group(2) @binding(0) var char_atlas: texture_3d<f32>;

struct CIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) atlas: vec4<u32>,
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
    @location(7) tint: vec4<f32>,
};

struct COut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // centroid — иначе при сглаживании точка выборки может выйти за грань.
    @location(2) @interpolate(perspective, centroid) atlas: vec3<f32>,
    @location(3) @interpolate(flat) local_normal: vec3<f32>,
};

@vertex
fn vs_char(v: CIn) -> COut {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    let world = model * vec4<f32>(v.pos, 1.0);
    var out: COut;
    out.clip = g.view_proj * world;
    out.world = world.xyz;
    out.normal = normalize((model * vec4<f32>(v.normal.xyz, 0.0)).xyz);
    out.atlas = vec3<f32>(v.atlas.xyz);
    out.local_normal = v.normal.xyz;
    return out;
}

@fragment
fn fs_char(in: COut) -> @location(0) vec4<f32> {
    let cell = vec3<i32>(floor(in.atlas - in.local_normal * 0.5));
    let albedo = textureLoad(char_atlas, cell, 0).rgb;
    let c = light(albedo, normalize(in.normal), in.world);
    return vec4<f32>(fogged(c, in.world), 1.0);
}

@vertex
fn vs_char_shadow(v: CIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(v.m0, v.m1, v.m2, v.m3);
    return g.light_view_proj * model * vec4<f32>(v.pos, 1.0);
}
