# Изба: сруб из брёвен с выпусками на углах, шиферная двускатная крыша,
# крашеные фронтоны, окна с белыми наличниками и голубыми ставнями, крыльцо, печная труба.
import math
from mathutils import Matrix

W, D = 6.0, 7.0          # сруб по осям брёвен: x — ширина фасада, y — глубина
BASE = 0.45              # высота фундамента
R = 0.13                 # радиус бревна
STEP = 0.24              # шаг венцов
ROWS = 11                # венцов
OUT = 0.28               # выпуск брёвен за угол
TOP = BASE + ROWS * STEP # верх сруба
RIDGE = TOP + 1.55       # конёк
EAVE = 0.55              # свес крыши

b = Builder("изба")
hx, hy = W / 2, D / 2

# Фундамент.
b.box((0, 0, BASE / 2), (W + 0.3, D + 0.3, BASE), "stone")

# Сруб: фасадные и задние брёвна (вдоль x) и боковые (вдоль y) — через полвенца.
for i in range(ROWS):
    z = BASE + R + i * STEP
    color = "log" if i % 2 == 0 else "log_dark"
    for y in (-hy, hy):
        b.cylinder((-hx - OUT, y, z), (hx + OUT, y, z), R, color, sides=6)
    zs = z + STEP / 2
    if zs + R < TOP + 0.05:
        for x in (-hx, hx):
            b.cylinder((x, -hy - OUT, zs), (x, hy + OUT, zs), R, color, sides=6)

# Перекрытие под крышей — чтобы сверху не было видно щелей между брёвнами.
b.box((0, 0, TOP - 0.05), (W + 0.1, D + 0.1, 0.1), "log_dark")

# Фронтоны (треугольники над торцами по x = ±hx), доски крашены.
def gable(x0, sign):
    m = Matrix(((0, 0, 1, x0), (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 0, 1)))
    pts = [(-hy - 0.05, TOP), (hy + 0.05, TOP), (0.0, RIDGE - 0.05)]
    if sign < 0:
        pts = list(reversed(pts))
    b.prism(pts, -0.06, 0.06, "gable", matrix=m)
    # Окошко на фронтоне.
    b.box((x0 + sign * 0.08, 0, TOP + 0.55), (0.04, 0.55, 0.5), "trim")
    b.box((x0 + sign * 0.10, 0, TOP + 0.55), (0.04, 0.4, 0.35), "glass")
gable(hx + 0.02, 1)
gable(-hx - 0.02, -1)

# Крыша: два ската из шифера с волнами и конёк.
slope = math.atan2(RIDGE - TOP, hy + EAVE)
length = math.hypot(RIDGE - TOP, hy + EAVE) + 0.15
for side in (-1, 1):
    # Скат вдоль x; поворот вокруг x, середина ската.
    cy = side * (hy + EAVE) / 2
    cz = (TOP + RIDGE) / 2 + 0.06
    rot = Matrix.Translation((0, cy, cz)) @ Matrix.Rotation(-side * slope, 4, "X")
    b.box((0, 0, 0), (W + 2 * OUT + 0.6, length, 0.07), "slate", matrix=rot)
    # Волны шифера — полосы вниз по скату.
    n = 16
    for k in range(n):
        x = -hx - OUT - 0.25 + (k + 0.5) * (W + 2 * OUT + 0.5) / n
        b.box((x, 0, 0.055), (0.09, length, 0.04), "slate_dark", matrix=rot)
b.box((0, 0, RIDGE + 0.1), (W + 2 * OUT + 0.65, 0.3, 0.12), "slate_dark")

# Печная труба.
b.box((1.1, 0.7, (TOP + RIDGE) / 2 + 0.7), (0.55, 0.55, RIDGE - TOP + 1.0), "brick")
b.box((1.1, 0.7, RIDGE + 0.62), (0.68, 0.68, 0.1), "concrete")

# Окна с наличниками и ставнями: на фасаде три, по бокам по два.
def window(center, normal):
    nx, ny = normal
    x, y = center
    z = BASE + 1.45
    face = Matrix.Translation((x, y, z)) @ Matrix.Rotation(math.atan2(ny, nx) + math.pi / 2, 4, "Z")
    # Местные оси: x — вдоль стены, y — наружу (−y), z — вверх.
    out = -1.0
    b.box((0, out * 0.15, 0), (0.75, 0.04, 1.0), "glass", matrix=face)
    for (cx, cz, sx, sz) in ((0, 0.56, 0.95, 0.12), (0, -0.56, 0.95, 0.1), (-0.42, 0, 0.1, 1.1), (0.42, 0, 0.1, 1.1)):
        b.box((cx, out * 0.18, cz), (sx, 0.06, sz), "trim", matrix=face)
    # Резной верх наличника — «кокошник».
    b.box((0, out * 0.19, 0.72), (0.7, 0.05, 0.2), "trim", matrix=face)
    b.box((0, out * 0.19, 0.86), (0.35, 0.05, 0.12), "trim", matrix=face)
    # Ставни, распахнутые.
    for s in (-1, 1):
        b.box((s * 0.68, out * 0.17, 0), (0.36, 0.05, 1.0), "shutter", matrix=face)

for x in (-1.9, 0.0, 1.9):
    window((x, -hy - R), (0, -1))
for y in (-1.6, 1.6):
    window((hx + R, y), (1, 0))
    window((-hx - R, y), (-1, 0))

# Крыльцо с навесом у задней стены сбоку (вход со двора).
px, py = -1.6, hy + R
b.box((px, py + 0.75, BASE + 0.05), (1.6, 1.4, 0.1), "plank")
for k in range(3):
    b.box((px, py + 1.6 + k * 0.3, BASE - 0.15 - k * 0.15), (1.4, 0.3, 0.15), "plank_grey")
for sx in (-0.7, 0.7):
    b.box((px + sx, py + 1.35, BASE + 1.15), (0.12, 0.12, 2.2), "plank_dark")
canopy = Matrix.Translation((px, py + 0.75, BASE + 2.35)) @ Matrix.Rotation(-0.25, 4, "X")
b.box((0, 0, 0), (1.9, 1.7, 0.06), "rust", matrix=canopy)
# Дверь.
b.box((px, hy + R + 0.02, BASE + 1.0), (0.9, 0.06, 1.9), "plank_dark")

izba = b.finish(location=(0, 0, 0))
print("изба: вершин", len(izba.data.vertices), "граней", len(izba.data.polygons))
