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
    let albedo = to_linear(albedo_srgb);
    let direct = max(dot(n, g.sun_dir.xyz), 0.0) * shadow(world, n);
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
    let albedo = zones(battle_traces(material(m, p), p, vec2<f32>(size)), p);
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

// `scale` — множитель размера точки, `extra` — сколько пикселей добавить (при 1080p),
// `push` — сдвиг к камере, м (минус — от камеры).
fn point_out(v: PIn, scale: f32, extra: f32, push: f32) -> POut {
    let world = skinned(v);
    let to_eye = normalize(g.eye.xyz - world);
    var clip = g.view_proj * vec4<f32>(world + to_eye * push, 1.0);
    // У RWR точка — 2,6 пикселя при 1080p и не зависит от расстояния. Вблизи мы не
    // даём ей стать меньше полутора вокселей, иначе между точками появятся щели.
    let res = g.screen.w / 1080.0;
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
    var out = point_out(v, 1.0, 0.0, 0.5);
    out.color = unpack4x8unorm(v.inst.z).rgb;
    return out;
}

@fragment
fn fs_point(in: POut) -> @location(0) vec4<f32> {
    // Свет сверху без направления: небо + доля солнца.
    let l = g.ambient_sky.rgb + g.sun_color.rgb * 0.7;
    var c = to_srgb(to_linear(in.color) * l);
    // Нижняя половина точки темнее; у оружия — сильнее.
    let shade = select(0.85, 0.6, in.weapon > 0.5);
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
