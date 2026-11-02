# Помощники для low-poly моделей The Wind Engine: плоские цвета из палитры,
# всё в метрах, основание модели — в нуле, плоское затенение.
import bpy, bmesh, math
from mathutils import Vector, Matrix

COLLECTION = "Wind Props"

# Палитра (sRGB 0..1) — общая для всех моделей, чтобы стиль был единым.
PALETTE = {
    "log": (0.43, 0.31, 0.21),
    "log_end": (0.62, 0.48, 0.33),
    "log_dark": (0.33, 0.24, 0.17),
    "plank": (0.47, 0.40, 0.32),
    "plank_grey": (0.52, 0.49, 0.44),
    "plank_dark": (0.31, 0.25, 0.19),
    "slate": (0.56, 0.58, 0.56),
    "slate_dark": (0.44, 0.46, 0.45),
    "rust": (0.50, 0.30, 0.19),
    "rust_dark": (0.38, 0.22, 0.15),
    "trim": (0.88, 0.87, 0.82),
    "shutter": (0.36, 0.55, 0.70),
    "gable": (0.42, 0.55, 0.47),
    "glass": (0.11, 0.14, 0.17),
    "stone": (0.47, 0.46, 0.43),
    "concrete": (0.58, 0.57, 0.54),
    "brick": (0.55, 0.27, 0.19),
    "metal": (0.30, 0.31, 0.32),
    "birch": (0.86, 0.85, 0.80),
    "birch_mark": (0.16, 0.15, 0.14),
    # Осень: берёзы золотые.
    "leaf": (0.76, 0.56, 0.19),
    "leaf_light": (0.84, 0.68, 0.29),
    "spruce": (0.16, 0.30, 0.19),
    "spruce_light": (0.22, 0.37, 0.22),
    "bark": (0.30, 0.22, 0.16),
    "water": (0.12, 0.20, 0.24),
}


def to_linear(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def material(name):
    key = "WE_" + name
    mat = bpy.data.materials.get(key)
    if mat is None:
        mat = bpy.data.materials.new(key)
        mat.use_nodes = True
    r, g, b = PALETTE[name]
    lin = (to_linear(r), to_linear(g), to_linear(b), 1.0)
    bsdf = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    bsdf.inputs[0].default_value = lin
    mat.diffuse_color = lin
    return mat


def collection():
    col = bpy.data.collections.get(COLLECTION)
    if col is None:
        col = bpy.data.collections.new(COLLECTION)
        bpy.context.scene.collection.children.link(col)
    return col


class Builder:
    """Собирает одну модель из кусков в одном bmesh, с материалами по палитре."""

    def __init__(self, name):
        self.name = name
        self.bm = bmesh.new()
        self.mats = []

    def mat_index(self, color):
        if color not in self.mats:
            self.mats.append(color)
        return self.mats.index(color)

    def _add(self, verts, faces, color, matrix=None):
        idx = self.mat_index(color)
        vs = []
        for v in verts:
            p = Vector(v)
            if matrix is not None:
                p = matrix @ p
            vs.append(self.bm.verts.new(p))
        for f in faces:
            face = self.bm.faces.new([vs[i] for i in f])
            face.material_index = idx
            face.smooth = False

    def box(self, center, size, color, rot_z=0.0, matrix=None):
        cx, cy, cz = center
        sx, sy, sz = (s / 2 for s in size)
        verts = [(-sx, -sy, -sz), (sx, -sy, -sz), (sx, sy, -sz), (-sx, sy, -sz),
                 (-sx, -sy, sz), (sx, -sy, sz), (sx, sy, sz), (-sx, sy, sz)]
        faces = [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
        m = Matrix.Translation((cx, cy, cz)) @ Matrix.Rotation(rot_z, 4, "Z")
        if matrix is not None:
            m = matrix @ m
        self._add(verts, faces, color, m)

    def prism(self, points, z0, z1, color, matrix=None):
        """Многоугольник `points` (x, y) против часовой, вытянутый от z0 до z1."""
        n = len(points)
        verts = [(x, y, z0) for x, y in points] + [(x, y, z1) for x, y in points]
        faces = [tuple(reversed(range(n))), tuple(range(n, 2 * n))]
        for i in range(n):
            j = (i + 1) % n
            faces.append((i, j, n + j, n + i))
        self._add(verts, faces, color, matrix)

    def cylinder(self, a, b, radius, color, sides=6, radius_b=None):
        """Цилиндр (или конус) от точки a до точки b."""
        a, b = Vector(a), Vector(b)
        axis = b - a
        length = axis.length
        rb = radius if radius_b is None else radius_b
        verts = []
        for i in range(sides):
            t = 2 * math.pi * i / sides
            verts.append((math.cos(t) * radius, math.sin(t) * radius, 0.0))
        for i in range(sides):
            t = 2 * math.pi * i / sides
            verts.append((math.cos(t) * rb, math.sin(t) * rb, length))
        faces = [tuple(reversed(range(sides))), tuple(range(sides, 2 * sides))]
        for i in range(sides):
            j = (i + 1) % sides
            faces.append((i, j, sides + j, sides + i))
        rot = Vector((0, 0, 1)).rotation_difference(axis.normalized()).to_matrix().to_4x4()
        self._add(verts, faces, color, Matrix.Translation(a) @ rot)

    def cone(self, base, height, radius, color, sides=7, rot=0.0):
        verts = [(math.cos(2 * math.pi * i / sides + rot) * radius, math.sin(2 * math.pi * i / sides + rot) * radius, 0.0) for i in range(sides)]
        verts.append((0, 0, height))
        faces = [tuple(reversed(range(sides)))] + [(i, (i + 1) % sides, sides) for i in range(sides)]
        self._add(verts, faces, color, Matrix.Translation(base))

    def blob(self, center, radius, color, squash=1.0, subdiv=1, seed=0):
        """Низкополигональный «ком» листвы: икосфера с чуть сдвинутыми вершинами."""
        tmp = bmesh.new()
        bmesh.ops.create_icosphere(tmp, subdivisions=subdiv, radius=radius)
        import random
        rnd = random.Random(seed)
        verts = []
        for v in tmp.verts:
            p = v.co * (1.0 + rnd.uniform(-0.12, 0.12))
            verts.append((p.x, p.y, p.z * squash))
        index = {v: i for i, v in enumerate(tmp.verts)}
        faces = [tuple(index[v] for v in f.verts) for f in tmp.faces]
        tmp.free()
        self._add(verts, faces, color, Matrix.Translation(center))

    def finish(self, location=(0, 0, 0)):
        """Готовая модель — один объект в коллекции; старый с тем же именем заменяется."""
        old = bpy.data.objects.get(self.name)
        if old is not None:
            bpy.data.objects.remove(old, do_unlink=True)
        # Прежние меши этой модели без владельцев — прочь, иначе новый станет «изба.001».
        for m in [m for m in bpy.data.meshes if m.users == 0 and (m.name == self.name or m.name.startswith(self.name + "."))]:
            bpy.data.meshes.remove(m)
        bmesh.ops.remove_doubles(self.bm, verts=self.bm.verts, dist=0.0001)
        mesh = bpy.data.meshes.new(self.name)
        self.bm.to_mesh(mesh)
        self.bm.free()
        for color in self.mats:
            mesh.materials.append(material(color))
        for p in mesh.polygons:
            p.use_smooth = False
        obj = bpy.data.objects.new(self.name, mesh)
        obj.location = location
        collection().objects.link(obj)
        return obj


def export(obj, path):
    """Один объект — в .glb (Y вверх, как в движке), без положения в сцене."""
    loc = obj.location.copy()
    obj.location = (0, 0, 0)
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB", use_selection=True, export_yup=True,
                              export_apply=True, export_materials="EXPORT", export_normals=True,
                              export_texcoords=False, export_animations=False)
    obj.location = loc
