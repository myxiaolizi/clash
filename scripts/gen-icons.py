"""生成应用图标（盾牌 + 加速箭头，深蓝渐变）。

Tauri 需要这些尺寸：
  icon.ico (多尺寸), 32x32.png, 128x128.png, 128x128@2x.png, icon.png
"""
import os
import math
from PIL import Image, ImageDraw

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "icons")
SIZE = 1024


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def make_icon(size=SIZE):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # ---- 圆角方形背景，左上→右下渐变 ----
    top = (30, 60, 140)
    bottom = (12, 26, 70)
    grad = Image.new("RGB", (1, size))
    gd = ImageDraw.Draw(grad)
    for y in range(size):
        gd.point((0, y), fill=lerp(top, bottom, y / (size - 1)))
    grad = grad.resize((size, size))

    mask = Image.new("L", (size, size), 0)
    md = ImageDraw.Draw(mask)
    r = int(size * 0.22)
    md.rounded_rectangle([0, 0, size - 1, size - 1], radius=r, fill=255)
    img.paste(grad, (0, 0), mask)

    # ---- 盾牌轮廓 ----
    cx = size / 2
    top_y = size * 0.20
    bot_y = size * 0.82
    half_w = size * 0.27

    shield = [
        (cx, top_y),
        (cx + half_w, top_y + size * 0.10),
        (cx + half_w * 0.92, top_y + size * 0.34),
        (cx, bot_y),
        (cx - half_w * 0.92, top_y + size * 0.34),
        (cx - half_w, top_y + size * 0.10),
    ]
    d.polygon(shield, fill=(255, 255, 255, 240))

    # 盾牌内圈（挖空成描边效果）
    inset = size * 0.035
    shield_in = [
        (cx, top_y + inset * 1.6),
        (cx + half_w - inset, top_y + size * 0.105),
        (cx + (half_w - inset) * 0.92, top_y + size * 0.34),
        (cx, bot_y - inset * 1.3),
        (cx - (half_w - inset) * 0.92, top_y + size * 0.34),
        (cx - half_w + inset, top_y + size * 0.105),
    ]
    d.polygon(shield_in, fill=(30, 60, 140, 255))

    # ---- 加速箭头（两个叠放的 chevron）----
    accent = (90, 200, 255, 255)
    for i, yoff in enumerate([0.0, 0.115]):
        w = size * (0.115 - i * 0.012)
        thick = size * 0.052
        apex_y = size * (0.40 + yoff)
        pts = [
            (cx - w, apex_y + thick * 1.5),
            (cx, apex_y),
            (cx + w, apex_y + thick * 1.5),
            (cx + w, apex_y + thick * 2.3),
            (cx, apex_y + thick * 0.8),
            (cx - w, apex_y + thick * 2.3),
        ]
        d.polygon(pts, fill=accent)

    return img


def main():
    os.makedirs(OUT, exist_ok=True)
    base = make_icon(SIZE)

    # 标准尺寸
    sizes = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }
    for name, px in sizes.items():
        base.resize((px, px), Image.LANCZOS).save(os.path.join(OUT, name), "PNG")
        print(f"  {name}  {px}x{px}")

    # .ico 多尺寸
    ico_sizes = [16, 24, 32, 48, 64, 128, 256]
    base.save(
        os.path.join(OUT, "icon.ico"),
        format="ICO",
        sizes=[(s, s) for s in ico_sizes],
    )
    print(f"  icon.ico  {ico_sizes}")

    # .icns（本机不打包 macOS，但 Tauri 校验清单会查；生成一个最小可用版本）
    try:
        base.resize((512, 512), Image.LANCZOS).save(os.path.join(OUT, "icon.icns"), format="ICNS")
        print("  icon.icns 512x512")
    except Exception as e:
        print(f"  icon.icns 跳过：{e}")

    print("\n图标生成完成:", OUT)


if __name__ == "__main__":
    main()
