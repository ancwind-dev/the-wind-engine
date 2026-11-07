# Значки движка из логотипа: `python icon/make.py` из корня проекта.
# logo.png → icon/icon-256.png (окно и панель задач) и icon/wind.ico (файл .exe).
from PIL import Image

logo = Image.open("logo.png").convert("RGBA")
# Обрезаем по рисунку и кладём в квадрат с полями 4 %.
box = logo.getchannel("A").point(lambda v: 255 if v > 8 else 0).getbbox()
art = logo.crop(box)
w, h = art.size
side = int(max(w, h) * 1.08)
square = Image.new("RGBA", (side, side), (0, 0, 0, 0))
square.paste(art, ((side - w) // 2, (side - h) // 2))
square.resize((256, 256), Image.LANCZOS).save("icon/icon-256.png", optimize=True)
square.save("icon/wind.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
