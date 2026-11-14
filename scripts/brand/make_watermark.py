"""Draws the mark Lumen lays over the videos it makes (Round 8A, Task 4): the logo and the
word *Lumen* in Inter, white with a soft shadow so it reads on light and dark frames.

    python scripts/brand/make_watermark.py

Writes src-tauri/resources/watermark.png (RGBA, 256 px tall), which the app embeds. FFmpeg
scales it to the film's size and sets its opacity, so it is drawn opaque here.
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = Path(__file__).resolve().parents[2]
LOGO = ROOT / "src/assets/brand/lumen-512.png"
FONT = ROOT / "node_modules/@fontsource-variable/inter/files/inter-latin-wght-normal.woff2"
OUT = ROOT / "src-tauri/resources/watermark.png"
H = 256

logo = Image.open(LOGO).convert("RGBA")
logo = logo.crop(logo.getchannel("A").getbbox())
logo = logo.resize((round(logo.width * H / logo.height), H), Image.LANCZOS)

font = ImageFont.truetype(str(FONT), round(H * 0.5))
try:
    font.set_variation_by_axes([600])
except OSError:
    pass
gap = round(H * 0.16)
left, top, right, bottom = font.getbbox("Lumen")
text_w = right - left
pad = round(H * 0.08)
W = logo.width + gap + text_w + pad
canvas = Image.new("RGBA", (W, H + pad), (0, 0, 0, 0))

text = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
# Cap height centred on the logo.
y = (H - (bottom - top)) // 2 - top
ImageDraw.Draw(text).text((logo.width + gap - left, y), "Lumen", font=font, fill=(255, 255, 255, 255))
shadow = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
shadow.putalpha(text.getchannel("A").point(lambda a: a * 0.55))
shadow = shadow.filter(ImageFilter.GaussianBlur(H * 0.035))
canvas.alpha_composite(shadow, (0, round(H * 0.02)))
canvas.alpha_composite(text)
canvas.alpha_composite(logo, (0, 0))
canvas = canvas.crop(canvas.getchannel("A").getbbox())
OUT.parent.mkdir(parents=True, exist_ok=True)
canvas.save(OUT, optimize=True)
print(OUT, canvas.size)
