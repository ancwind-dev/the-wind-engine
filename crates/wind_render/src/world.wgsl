// Шейдеры мира: воксельные меши (карта, солдаты, эффекты), земля, карта теней.
//
// Цвета в вершинах — sRGB. Свет считаем в линейном пространстве и обратно
// переводим в sRGB: egui рисует в обычную (не sRGB) поверхность.

struct Point {
    // xy — центр, z — радиус, w — полуширина чёткого кольца (0 — мягкое, широкое).
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
    view: mat4x4<f32>,
    // Насыщенность, контраст, яркость, ступенчатость света.
    grade: vec4<f32>,
    tint: vec4<f32>,
    // Обводка, затенение, виньетка, дальность для глубины.
    post: vec4<f32>,
    // Вьюпорт в пикселях кадра: x, y, ширина, высота.
    screen: vec4<f32>,
    // x — ширина плавного сгиба у суставов солдат, воксели (0 — без сгиба).
    skin: vec4<f32>,
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
    return lit(albedo_srgb, n, shadow(world, n));
}

fn lit(albedo_srgb: vec3<f32>, n: vec3<f32>, sh: f32) -> vec3<f32> {
    let albedo = to_linear(albedo_srgb);
    let direct = max(dot(n, g.sun_dir.xyz), 0.0) * sh;
    // Ступени, как в мультфильме: тень — полутень — свет, с узкими мягкими переходами.
    let stepped = smoothstep(0.05, 0.12, direct) * 0.55 + smoothstep(0.42, 0.52, direct) * 0.45;
    let d = mix(direct, stepped, g.grade.w);
    // Тень окрашена цветом тени, а не просто темнее.
    let ambient = mix(g.ambient_ground.rgb, g.ambient_sky.rgb, n.y * 0.5 + 0.5);
    return to_srgb(albedo * (ambient + g.sun_color.rgb * d));
}

// «Плёнка»: оттенок, яркость, насыщенность, контраст.
fn grade(c: vec3<f32>) -> vec3<f32> {
    var x = c * g.tint.rgb * g.grade.z;
    let luma = dot(x, vec3<f32>(0.299, 0.587, 0.114));
    x = mix(vec3<f32>(luma), x, g.grade.x);
    x = (x - vec3<f32>(0.5)) * g.grade.y + vec3<f32>(0.5);
    return clamp(x, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn fogged(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    // Туман начинается за точкой, на которую смотрит камера (ambient_ground.w) —
    // так он не зависит от того, насколько камера отдалена.
    let d = max(distance(world, g.eye.xyz) - g.ambient_ground.w, 0.0);
    let f = 1.0 - exp(-d * g.fog.w);
    return grade(mix(color, g.fog.rgb, clamp(f, 0.0, 1.0)));
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
    // Узел сетки — целыми числами (PCG). Через sin() на больших координатах один и тот же
    // узел, посчитанный из соседних ячеек, давал разные значения — шум рвался по линиям
    // сетки, и на земле и траве проступали наложенные квадраты.
    var v = bitcast<vec2<u32>>(vec2<i32>(round(p)));
    v = v * 1664525u + vec2<u32>(1013904223u);
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    return f32(v.x >> 8u) / 16777216.0;
}

// Градиентный шум (Perlin), около −0,7…0,7: в узлах сетки — случайные направления, пятна
// получаются плавные и круглые, без граней по сетке.
fn perlin(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - vec2<f32>(15.0)) + vec2<f32>(10.0));
    let a = dot(gradient(i), f);
    let b = dot(gradient(i + vec2<f32>(1.0, 0.0)), f - vec2<f32>(1.0, 0.0));
    let c = dot(gradient(i + vec2<f32>(0.0, 1.0)), f - vec2<f32>(0.0, 1.0));
    let d = dot(gradient(i + vec2<f32>(1.0, 1.0)), f - vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn gradient(i: vec2<f32>) -> vec2<f32> {
    let a = hash2(i) * 6.2831853;
    return vec2<f32>(cos(a), sin(a));
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
// Следы боя на всю карту: r — подпалины, g — кровь.
@group(1) @binding(2) var decal_tex: texture_2d<f32>;
@group(1) @binding(3) var decal_sampler: sampler;

fn battle_traces(albedo_in: vec3<f32>, p: vec2<f32>, map_size: vec2<f32>) -> vec3<f32> {
    let t = textureSample(decal_tex, decal_sampler, p / map_size).rg;
    // Неровные края и пятнистость — чтобы не было ровных кругов.
    let n = noise(p * 1.7);
    let scorch = smoothstep(0.05, 0.85, t.r - (n - 0.5) * 0.35);
    let blood = smoothstep(0.35, 0.7, t.g - (n - 0.5) * 0.4);
    var albedo = mix(albedo_in, vec3<f32>(0.13, 0.11, 0.10) * (0.8 + 0.4 * n), scorch * 0.75);
    albedo = mix(albedo, vec3<f32>(0.22, 0.05, 0.04), blood * 0.6);
    return albedo;
}

// --- Земля: нарисованная, по приёму RWR -----------------------------------------
// Ковра травинок нет (см. «Растительность — как у RWR, без ветра и ковра»): трава — это
// раскраска земли. Крупные пятна осенних тонов (палитра травы хозяина), короткие
// наклонные мазки травы, гуще пучками, и плавные «рисованные» границы между покрытиями.

// Крупные пятна (0..1): градиентный шум в три октавы — круглые, без граней по сетке.
fn ground_patches(p: vec2<f32>) -> f32 {
    let q = p * 0.07;
    let n = perlin(q) * 0.6 + perlin(q * 2.3 + vec2<f32>(5.2, 1.3)) * 0.3 + perlin(q * 5.1 + vec2<f32>(1.7, 9.2)) * 0.1;
    return clamp(0.5 + n * 0.9, 0.0, 1.0);
}

// Тон покрытия без мазков (sRGB) при пятнистости `n`.
fn ground_tone(m: u32, n: f32, p: vec2<f32>) -> vec3<f32> {
    var c: vec3<f32>;
    switch m {
        // Сухая трава: солома, светлее на «высоких» пятнах.
        case 1u: { c = mix(vec3<f32>(0.55, 0.46, 0.27), vec3<f32>(0.68, 0.58, 0.36), smoothstep(0.3, 0.8, n)); }
        case 2u: { c = vec3<f32>(0.47, 0.40, 0.30) * (0.92 + 0.12 * n); }  // земля
        case 3u: { c = vec3<f32>(0.74, 0.68, 0.50) * (0.95 + 0.07 * n); }  // песок
        case 4u: { c = vec3<f32>(0.30, 0.30, 0.31); }  // асфальт
        case 5u: { c = vec3<f32>(0.62, 0.61, 0.58); }  // бетон
        // Трава (осенняя): тёмная олива → охра → приглушённое золото.
        default: {
            c = mix(vec3<f32>(0.33, 0.33, 0.17), vec3<f32>(0.50, 0.44, 0.21), smoothstep(0.25, 0.55, n));
            c = mix(c, vec3<f32>(0.63, 0.52, 0.25), smoothstep(0.6, 0.85, n));
        }
    }
    // Чуть-чуть мелкого шума — без него большие пятна выглядят пластиком.
    return c * (0.96 + 0.08 * noise(p * 0.9));
}

// Мазки травы: пучки из 2–3 коротких стеблей веером от одного корня — как нарисованная
// трава у RWR2. Пучок «смотрит» от камеры: на экране стебли торчат вверх, будто трава
// стоит, хотя это раскраска земли. Тёмные стебли — тень между травой, изредка светлые —
// освещённые кончики. Возвращает (тёмные, светлые) 0..1. Гуще — пучками по шуму; вдали
// гаснут (`fade`), чтобы не рябило.
fn grass_strokes(p: vec2<f32>, fade: f32) -> vec2<f32> {
    if (fade <= 0.0) {
        return vec2<f32>(0.0);
    }
    let cell = 0.5;
    let base = floor(p / cell);
    let clump = smoothstep(0.3, 0.75, noise(p * 0.45));
    // От камеры по земле — «вверх» на экране.
    let away = normalize(p - g.eye.xz + vec2<f32>(1e-3, 0.0));
    let up_angle = atan2(away.y, away.x);
    var dark = 0.0;
    var light = 0.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let c = base + vec2<f32>(f32(dx), f32(dy));
            let h1 = hash2(c);
            let h2 = hash2(c + vec2<f32>(17.0, 3.0));
            let h3 = hash2(c + vec2<f32>(41.0, 29.0));
            // Вне пучков — реже.
            if (h3 > 0.4 + 0.55 * clump) {
                continue;
            }
            let root = (c + vec2<f32>(0.15 + 0.7 * h1, 0.15 + 0.7 * h2)) * cell;
            let bright = h3 < 0.07;
            let blades = 2 + i32(h2 * 1.99);
            for (var b = 0; b < blades; b++) {
                let fb = f32(b) - f32(blades - 1) * 0.5;
                let a = up_angle + fb * 0.5 + (hash2(c + vec2<f32>(f32(b) * 7.0, 11.0)) - 0.5) * 0.35;
                let dir = vec2<f32>(cos(a), sin(a));
                let len = 0.13 + 0.14 * hash2(c + vec2<f32>(5.0, f32(b) * 13.0));
                let t = clamp(dot(p - root, dir), 0.0, len);
                let d = length(p - root - dir * t);
                // К кончику тоньше.
                let w = 0.03 * (1.0 - 0.7 * t / len);
                let s = 1.0 - smoothstep(w * 0.5, w, d);
                if (bright) {
                    light = max(light, s);
                } else {
                    dark = max(dark, s);
                }
            }
        }
    }
    return vec2<f32>(dark, light) * fade;
}

// Раскраска земли в точке (sRGB, до света): покрытия четырёх соседних клеток смешиваются
// через неровный край (сдвиг по шуму + сглаженная граница ~0,6 м), на траве — мазки.
fn ground_color(p: vec2<f32>, eye_dist: f32) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(ground_tex));
    let jitter = vec2<f32>(noise(p * 0.35) - 0.5, noise(p * 0.35 + vec2<f32>(31.0, 17.0)) - 0.5) * 1.4;
    let q = p + jitter - vec2<f32>(0.5);
    let i = vec2<i32>(floor(q));
    let f = smoothstep(vec2<f32>(0.2), vec2<f32>(0.8), fract(q));
    let n = ground_patches(p);
    var color = vec3<f32>(0.0);
    var grassy = 0.0;
    for (var k = 0; k < 4; k++) {
        let o = vec2<i32>(k & 1, k >> 1);
        let cell = clamp(i + o, vec2<i32>(0), size - vec2<i32>(1));
        let m = textureLoad(ground_tex, cell, 0).r;
        let wx = select(1.0 - f.x, f.x, o.x == 1);
        let wy = select(1.0 - f.y, f.y, o.y == 1);
        color += ground_tone(m, n, p) * (wx * wy);
        grassy += select(0.0, wx * wy, m <= 1u);
    }
    if (grassy > 0.01) {
        let fade = 1.0 - smoothstep(45.0, 95.0, eye_dist);
        let s = grass_strokes(p, fade * grassy);
        color = mix(color, color * vec3<f32>(0.66, 0.64, 0.58), s.x * 0.7);
        color = mix(color, color * 1.25 + vec3<f32>(0.03, 0.025, 0.0), s.y * 0.7);
    }
    return color;
}

// Зоны захвата, метка своего солдата, кисть редактора.
fn zones(albedo_in: vec3<f32>, p: vec2<f32>) -> vec3<f32> {
    var albedo = albedo_in;
    let count = i32(g.params.z);
    for (var k = 0; k < count; k++) {
        let pt = g.points[k];
        let d = distance(p, pt.center_radius.xy);
        let r = pt.center_radius.z;
        let half = pt.center_radius.w;
        var ring = 1.0 - smoothstep(0.05, 0.25, abs(d - r));
        if (half > 0.0) {
            // Чёткое кольцо (метка своего солдата, как в RWR): ровная полоса с узким краем.
            ring = 1.0 - smoothstep(half - 0.02, half + 0.02, abs(d - r));
        }
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
    let albedo = zones(battle_traces(ground_color(p, distance(in.world, g.eye.xyz)), p, vec2<f32>(size)), p);
    let c = light(albedo, normalize(in.normal), in.world);
    return vec4<f32>(fogged(c, in.world), 1.0);
}

// За краем карты — тёмная подложка; внутри её закрывает рельеф.
@fragment
fn fs_ground(in: GOut) -> @location(0) vec4<f32> {
    let p = in.world.xz;
    let size = g.params.x;
    let albedo = vec3<f32>(0.47, 0.43, 0.24) * 0.45 * (0.95 + 0.05 * hash2(floor(p)));
    let c = light(albedo, vec3<f32>(0.0, 1.0, 0.0), in.world);
    return vec4<f32>(fogged(c, in.world), 1.0);
}

// --- Свои солдаты ------------------------------------------------------------
// Как у RWR: каждый воксель — квадратик постоянного размера на экране. Свет без
// направления и без теней — цвет вокселя чуть ярче, а нижняя половина каждой точки
// чуть темнее (отсюда «пиксельная» фактура). Обводка — те же точки, крупнее и чёрные,
// чуть дальше от камеры. Где солдата закрывает стена — силуэт цвета команды.
//
// Матрицы палок всех солдат лежат подряд в одном буфере; экземпляр знает, с какой
// начинаются его палки. Воксель у сустава смешивает положение между своей палкой
// и соседней — сгиб получается плавным, а не ступенькой.

@group(1) @binding(4) var<storage, read> bones: array<mat4x4<f32>>;

// Экземпляр: x — первая матрица палок, y — бит 0: оружие, z — цвет команды (RGBA8).
struct CIn {
    // Позиция в позе покоя, м.
    @location(0) pos: vec3<f32>,
    @location(1) bone: u32,
    @location(3) inst: vec4<u32>,
};

@vertex
fn vs_char_shadow(v: CIn) -> @builtin(position) vec4<f32> {
    return g.light_view_proj * bones[v.inst.x + v.bone] * vec4<f32>(v.pos, 1.0);
}

struct PIn {
    // Центр вокселя в позе покоя, м.
    @location(0) pos: vec3<f32>,
    // rgb — цвет (sRGB 0..255), a — угол квадратика 0..3 и уровень детализации (биты 2..3).
    @location(1) color: vec4<u32>,
    // x — палка, y — соседняя палка у сустава, z — расстояние до сустава (¼ вокселя).
    @location(2) bones: vec4<u32>,
    @location(3) inst: vec4<u32>,
};

// Где воксель в мире: у сустава — между своей палкой и соседней. На самом суставе
// пополам (обе палки сходятся в этой точке), дальше ширины сгиба — только своя.
fn skinned(v: PIn) -> vec3<f32> {
    let p = vec4<f32>(v.pos, 1.0);
    let own = (bones[v.inst.x + v.bones.x] * p).xyz;
    let width = g.skin.x;
    if (v.bones.y == v.bones.x || width <= 0.0) {
        return own;
    }
    let w = 0.5 * (1.0 - smoothstep(0.0, width, f32(v.bones.z) * 0.25));
    if (w <= 0.0) {
        return own;
    }
    let other = (bones[v.inst.x + v.bones.y] * p).xyz;
    return mix(own, other, w);
}

fn is_weapon(v: PIn) -> bool {
    return (v.inst.y & 1u) != 0u;
}

struct POut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) @interpolate(flat) color: vec3<f32>,
    // Положение внутри квадратика: −1..1, y вверх.
    @location(2) corner: vec2<f32>,
    @location(3) @interpolate(flat) weapon: f32,
};

// Размер вокселя модели, м.
const CHAR_VOXEL: f32 = 1.0 / 32.0;

// Солдаты «точно как RWR» (g.skin.y = 1, по умолчанию) или «как было» (0) — сверено
// с soldier_skin.cg и материалами RWR 2026-10-28:
// - размеры точек — в пикселях экрана при любом разрешении (2,6; обводка 4,2;
//   силуэт 3,0; в проходе нормалей 2,3), а не «как при 1080p»;
// - оружие без света, нижняя половина точки × 0,55.
fn rwr_exact() -> bool {
    return g.skin.y > 0.5;
}

// `scale` — множитель размера точки, `extra` — сколько пикселей добавить (при 1080p),
// `push` — сдвиг к камере, м (минус — от камеры).
fn point_out(v: PIn, scale: f32, extra: f32, push: f32) -> POut {
    let world = skinned(v);
    let to_eye = normalize(g.eye.xyz - world);
    var clip = g.view_proj * vec4<f32>(world + to_eye * push, 1.0);
    // У RWR точка — 2,6 пикселя экрана и не зависит ни от расстояния, ни от разрешения
    // («как было» — 2,6 пикселя при 1080p). Вблизи мы не даём ей стать меньше полутора
    // вокселей, иначе между точками появятся щели (у RWR так близко камера не бывает).
    let res = select(g.screen.w / 1080.0, 1.0, rwr_exact());
    let voxel_px = CHAR_VOXEL * g.tint.w / max(clip.w, 0.01);
    // На уровне детализации n точка заменяет кубик 2ⁿ вокселей.
    let lod = f32(1u << (v.color.a >> 2u));
    // Вдали солдат — несколько пикселей, и обводка съела бы его целиком: убираем её
    // плавно, пока воксель меньше 0,6 пикселя (при 1080p).
    let far = smoothstep(0.2, 0.6, voxel_px / res);
    let size = max(voxel_px * lod * 1.5, 2.6 * res) * scale + extra * far * res;
    let corners = array<vec2<f32>, 4>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let c = corners[v.color.a & 3u];
    clip = vec4<f32>(clip.xy + c * size / g.screen.zw * clip.w, clip.zw);
    var out: POut;
    out.clip = clip;
    out.world = world;
    out.color = vec3<f32>(v.color.rgb) / 255.0;
    out.corner = c;
    out.weapon = select(0.0, 1.0, is_weapon(v));
    return out;
}

@vertex
fn vs_point(v: PIn) -> POut {
    // У RWR точки оружия чуть крупнее (3,0 против 2,6).
    return point_out(v, select(1.0, 1.15, is_weapon(v)), 0.0, 0.0);
}

@vertex
fn vs_point_outline(v: PIn) -> POut {
    // У оружия обводки нет: квадратик нулевого размера ничего не рисует.
    if (is_weapon(v)) {
        return point_out(v, 0.0, 0.0, 0.0);
    }
    return point_out(v, 1.0, 1.6, -0.08);
}

@vertex
fn vs_point_ghost(v: PIn) -> POut {
    // Ближе к камере на полметра — чтобы земля под ногами не считалась «стеной».
    // Силуэт врага не рисуем: квадратик нулевого размера (бит 1 признаков снят).
    // У RWR точки силуэта крупнее основных: 3,0 против 2,6.
    let shown = select(0.0, 1.0, (v.inst.y & 2u) != 0u) * select(1.0, 3.0 / 2.6, rwr_exact());
    var out = point_out(v, shown, 0.0, 0.5);
    out.color = unpack4x8unorm(v.inst.z).rgb;
    return out;
}

// Солдат в проходе нормалей и глубины: у RWR точки там мельче (2,3 против 2,6).
@vertex
fn vs_point_nd(v: PIn) -> POut {
    return point_out(v, select(1.0, 2.3 / 2.6, rwr_exact()), 0.0, 0.0);
}

@fragment
fn fs_point(in: POut) -> @location(0) vec4<f32> {
    let weapon = in.weapon > 0.5;
    // Свет сверху без направления: небо + доля солнца. Оружие у RWR — без света.
    let l = g.ambient_sky.rgb + g.sun_color.rgb * 0.7;
    var c = to_srgb(to_linear(in.color) * l);
    if (weapon && rwr_exact()) {
        c = in.color;
    }
    // Нижняя половина точки темнее; у оружия — сильнее (у RWR 0,55).
    let shade = select(0.85, select(0.6, 0.55, rwr_exact()), weapon);
    c *= select(1.0, shade, in.corner.y < 0.0);
    return vec4<f32>(fogged(c, in.world), 1.0);
}

@fragment
fn fs_point_outline(in: POut) -> @location(0) vec4<f32> {
    return vec4<f32>(fogged(vec3<f32>(0.0), in.world), 1.0);
}

@fragment
fn fs_point_ghost(in: POut) -> @location(0) vec4<f32> {
    let c = to_linear(in.color) * 0.6 + g.ambient_sky.rgb * 0.5;
    return vec4<f32>(to_srgb(c), 1.0);
}

// --- Пыль и дым -------------------------------------------------------------
// Мягкий круг, повёрнутый к камере. Свет — небо и немного солнца, туман — как у всех.

struct ParticleIn {
    @builtin(vertex_index) i: u32,
    @location(0) pos: vec3<f32>,
    @location(1) size: f32,
    @location(2) color: vec4<f32>,
};

struct ParticleOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
};

@vertex
fn vs_particle(v: ParticleIn) -> ParticleOut {
    // Оси камеры в мире — строки поворота матрицы вида.
    let right = vec3<f32>(g.view[0][0], g.view[1][0], g.view[2][0]);
    let up = vec3<f32>(g.view[0][1], g.view[1][1], g.view[2][1]);
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[v.i];
    // Ближе к камере на полразмера: плоский клуб не врезается в землю и стены
    // (иначе край обрезается ступеньками по клеткам).
    let to_eye = normalize(g.eye.xyz - v.pos);
    let world = v.pos + to_eye * v.size * 0.5 + (right * c.x + up * c.y) * v.size * 0.5;
    var out: ParticleOut;
    out.clip = g.view_proj * vec4<f32>(world, 1.0);
    out.uv = c;
    out.color = v.color;
    out.world = world;
    return out;
}

@fragment
fn fs_particle(in: ParticleOut) -> @location(0) vec4<f32> {
    let d = length(in.uv);
    // Край неровный: клуб, а не диск.
    let wobble = 0.12 * sin(atan2(in.uv.y, in.uv.x) * 5.0 + in.color.r * 40.0);
    let a = in.color.a * (1.0 - smoothstep(0.35, 1.0 + wobble, d));
    if (a <= 0.003) {
        discard;
    }
    let l = g.ambient_sky.rgb + g.sun_color.rgb * 0.45;
    let c = to_srgb(to_linear(in.color.rgb) * l);
    return vec4<f32>(fogged(c, in.world), a);
}

// --- Нормали и глубина (для обводки) -------------------------------------------------
// Как у RWR: отдельно рисуем нормали в пространстве камеры и расстояние до камеры,
// а потом по ним находим края.

@fragment
fn fs_nd(in: VOut) -> @location(0) vec4<f32> {
    let n = normalize((g.view * vec4<f32>(normalize(in.normal), 0.0)).xyz);
    let depth = -(g.view * vec4<f32>(in.world, 1.0)).z;
    return vec4<f32>(n, depth);
}

// Солдат: нормаль «на камеру», как у RWR, — обводка не рисует линий внутри фигуры.
@fragment
fn fs_point_nd(in: POut) -> @location(0) vec4<f32> {
    let depth = -(g.view * vec4<f32>(in.world, 1.0)).z;
    return vec4<f32>(0.0, 0.0, 1.0, depth);
}

// --- Обводка, затенение, виньетка ------------------------------------------------------
// Полноэкранный треугольник; результат умножается на уже нарисованную картинку.

@group(1) @binding(1) var nd_tex: texture_2d<f32>;

@vertex
fn vs_post(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - vec2<f32>(1.0), 0.0, 1.0);
}

fn nd_at(p: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(nd_tex));
    return textureLoad(nd_tex, clamp(p, vec2<i32>(0), size - vec2<i32>(1)), 0);
}

// Точка в пространстве камеры по пикселю вьюпорта и глубине (камера смотрит вдоль −z).
fn view_pos(p: vec2<i32>, depth: f32) -> vec3<f32> {
    let half = g.screen.zw * 0.5;
    let xy = (vec2<f32>(p) + vec2<f32>(0.5) - half) / g.tint.w * depth;
    return vec3<f32>(xy.x, -xy.y, -depth);
}

@fragment
fn fs_post(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(nd_tex));
    let p = vec2<i32>(floor(frag.xy - g.screen.xy));
    let uv = (vec2<f32>(p) + vec2<f32>(0.5)) / size;

    // Виньетка: края экрана темнее.
    let aspect = vec2<f32>(1.0, size.y / max(size.x, 1.0));
    let v = 1.0 - g.post.z * smoothstep(0.35, 0.95, length((uv - vec2<f32>(0.5)) * aspect) * 1.25);

    let c = nd_at(p);
    if (c.w <= 0.0) {
        return vec4<f32>(v, v, v, 1.0);
    }

    // Обводка: соседи крестом. Край — там, где глубина скачет или нормали сильно расходятся.
    var edge = 0.0;
    let cross_offsets = array<vec2<i32>, 4>(vec2<i32>(1, 0), vec2<i32>(-1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1));
    for (var k = 0; k < 4; k++) {
        let s = nd_at(p + cross_offsets[k]);
        if (s.w <= 0.0) {
            edge = 1.0;
            continue;
        }
        let depth_jump = abs(s.w - c.w) / c.w;
        let bend = 1.0 - dot(s.xyz, c.xyz);
        edge = max(edge, smoothstep(0.02, 0.06, depth_jump));
        edge = max(edge, smoothstep(0.35, 0.8, bend) * 0.6);
    }

    // Затенение в углах: 8 выборок по кругу радиусом ~0,6 м. Затеняет то, что поднимается
    // над плоскостью поверхности (по нормали) и не слишком далеко — ровный склон сам себя
    // не затеняет.
    let radius = clamp(0.6 * g.tint.w / c.w, 2.0, 24.0);
    let center = view_pos(p, c.w);
    var occ = 0.0;
    for (var k = 0; k < 8; k++) {
        let a = f32(k) * 0.785398;
        let r = radius * select(1.0, 0.5, k % 2 == 0);
        let q = p + vec2<i32>(vec2<f32>(cos(a), sin(a)) * r);
        let s = nd_at(q);
        if (s.w > 0.0) {
            let d = view_pos(q, s.w) - center;
            let dist = length(d);
            let rise = dot(c.xyz, d) / max(dist, 1e-4);
            occ += clamp((rise - 0.15) * 2.0, 0.0, 1.0) * (1.0 - clamp(dist / 3.0, 0.0, 1.0));
        }
    }
    let ao = 1.0 - (occ / 8.0) * g.post.y * 0.9;
    let line = 1.0 - edge * g.post.x * 0.75;
    let m = line * ao * v;
    return vec4<f32>(m, m, m, 1.0);
}

// --- Вода ----------------------------------------------------------------------------
// Плоскость на уровне воды, прозрачная. Что под ней (дно, стены, солдаты) — из прохода
// нормалей и глубины: по разнице глубин узнаём, сколько воды до дна или до предмета.
// Светлая и прозрачная: мелко дно видно почти как без воды, глубже — светло-бирюзовая
// толща; у дна и у предметов — тонкая пенная кайма (как на референсе Артёма). Рябь —
// мягкая, из плавного шума, с редким бликом солнца. Без полос и пятен по воде.

struct WOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_water(v: VIn) -> WOut {
    var out: WOut;
    out.world = v.pos;
    out.clip = g.view_proj * vec4<f32>(v.pos, 1.0);
    return out;
}

// Высота ряби (условная): два слоя плавного шума, бегущих в разные стороны.
fn water_wave(p: vec2<f32>, t: f32) -> f32 {
    return perlin(p * 0.45 + vec2<f32>(t * 0.3, t * 0.2)) * 0.6 + perlin(p * 1.1 - vec2<f32>(t * 0.22, -t * 0.27)) * 0.4;
}

@fragment
fn fs_water(in: WOut) -> @location(0) vec4<f32> {
    let t = g.params.y;
    let p = in.world.xz;
    let view_dir = normalize(in.world - g.eye.xyz);

    // Сколько воды под этой точкой: по лучу — до того, что видно под водой, и по вертикали.
    let px = vec2<i32>(floor(in.clip.xy - g.screen.xy));
    let below = nd_at(px).w;
    let surface = -(g.view * vec4<f32>(in.world, 1.0)).z;
    var depth = 40.0;
    if (below > 0.0) {
        depth = max(below - surface, 0.0) * abs(view_dir.y);
    }

    // Рябь: нормаль по наклону шума — мягкая.
    let e = 0.1;
    let dx = water_wave(p + vec2<f32>(e, 0.0), t) - water_wave(p - vec2<f32>(e, 0.0), t);
    let dz = water_wave(p + vec2<f32>(0.0, e), t) - water_wave(p - vec2<f32>(0.0, e), t);
    let n = normalize(vec3<f32>(-dx * 1.3, 1.0, -dz * 1.3));

    // Цвет по глубине (sRGB): мелко — светлая бирюза, глубже — чистая голубая.
    let k = 1.0 - exp(-depth * 0.9);
    let shallow = vec3<f32>(0.50, 0.90, 0.96);
    let deep = vec3<f32>(0.18, 0.62, 0.88);
    let albedo = to_linear(mix(shallow, deep, k));
    let sun = max(dot(n, g.sun_dir.xyz), 0.0);
    var c = albedo * (g.ambient_sky.rgb * 1.2 + g.sun_color.rgb * (0.45 + 0.4 * sun));
    // Под острым углом вода чуть светлее — отражает небо.
    let fresnel = pow(1.0 - max(dot(-view_dir, n), 0.0), 4.0);
    c = mix(c, to_linear(vec3<f32>(0.85, 0.95, 1.0)) * g.ambient_sky.rgb * 1.6, fresnel * 0.3);
    // Блик солнца — узкий и редкий.
    let h = normalize(g.sun_dir.xyz - view_dir);
    let spec = pow(max(dot(n, h), 0.0), 300.0) * 0.5;
    var col = to_srgb(c) + vec3<f32>(spec);

    // Пена: тонкая кайма у дна и у предметов, край чуть гуляет.
    let wobble = perlin(p * 1.8 + vec2<f32>(t * 0.4, -t * 0.25));
    let edge = 0.04 + 0.02 * wobble;
    let foam = 1.0 - smoothstep(edge * 0.4, edge, depth);
    col = mix(col, vec3<f32>(0.93, 0.99, 1.0), foam * 0.9);

    // Прозрачность: мелко — дно видно, глубоко — толща светлая, но не сплошная.
    let alpha = clamp(max(mix(0.3, 0.82, k), foam * 0.9) + spec * 0.4, 0.0, 1.0);
    return vec4<f32>(fogged(col, in.world), alpha);
}
