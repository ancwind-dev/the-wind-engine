"""Выгрузка травы GrassV1 из `grass_lab.blend` в данные игры.

Хозяин рисует траву в Blender (`analyze/blender/grass_lab.blend`, сцена `GrassV1_Pads`,
генератор `analyze/blender/grass_v1.py`). Игре нужны шесть кусков без материала (только
положения и UV) и атлас:

    data/models/grass/{low,mid,tall,khaki,weeds,clump}.glb
    data/models/grass/atlas.png

Запуск — в Blender с открытым `grass_lab.blend` (вкладка Scripting или через MCP):

    exec(open(r"C:/Users/anchorwind/Documents/projects/the wind engine/tools/blender/export_grass_v1.py", encoding="utf-8").read())

Атлас берётся из картинки материала кусков (`EngineLit_GrassV1`). Перезапусти игру — трава
обновится (на лету пока не перечитывается).
"""

import os
import shutil

import bpy

ENGINE = r"C:/Users/anchorwind/Documents/projects/the wind engine"
OUT = os.path.join(ENGINE, "data", "models", "grass")
KINDS = ["low", "mid", "tall", "khaki", "weeds", "clump"]
SCENE = "GrassV1_Pads"


def export():
    os.makedirs(OUT, exist_ok=True)
    win = bpy.context.window
    prev = win.scene
    scene = bpy.data.scenes[SCENE]
    win.scene = scene
    done = []
    try:
        for kind in KINDS:
            me = bpy.data.meshes["grass_" + kind]
            ob = bpy.data.objects.new("export_" + kind, me)
            scene.collection.objects.link(ob)
            bpy.ops.object.select_all(action="DESELECT")
            ob.select_set(True)
            bpy.context.view_layer.objects.active = ob
            path = os.path.join(OUT, kind + ".glb")
            bpy.ops.export_scene.gltf(filepath=path, export_format="GLB", use_selection=True, use_active_scene=True,
                                      export_normals=True, export_texcoords=True, export_materials="NONE", export_yup=True)
            bpy.data.objects.remove(ob, do_unlink=True)
            done.append(path)
        # Атлас — картинка из материала кусков.
        mat = bpy.data.meshes["grass_low"].materials[0]
        img = next(n.image for n in mat.node_tree.nodes if n.type == "TEX_IMAGE" and n.image)
        src = bpy.path.abspath(img.filepath)
        shutil.copyfile(src, os.path.join(OUT, "atlas.png"))
        done.append(src + " -> atlas.png")
    finally:
        bpy.ops.object.select_all(action="DESELECT")
        win.scene = prev
    return done


print(export())
