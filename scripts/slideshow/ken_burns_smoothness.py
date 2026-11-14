"""Ken Burns smoothness in slideshow exports (Round 6): render one clip of a synthetic,
photo-like texture through FFmpeg, track the frame-to-frame motion with sub-pixel phase
correlation, and compare it with the ideal eased motion. Writes only to a temp folder.

    python scripts/slideshow/ken_burns_smoothness.py <ffmpeg> zoompan2      # before Round 6
    python scripts/slideshow/ken_burns_smoothness.py <ffmpeg> export        # what the export builds

"Second difference RMS" is the judder: ~0 for smooth motion.
"""
import subprocess, sys, time
import numpy as np
from PIL import Image

FF = sys.argv[1]
MODE = sys.argv[2]
W, H, FPS, SECS = 1920, 1080, 30, 4
FRAMES = FPS * SECS
import tempfile
HERE = tempfile.mkdtemp(prefix="lumen-kb-")

# A detailed, photo-like texture at 2x: blurred noise at several scales.
src = f"{HERE}/texture.png"
try:
    Image.open(src)
except Exception:
    rng = np.random.default_rng(1)
    img = np.zeros((H * 2, W * 2, 3), np.float32)
    for s in (4, 16, 64):
        n = rng.random((H * 2 // s + 1, W * 2 // s + 1, 3)).astype(np.float32)
        img += np.asarray(Image.fromarray((n * 255).astype(np.uint8)).resize((W * 2, H * 2), Image.BICUBIC), np.float32) / 3
    Image.fromarray(img.clip(0, 255).astype(np.uint8)).save(src)


def eased(a, b, frames):
    u = f"min(on/{frames - 1},1)"
    return f"{a:.6f}+({b - a:.6f})*({u})*({u})*(3-2*({u}))"


def filt(frm, to, mode):
    if mode.startswith("zoompan"):
        k = int(mode[-1])
        z = eased(frm[2], to[2], FRAMES)
        x = f"iw/2+({eased(frm[0], to[0], FRAMES)})*iw-iw/zoom/2"
        y = f"ih/2+({eased(frm[1], to[1], FRAMES)})*ih-ih/zoom/2"
        return f"scale={W*k}:{H*k},setsar=1,zoompan=z='{z}':x='{x}':y='{y}':d={FRAMES}:s={W}x{H}:fps={FPS},format=gray"
    if mode == "export":
        # As photos-media's clip_filter builds it.
        n = f"min(on/{FRAMES - 1},1)"
        e = f"({n})*({n})*(3-2*({n}))"
        lerp = lambda k: f"{frm[k]:.6f}+({to[k] - frm[k]:.6f})*{e}"
        z, cx, cy = lerp(2), f"(0.5+{lerp(0)})", f"(0.5+{lerp(1)})"
        x0, x1 = f"W*({cx}-0.5/({z}))", f"W*({cx}+0.5/({z}))"
        y0, y1 = f"H*({cy}-0.5/({z}))", f"H*({cy}+0.5/({z}))"
        return (f"scale={W*2}:{H*2}:force_original_aspect_ratio=increase,crop={W*2}:{H*2},setsar=1,format=yuv420p,loop=loop={FRAMES-1}:size=1:start=0,settb=1/{FPS},setpts=N,fps={FPS},"
                f"perspective=x0='{x0}':y0='{y0}':x1='{x1}':y1='{y0}':x2='{x0}':y2='{y1}':x3='{x1}':y3='{y1}':interpolation=linear:sense=source:eval=frame,scale={W}:{H}:flags=bicubic,format=gray")
    if mode.startswith("perspective"):
        # The view rectangle in the 2x source, at sub-pixel precision, stretched over the
        # whole frame (sense=source), then scaled down to the output.
        n = f"min(on/{FRAMES - 1},1)"
        e = f"({n})*({n})*(3-2*({n}))"
        lerp = lambda k: f"({frm[k]:.6f}+({to[k] - frm[k]:.6f})*{e})"
        z, cx, cy = lerp(2), f"(0.5+{lerp(0)})", f"(0.5+{lerp(1)})"
        x0 = f"W*({cx}-0.5/{z})"; x1 = f"W*({cx}+0.5/{z})"
        y0 = f"H*({cy}-0.5/{z})"; y1 = f"H*({cy}+0.5/{z})"
        return (f"scale={W*2}:{H*2},setsar=1,perspective=x0='{x0}':y0='{y0}':x1='{x1}':y1='{y0}':x2='{x0}':y2='{y1}':x3='{x1}':y3='{y1}'"
                f":interpolation={('linear' if mode.endswith('lin') else 'cubic')}:sense=source:eval=frame,scale={W}:{H}:flags=bicubic,format=gray")
    raise SystemExit(mode)


def run(frm, to, mode, label):
    out = f"{HERE}/{label}-{mode}.raw"
    f = filt(frm, to, mode)
    t = time.time()
    looped = [] if mode == "export" else ["-framerate", str(FPS), "-loop", "1"]
    cmd = [FF, "-v", "error", "-y", *looped, "-i", src, "-filter_complex", f"[0:v]{f}[v]", "-map", "[v]", "-frames:v", str(FRAMES), "-f", "rawvideo", "-pix_fmt", "gray", out]
    subprocess.run(cmd, check=True)
    took = time.time() - t
    v = np.fromfile(out, np.uint8).reshape(-1, H, W).astype(np.float32)
    return v, took


def shift(a, b):
    """Sub-pixel translation from a to b (phase correlation, parabolic peak)."""
    win = np.outer(np.hanning(a.shape[0]), np.hanning(a.shape[1]))
    A, B = np.fft.fft2(a * win), np.fft.fft2(b * win)
    r = A * np.conj(B)
    r /= np.abs(r) + 1e-9
    c = np.fft.ifft2(r).real
    py, px = np.unravel_index(np.argmax(c), c.shape)
    def sub(m, p, n):
        l, ce, rr = m[(p - 1) % n], m[p], m[(p + 1) % n]
        d = l - 2 * ce + rr
        return p + (0.5 * (l - rr) / d if d != 0 else 0)
    sy = sub(c[:, px], py, c.shape[0]); sx = sub(c[py, :], px, c.shape[1])
    if sy > c.shape[0] / 2: sy -= c.shape[0]
    if sx > c.shape[1] / 2: sx -= c.shape[1]
    return -sx, -sy


def ideal_pos(frm, to, n, px, py):
    """Where the source point seen at output pixel (px, py) on frame 0 is on frame n."""
    def view(i):
        u = min(i / (FRAMES - 1), 1); e = u * u * (3 - 2 * u)
        x, y, z = [frm[k] + (to[k] - frm[k]) * e for k in range(3)]
        return x, y, z
    x0, y0, z0 = view(0)
    # source coords (0..1 of the 2x image) of output pixel at frame 0
    sx = 0.5 + x0 + (px / W - 0.5) / z0
    sy = 0.5 + y0 + (py / H - 0.5) / z0
    x, y, z = view(n)
    return (W * (0.5 + (sx - 0.5 - x) * z), H * (0.5 + (sy - 0.5 - y) * z))


def analyse(v, frm, to, label, patch):
    x0, y0 = patch
    S = 256
    meas = []
    for n in range(1, len(v)):
        a = v[n - 1, y0:y0 + S, x0:x0 + S]; b = v[n, y0:y0 + S, x0:x0 + S]
        meas.append(shift(a, b))
    meas = np.array(meas)
    cx, cy = x0 + S / 2, y0 + S / 2
    # ideal per-frame motion of the patch centre (tracked from frame 0 onward, approx: local)
    ideal = []
    for n in range(1, len(v)):
        # position of the point currently at the patch centre at frame n-1, at frame n
        a = ideal_pos(frm, to, n - 1, cx, cy); b = ideal_pos(frm, to, n, cx, cy)
        ideal.append((b[0] - a[0], b[1] - a[1]))
    ideal = np.array(ideal)
    err = meas - ideal
    jerk = np.diff(meas, n=2, axis=0)
    print(f"{label}: frame-to-frame error RMS {np.sqrt((err ** 2).mean()):.3f} px, max {np.abs(err).max():.3f} px; "
          f"second difference RMS {np.sqrt((jerk ** 2).mean()):.3f} (ideal {np.sqrt((np.diff(ideal, n=2, axis=0) ** 2).mean()):.3f})")


for label, frm, to, patch in [
    ("pan", (-0.03, 0.0, 1.15), (0.03, 0.0, 1.15), (832, 412)),
    ("zoom", (0.0, 0.0, 1.0), (0.0, 0.0, 1.2), (200, 150)),
]:
    v, took = run(frm, to, MODE, label)
    print(f"[{MODE}] {label} rendered {len(v)} frames in {took:.1f} s")
    analyse(v, frm, to, f"[{MODE}] {label}", patch)
