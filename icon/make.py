# Значки движка из логотипа: `python icon/make.py` из корня проекта.
#
# Логотип красим в белый. Белый не виден на светлой панели задач и в светлом
# проводнике, поэтому есть второй вариант — на чёрной скруглённой подложке.
#   icon/icon-dark.png  — белый на прозрачном: окно при тёмной теме Windows;
#   icon/icon-light.png — белый на чёрной подложке: окно при светлой теме;
#   icon/wind.ico       — значок файла .exe. У файла значок один на обе темы,
#                         поэтому — с подложкой: он виден и на светлом, и на тёмном.
from PIL import Image, ImageDraw

SIZE = 256
logo = Image.open("logo.png").convert("RGBA")
box = logo.getchannel("A").point(lambda v: 255 if v > 8 else 0).getbbox()
alpha = logo.getchannel("A").crop(box)
white = Image.new("RGBA", alpha.size, (255, 255, 255, 0))
white.putalpha(alpha)


def fit(art, side, margin):
    """Рисунок в квадрат side × side с полями margin (доля стороны) с каждого края."""
    inner = round(side * (1 - 2 * margin))
    w, h = art.size
    k = inner / max(w, h)
    art = art.resize((max(1, round(w * k)), max(1, round(h * k))), Image.LANCZOS)
    out = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    out.paste(art, ((side - art.width) // 2, (side - art.height) // 2), art)
    return out


def backed(side):
    """Белый логотип на чёрном скруглённом квадрате; рисуем вчетверо крупнее — края глаже."""
    big = side * 4
    plate = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    ImageDraw.Draw(plate).rounded_rectangle((0, 0, big - 1, big - 1), radius=round(big * 0.22), fill=(0, 0, 0, 255))
    plate.alpha_composite(fit(white, big, 0.17))
    return plate.resize((side, side), Image.LANCZOS)


fit(white, SIZE, 0.04).save("icon/icon-dark.png", optimize=True)
backed(SIZE).save("icon/icon-light.png", optimize=True)
# В .ico каждый размер рисуем отдельно: так мелкие значки резче, чем ужатый большой.
sizes = [16, 24, 32, 48, 64, 128, 256]
frames = [backed(s) for s in sizes]
frames[-1].save("icon/wind.ico", sizes=[(s, s) for s in sizes], append_images=frames[:-1])
