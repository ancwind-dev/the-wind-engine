# Выгружает траву из Grass.blend (меши Grass_G_*, материал M_Grass_Ghibli) в
# data/models/grass/*.glb — вместе с цветами вершин Wind (COLOR_0) и GrassShade (COLOR_1),
# по которым движок качает траву ветром и красит её (см. crates/wind_render/src/grass.rs).
#
# В Blender с открытым Grass.blend: Scripting → Open → этот файл → Run Script.
# Игра подхватывает траву при запуске.
import os
import bpy


def here():
    """Папка этого скрипта — и когда он открыт во вкладке Scripting, и когда запущен иначе."""
    space = getattr(bpy.context, "space_data", None)
    text = getattr(space, "text", None)
    if text is not None and text.filepath:
        return os.path.dirname(bpy.path.abspath(text.filepath))
    return os.path.dirname(os.path.abspath(__file__))


OUT = os.path.normpath(os.path.join(here(), "..", "..", "data", "models", "grass"))
# Меш в файле → имя в игре. «трава» и «трава-низкая» — лоскуты луга; пучки — про запас.
NAMES = {
    "Grass_G_Patch": "трава",
    "Grass_G_Patch_Low": "трава-низкая",
    "Grass_G_Tuft_S": "пучок-малый",
    "Grass_G_Tuft_L": "пучок-большой",
    "Grass_G_Tuft_Flowers": "пучок-цветы",
}

os.makedirs(OUT, exist_ok=True)
selected = [o for o in bpy.context.selected_objects]
active = bpy.context.view_layer.objects.active
for src, dst in NAMES.items():
    obj = bpy.data.objects.get(src)
    if obj is None:
        print("нет меша", src)
        continue
    # Порядок цветов: COLOR_0 — активный (Wind), COLOR_1 — GrassShade.
    obj.data.color_attributes.active_color = obj.data.color_attributes["Wind"]
    location = obj.location.copy()
    obj.location = (0, 0, 0)
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.export_scene.gltf(
        filepath=os.path.join(OUT, dst + ".glb"),
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=True,
        export_materials="NONE",
        export_normals=True,
        export_texcoords=False,
        export_animations=False,
        export_vertex_color="ACTIVE",
        export_all_vertex_colors=True,
        export_active_vertex_color_when_no_material=True,
    )
    obj.location = location
bpy.ops.object.select_all(action="DESELECT")
for o in selected:
    o.select_set(True)
bpy.context.view_layer.objects.active = active
print("трава выгружена в", OUT)
