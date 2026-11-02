# Собирает все модели деревни и выгружает их в data/models/props/*.glb.
#
# В Blender: вкладка Scripting → Open → этот файл → Run Script. Модели появятся
# в коллекции «Wind Props» (в ряд вдоль X), а .glb — в data/models/props: игра и
# редактор подхватят их на лету. Поправить модель — правь её скрипт (izba.py,
# village.py) или саму модель в Blender, затем снова Run (или выгрузи вручную:
# File → Export → glTF 2.0, формат .glb, «Selected Objects», +Y Up).
import os
import bpy

def here():
    """Папка этого скрипта — и когда он открыт во вкладке Scripting, и когда запущен иначе."""
    space = getattr(bpy.context, "space_data", None)
    text = getattr(space, "text", None)
    if text is not None and text.filepath:
        return os.path.dirname(bpy.path.abspath(text.filepath))
    return os.path.dirname(os.path.abspath(__file__))


HERE = here()
PROPS = os.path.normpath(os.path.join(HERE, "..", "..", "data", "models", "props"))

g = {}
for script in ("we.py", "izba.py", "village.py"):
    with open(os.path.join(HERE, script), encoding="utf-8") as f:
        exec(f.read(), g)

os.makedirs(PROPS, exist_ok=True)
for name in ("изба", "сарай", "забор", "колодец", "берёза", "ель"):
    g["export"](bpy.data.objects[name], os.path.join(PROPS, name + ".glb"))
print("выгружено в", PROPS)
