# Остальная деревня: сарай, забор, колодец, берёза, ель. Ставятся в ряд справа от избы.
import math, random
from mathutils import Matrix


def saray():
    b = Builder("сарай")
    W, D, H = 5.0, 4.0, 2.6
    b.box((0, 0, 0.1), (W + 0.2, D + 0.2, 0.2), "stone")
    # Стены из вертикальных досок разного оттенка.
    rnd = random.Random(3)
    for side, (ax, n, fixed) in {"front": ("x", 20, -D / 2), "back": ("x", 20, D / 2), "left": ("y", 16, -W / 2), "right": ("y", 16, W / 2)}.items():
        length = W if ax == "x" else D
        for k in range(n):
            t = -length / 2 + (k + 0.5) * length / n
            color = rnd.choice(["plank_dark", "plank", "plank_dark"])
            h = H + (0.0 if ax == "x" else 0.0)
            if side == "front" and abs(t) < 1.0:
                continue  # проём под ворота
            pos = (t, fixed, 0.2 + h / 2) if ax == "x" else (fixed, t, 0.2 + h / 2)
            size = (length / n - 0.02, 0.08, h) if ax == "x" else (0.08, length / n - 0.02, h)
            b.box(pos, size, color)
    # Ворота, одна створка приоткрыта.
    b.box((-0.5, -D / 2 - 0.02, 0.2 + 1.1), (1.0, 0.07, 2.2), "plank_grey")
    door = Matrix.Translation((1.0, -D / 2, 0.2 + 1.1)) @ Matrix.Rotation(-0.5, 4, "Z")
    b.box((-0.5, -0.04, 0), (1.0, 0.07, 2.2), "plank_grey", matrix=door)
    # Обвязка.
    b.box((0, 0, 0.2 + H), (W + 0.2, D + 0.2, 0.12), "plank_dark")
    # Односкатная крыша из ржавого железа: выше спереди.
    slope = math.atan2(0.7, D + 0.8)
    roof = Matrix.Translation((0, 0, 0.2 + H + 0.45)) @ Matrix.Rotation(-slope, 4, "X")
    b.box((0, 0, 0), (W + 0.7, D + 1.0, 0.05), "rust", matrix=roof)
    for k in range(12):
        x = -W / 2 - 0.3 + (k + 0.5) * (W + 0.6) / 12
        b.box((x, 0, 0.04), (0.07, D + 1.0, 0.035), "rust_dark", matrix=roof)
    # Фронтоны-треугольники под односкатной крышей.
    for x in (-W / 2, W / 2):
        m = Matrix(((0, 0, 1, x), (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 0, 1)))
        b.prism([(-D / 2, 0.2 + H), (D / 2, 0.2 + H), (-D / 2, 0.2 + H + 0.75)], -0.04, 0.04, "plank_dark", matrix=m)
    return b.finish(location=(10, 0, 0))


def zabor():
    b = Builder("забор")
    L = 3.0
    rnd = random.Random(7)
    for x in (-L / 2, L / 2):
        b.box((x, 0, 0.75), (0.12, 0.12, 1.5), "plank_dark")
    for z in (0.4, 1.1):
        b.box((0, 0.08, z), (L, 0.05, 0.09), "plank_grey")
    n = 13
    for k in range(n):
        if k == 9:
            continue  # одной доски нет
        x = -L / 2 + (k + 0.5) * L / n
        tilt = 0.18 if k == 5 else rnd.uniform(-0.03, 0.03)
        h = 1.35 + rnd.uniform(-0.08, 0.06)
        m = Matrix.Translation((x, 0.12, 0.08)) @ Matrix.Rotation(tilt, 4, "Y")
        b.box((0, 0, h / 2), (0.1, 0.03, h), rnd.choice(["plank_grey", "plank", "plank_grey"]), matrix=m)
        b.box((0, 0, h + 0.04), (0.07, 0.03, 0.08), "plank_grey", matrix=m)
    return b.finish(location=(16, 0, 0))


def kolodets():
    b = Builder("колодец")
    # Сруб колодца.
    for i in range(5):
        z = 0.12 + i * 0.2
        for y in (-0.55, 0.55):
            b.cylinder((-0.72, y, z), (0.72, y, z), 0.1, "log" if i % 2 else "log_dark", sides=6)
        for x in (-0.55, 0.55):
            b.cylinder((x, -0.72, z + 0.1), (x, 0.72, z + 0.1), 0.1, "log" if i % 2 else "log_dark", sides=6)
    b.box((0, 0, 0.9), (1.0, 1.0, 0.04), "water")
    # Стойки, ворот с цепью, ведро.
    for x in (-0.6, 0.6):
        b.box((x, 0, 1.4), (0.12, 0.12, 1.8), "plank_dark")
    b.cylinder((-0.72, 0, 1.75), (0.72, 0, 1.75), 0.09, "log_end", sides=6)
    b.box((0.85, 0, 1.6), (0.05, 0.05, 0.35), "metal")
    b.box((0.85, 0.15, 1.45), (0.05, 0.3, 0.05), "metal")
    b.cylinder((0.0, 0.0, 1.7), (0.0, 0.0, 1.3), 0.015, "metal", sides=4)
    b.cylinder((0.0, 0.0, 1.05), (0.0, 0.0, 1.3), 0.14, "metal", sides=7, radius_b=0.17)
    # Двускатная крышка.
    for side in (-1, 1):
        m = Matrix.Translation((0, side * 0.42, 2.45)) @ Matrix.Rotation(side * -0.65, 4, "X")
        b.box((0, 0, 0), (1.6, 1.0, 0.05), "plank_grey", matrix=m)
    b.box((0, 0, 2.75), (1.65, 0.12, 0.1), "plank_dark")
    return b.finish(location=(20, 0, 0))


def bereza():
    b = Builder("берёза")
    rnd = random.Random(11)
    # Ствол — сужается, с чёрными отметинами.
    pts = [(0, 0, 0), (0.08, 0.05, 2.5), (0.0, 0.12, 5.0), (-0.1, 0.1, 7.5)]
    radii = [0.2, 0.16, 0.12, 0.07]
    for i in range(3):
        b.cylinder(pts[i], pts[i + 1], radii[i], "birch", sides=7, radius_b=radii[i + 1])
    for k in range(14):
        z = 0.4 + k * 0.45 + rnd.uniform(-0.1, 0.1)
        t = z / 7.5
        r = 0.2 - 0.13 * t
        a = rnd.uniform(0, math.tau)
        m = Matrix.Translation((math.cos(a) * r * 0.85 + 0.05 * t, math.sin(a) * r * 0.85 + 0.1 * t, z)) @ Matrix.Rotation(a, 4, "Z")
        b.box((0, 0, 0), (0.06, r * 1.1, 0.06), "birch_mark", matrix=m)
    # Ветви.
    for (a, z, l) in ((0.3, 3.8, 1.5), (2.4, 4.6, 1.4), (4.2, 5.3, 1.2), (1.4, 6.0, 1.0)):
        end = (math.cos(a) * l, math.sin(a) * l + 0.1, z + 0.9)
        b.cylinder((0.02, 0.08, z), end, 0.05, "birch", sides=5, radius_b=0.025)
    # Листва — несколько комьев.
    for (x, y, z, r, c) in ((0.0, 0.1, 6.6, 1.5, "leaf"), (1.0, 0.3, 5.4, 1.15, "leaf_light"), (-0.9, -0.2, 5.6, 1.2, "leaf"),
                            (0.3, 1.0, 5.0, 1.0, "leaf_light"), (-0.3, -0.8, 4.7, 0.95, "leaf"), (0.2, 0.0, 7.6, 0.9, "leaf_light")):
        b.blob((x, y, z), r, c, squash=1.15, subdiv=1, seed=int(z * 10))
    return b.finish(location=(24, 0, 0))


def el():
    b = Builder("ель")
    b.cylinder((0, 0, 0), (0, 0, 1.4), 0.22, "bark", sides=6, radius_b=0.18)
    tiers = [(0.8, 2.6, 2.3), (2.0, 2.4, 1.9), (3.2, 2.2, 1.55), (4.3, 2.0, 1.2), (5.3, 1.8, 0.85), (6.2, 1.8, 0.55)]
    for i, (z, h, r) in enumerate(tiers):
        b.cone((0, 0, z), h, r, "spruce" if i % 2 == 0 else "spruce_light", sides=8, rot=i * 0.4)
    return b.finish(location=(30, 0, 0))


made = [saray(), zabor(), kolodets(), bereza(), el()]
for o in made:
    print(o.name, "вершин", len(o.data.vertices), "размер", tuple(round(d, 2) for d in o.dimensions))
