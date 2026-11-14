"""Build the offline base map from Natural Earth (public domain).

    python scripts/build-map-data.py [cache-dir]

Writes, under src/assets/map/:

  world-110m.json   land, lakes and country borders for the world view (and the mini-map)
  world-50m.json    the same at 50 m, loaded once the map zooms in
  relief-shade.webp shaded relief, reprojected to Web Mercator: where slopes face away
  relief-light.webp   from the light, and where they face it, each as an alpha mask

Coordinates are projected to Web Mercator here, once, as fractions of the world scaled to
integers (Q units per world width) and delta-encoded per ring. The map then draws each
layer as one cached Path2D under a canvas transform instead of projecting every vertex
on every frame.

Needs Python 3 with Pillow and NumPy. Sources are downloaded into the cache directory on
first run (about 70 MB, mostly the relief raster).
"""

import io
import json
import math
import sys
import urllib.request
import zipfile
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src" / "assets" / "map"
VECTOR = "https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/{}.geojson"
RELIEF = "https://naciscdn.org/naturalearth/50m/raster/SR_50M.zip"

# Integer units per world width. 2^20 keeps 50 m data exact to a pixel beyond zoom 12.
Q = 1 << 20
MAX_LAT = 85.05112878
# Relief output size (square, because Web Mercator's world is square). Two RGBA masks at
# 2048 decode to 32 MB; that is sharp to zoom 3 and soft, not wrong, beyond it.
RELIEF_SIZE = 2048
# Grey that flat ground has in SR_50M. Darker than this is shadow, lighter is highlight.
# The two are shipped as separate alpha masks so the map can tint each with a theme token:
# a single grey layer blended onto the land reads on dark themes or on light ones, never
# both, and would bake a colour into an asset.
RELIEF_FLAT = 206
SHADE_GAIN = 1.6
LIGHT_GAIN = 4.0
# Deviations smaller than this are noise on flat ground, not terrain; shading them makes
# plains look speckled.
DEAD_ZONE = 8


def fetch(cache: Path, url: str) -> bytes:
    path = cache / url.rsplit("/", 1)[1]
    if not path.exists():
        print(f"downloading {url}")
        with urllib.request.urlopen(url) as r:
            path.write_bytes(r.read())
    return path.read_bytes()


def mercator(lon: float, lat: float) -> tuple[int, int]:
    lat = max(-MAX_LAT, min(MAX_LAT, lat))
    x = (lon + 180) / 360
    y = 0.5 - math.log(math.tan(math.pi / 4 + math.radians(lat) / 2)) / (2 * math.pi)
    return round(x * Q), round(y * Q)


def encode(coords: list) -> list[int]:
    """One ring or line as [x0, y0, dx1, dy1, …], dropping points that do not move."""
    out: list[int] = []
    px = py = None
    for lon, lat, *_ in coords:
        x, y = mercator(lon, lat)
        if px is None:
            out += [x, y]
        elif (x, y) != (px, py):
            out += [x - px, y - py]
        else:
            continue
        px, py = x, y
    return out


def rings(geojson: bytes, lines: bool = False, holes: list | None = None) -> list[list[int]]:
    """Every ring or line. With `holes`, polygon holes go there instead of into the result,
    so each returned ring can be filled on its own (MapLibre fills features one by one)."""
    out = []
    for f in json.loads(geojson)["features"]:
        g = f["geometry"]
        if g is None:
            continue
        t, c = g["type"], g["coordinates"]
        polys = [c] if t == "Polygon" else c if t == "MultiPolygon" else None
        if polys is not None and holes is not None:
            for poly in polys:
                holes.extend(e for e in (encode(h) for h in poly[1:]) if len(e) >= 6)
            polys = [poly[:1] for poly in polys]
        if polys is not None:
            parts = [r for p in polys for r in p]
        elif t == "LineString":
            parts = [c]
        elif t == "MultiLineString":
            parts = c
        else:
            continue
        for p in parts:
            e = encode(p)
            if len(e) >= (4 if lines else 6):
                out.append(e)
    return out


def layer_set(cache: Path, scale: str) -> dict:
    # The land's holes are water — at 50 m, exactly one: the Caspian Sea, which the lakes
    # dataset does not carry. They join the lakes so every land ring is a plain outline.
    land_holes: list = []
    land = rings(fetch(cache, VECTOR.format(f"ne_{scale}_land")), holes=land_holes)
    return {
        "q": Q,
        "land": land,
        "lakes": rings(fetch(cache, VECTOR.format(f"ne_{scale}_lakes"))) + land_holes,
        "borders": rings(fetch(cache, VECTOR.format(f"ne_{scale}_admin_0_boundary_lines_land")), lines=True),
    }


def relief(cache: Path) -> tuple[Image.Image, Image.Image]:
    Image.MAX_IMAGE_PIXELS = None
    z = zipfile.ZipFile(io.BytesIO(fetch(cache, RELIEF)))
    name = next(n for n in z.namelist() if n.lower().endswith(".tif"))
    src = np.asarray(Image.open(io.BytesIO(z.read(name))).convert("L"), dtype=np.float32)
    h = src.shape[0]
    # Equirectangular to Web Mercator: each output row samples the source row at its latitude.
    ys = (np.arange(RELIEF_SIZE) + 0.5) / RELIEF_SIZE
    lat = np.degrees(np.arctan(np.sinh(np.pi * (1 - 2 * ys))))
    rows = np.clip(((90 - lat) / 180 * h).astype(int), 0, h - 1)
    # Box-average the columns first so the downsample does not alias ridgelines.
    col = Image.fromarray(src.astype(np.uint8)).resize((RELIEF_SIZE, h), Image.Resampling.BOX)
    merc = np.asarray(col, dtype=np.float32)[rows]
    shade = np.clip((RELIEF_FLAT - DEAD_ZONE - merc) * SHADE_GAIN, 0, 255).astype(np.uint8)
    light = np.clip((merc - RELIEF_FLAT - DEAD_ZONE / 2) * LIGHT_GAIN, 0, 255).astype(np.uint8)
    return mask(shade), mask(light)


def mask(alpha: np.ndarray) -> Image.Image:
    """An alpha-only image: the colour comes from the theme when it is drawn."""
    rgba = np.zeros((*alpha.shape, 4), dtype=np.uint8)
    rgba[..., 3] = alpha
    return Image.fromarray(rgba, "RGBA")


def main() -> None:
    cache = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / ".map-cache"
    cache.mkdir(parents=True, exist_ok=True)
    OUT.mkdir(parents=True, exist_ok=True)
    for scale in ("110m", "50m"):
        data = layer_set(cache, scale)
        path = OUT / f"world-{scale}.json"
        path.write_text(json.dumps(data, separators=(",", ":")))
        pts = sum(len(r) for k in ("land", "lakes", "borders") for r in data[k]) // 2
        print(f"{path.name}: {pts:,} points, {path.stat().st_size / 1024:.0f} KB")
    for name, img in zip(("shade", "light"), relief(cache)):
        path = OUT / f"relief-{name}.webp"
        # Lossless: lossy alpha quantises in 8 px blocks, which shows as a grid once the
        # map is zoomed in far enough to upscale the mask.
        img.save(path, "WEBP", lossless=True, quality=100, method=6)
        print(f"{path.name}: {img.size}, {path.stat().st_size / 1024:.0f} KB")


if __name__ == "__main__":
    main()
