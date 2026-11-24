"""Round 24 W2 Step 3: Wayfinder's speed and accuracy check on a COPY of a library.

Never point this at a live library.db: `prep` reads only the copy of the database you give it and
copies (reads, never writes) the 1024 px thumbnails of the photos that have camera GPS.

    # 1. Windows Python (sqlite3 + numpy). <copy> holds library.db (+ -wal, -shm) copied from the
    #    library; <library> is the library folder, opened read-only for its thumbnails.
    python scripts/ml/eval_wayfinder.py prep --db-copy <copy>/library.db --library <library> --out <scratch>
    # 2. photos-ml/tests/wayfinder_eval.rs embeds three views of every thumbnail on DirectML
    #    (LUMEN_WAYFINDER_DIR=... LUMEN_WAYFINDER_EVAL=<scratch> cargo test --release ...)
    # 3. WSL venv-wayfinder (numpy + onnxruntime), model files from export_wayfinder.py:
    python scripts/ml/eval_wayfinder.py score --out <scratch> --models <wayfinder dir>
    # FindMyPhoto's path (GeoCLIP's own CPU predict), 50 photos, for the speed comparison:
    python scripts/ml/eval_wayfinder.py baseline --out <scratch> --n 50

`score` hides each of 500 sampled photos' location (the "test" set) and treats the other
camera-GPS photos as the person's own geotagged set (the "personal" set). Methods, each as the
median error in km and the share within 1 / 25 / 200 / 750 km:

- raw GeoCLIP: the gallery place with the highest cosine to the image embedding;
- + crops: the embedding is the mean of the full view and two centre crops;
- + personal retrieval: the nearest personal photo (cosine of the same embeddings) above a
  threshold takes its place, else GeoCLIP's;
- + denser gallery: extra gallery points (1 km grid within 25 km of each cluster of the
  personal set) encoded with `wayfinder-location.onnx` join the search;
- + library prior: GeoCLIP's top 50 re-ranked by closeness to the person's clusters;
- everything together.

Two scenarios: the personal photos within 2 hours of a test photo are left out (a trip whose
other photos have GPS), or those within 7 days (only places visited on other trips can help).
Thresholds and the prior weight are tuned on a first half of the test set and every row is
reported on the second half (the first-half and 500-photo numbers are in the JSON).
"""

import argparse
import json
import math
import random
import sqlite3
import struct
import time
from pathlib import Path

import numpy as np

DIM = 512
SCENARIOS = {"same trip (hide +-2 h)": 2 * 3600, "other trips (hide +-7 d)": 7 * 86400}
BANDS = [1, 25, 200, 750]
THRESHOLDS = [0.50, 0.55, 0.60, 0.65, 0.70, 0.75, 0.80, 0.85, 0.90]
PRIOR_WEIGHTS = [0.0, 3.0, 10.0, 30.0, 100.0, 300.0, 1000.0]


# ----------------------------------------------------------------------------- prep


def prep(a):
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(f"file:{Path(a.db_copy).as_posix()}?mode=ro", uri=True)
    base = Path(a.library) / "derivatives" / "thumbs"
    rows = db.execute(
        "select id, content_hash, gps_lat, gps_lon, coalesce(captured_at_utc, captured_at) "
        "from assets where gps_lat is not null and gps_lon is not null and media_type = 0 "
        "and state = 0 order by id"
    ).fetchall()
    photos = []
    for pid, h, lat, lon, t in rows:
        thumb = base / h[:2] / f"{h}_1024.jpg"
        if thumb.exists() and t is not None:
            photos.append({"id": pid, "lat": lat, "lon": lon, "t": t, "thumb": str(thumb)})
    rng = random.Random(24)
    test = sorted(rng.sample(range(len(photos)), a.test))
    (out / "photos.json").write_text(json.dumps({"photos": photos, "test": test}))
    (out / "thumbs.txt").write_text("\n".join(p["thumb"] for p in photos) + "\n")
    print(f"{len(rows)} GPS stills, {len(photos)} with a 1024 thumbnail, {len(test)} test photos")


# ----------------------------------------------------------------------------- shared


def haversine(lat1, lon1, lat2, lon2):
    """Kilometres; arguments broadcast."""
    p1, p2 = np.radians(lat1), np.radians(lat2)
    dphi, dl = p2 - p1, np.radians(lon2) - np.radians(lon1)
    h = np.sin(dphi / 2) ** 2 + np.cos(p1) * np.cos(p2) * np.sin(dl / 2) ** 2
    return 2 * 6371.0088 * np.arcsin(np.sqrt(np.clip(h, 0, 1)))


def read_gallery(path):
    b = Path(path).read_bytes()
    assert b[:4] == b"LWVG"
    version, rows, dim, dtype = struct.unpack_from("<4I", b, 4)
    assert (version, dim, dtype) == (1, DIM, 1)
    m = 20 + rows * dim * 2
    emb = np.frombuffer(b, np.float16, rows * dim, 20).reshape(rows, dim).astype(np.float32)
    gps = np.frombuffer(b, np.float32, rows * 2, m).reshape(rows, 2)
    return emb, gps


def unit(x):
    return x / np.linalg.norm(x, axis=-1, keepdims=True)


def clusters(lat, lon, radius_km=10.0, min_photos=3):
    """Greedy: the densest unassigned photo seeds a cluster of everything within radius_km."""
    left = np.ones(len(lat), bool)
    out = []
    while left.any():
        idx = np.flatnonzero(left)
        # Count neighbours of a bounded random subset to keep this quick and deterministic.
        rng = np.random.default_rng(len(idx))
        probe = rng.choice(idx, min(len(idx), 200), replace=False)
        counts = [(haversine(lat[i], lon[i], lat[idx], lon[idx]) <= radius_km).sum() for i in probe]
        seed = probe[int(np.argmax(counts))]
        member = idx[haversine(lat[seed], lon[seed], lat[idx], lon[idx]) <= radius_km]
        left[member] = False
        if len(member) >= min_photos:
            out.append((float(lat[member].mean()), float(lon[member].mean()), len(member)))
    return out


def grid_points(centres, step_km=1.0, radius_km=25.0):
    pts = []
    n = int(radius_km / step_km)
    for clat, clon, _ in centres:
        dlat = step_km / 111.195
        dlon = step_km / (111.195 * max(math.cos(math.radians(clat)), 0.05))
        for i in range(-n, n + 1):
            for j in range(-n, n + 1):
                if i * i + j * j <= n * n:
                    lat = clat + i * dlat
                    lon = clon + j * dlon
                    if -90 <= lat <= 90:
                        pts.append((lat, ((lon + 180) % 360) - 180))
    return np.array(pts, np.float32)


def encode_locations(models, pts):
    import onnxruntime as ort

    s = ort.InferenceSession(str(Path(models) / "wayfinder-location.onnx"), providers=["CPUExecutionProvider"])
    out = np.empty((len(pts), DIM), np.float32)
    for i in range(0, len(pts), 4096):
        out[i : i + 4096] = s.run(None, {"latlon": pts[i : i + 4096]})[0]
    return unit(out)


def stats(err):
    err = np.asarray(err)
    return {"median_km": float(np.median(err)), **{f"within_{b}_km": float((err <= b).mean()) for b in BANDS}}


# ----------------------------------------------------------------------------- score


def score(a):
    out = Path(a.out)
    meta = json.loads((out / "photos.json").read_text())
    photos, test = meta["photos"], np.array(meta["test"])
    n = len(photos)
    emb = np.fromfile(out / "embeddings.f32", np.float32).reshape(n, 3, DIM)
    lat = np.array([p["lat"] for p in photos])
    lon = np.array([p["lon"] for p in photos])
    t = np.array([p["t"] for p in photos], np.int64)
    full = unit(emb[:, 0])
    crops = unit(emb.mean(axis=1))
    g_emb, g_gps = read_gallery(Path(a.models) / "wayfinder-gallery.bin")

    is_test = np.zeros(n, bool)
    is_test[test] = True
    personal = np.flatnonzero(~is_test)
    # The first half of the test photos tunes, the second half reports.
    half = len(test) // 2
    tune, report = test[:half], test[half:]

    print(f"{n} photos, {len(test)} test ({len(tune)} tune / {len(report)} report), {len(personal)} personal")
    cl = clusters(lat[personal], lon[personal])
    print(f"{len(cl)} personal clusters (10 km, >= 3 photos); {sum(c[2] for c in cl)} photos in them")
    extra_pts = grid_points(cl)
    t0 = time.time()
    extra_emb = encode_locations(a.models, extra_pts)
    print(f"denser gallery: {len(extra_pts)} extra points encoded in {time.time() - t0:.0f} s")
    centres = np.array([[c[0], c[1]] for c in cl], np.float32)

    def gallery_scores(e):
        return g_emb @ e

    def prior_dist(plat, plon):
        return haversine(plat, plon, centres[:, 0], centres[:, 1]).min()

    def predict(i, window, view, personal_thr=None, dense=False, prior_w=0.0):
        e = view[i]
        s = gallery_scores(e)
        cand_lat, cand_lon, cand_s = g_gps[:, 0], g_gps[:, 1], s
        if dense:
            es = extra_emb @ e
            cand_lat = np.concatenate([cand_lat, extra_pts[:, 0]])
            cand_lon = np.concatenate([cand_lon, extra_pts[:, 1]])
            cand_s = np.concatenate([s, es])
        if prior_w > 0:
            top = np.argpartition(-cand_s, 50)[:50]
            boost = np.array([prior_dist(cand_lat[k], cand_lon[k]) for k in top])
            # exp(40 * s) is GeoCLIP's own softmax weight (logit scale 39.69).
            w = np.exp(39.69 * (cand_s[top] - cand_s[top].max())) * (1 + prior_w * np.exp(-boost / 25.0))
            k = top[int(np.argmax(w))]
        else:
            k = int(np.argmax(cand_s))
        pred = (cand_lat[k], cand_lon[k])
        if personal_thr is not None:
            ok = personal[np.abs(t[personal] - t[i]) > window]
            sims = view[ok] @ view[i]
            j = int(np.argmax(sims))
            if sims[j] >= personal_thr:
                pred = (lat[ok[j]], lon[ok[j]])
        return pred

    def evaluate(idx, window, view=full, **kw):
        errs = []
        for i in idx:
            p = predict(i, window, view, **kw)
            errs.append(float(haversine(lat[i], lon[i], p[0], p[1])))
        return stats(errs)

    result = {"photos": n, "test": len(test), "personal": len(personal), "clusters": len(cl),
              "extra_gallery_points": int(len(extra_pts)), "scenarios": {}}
    for name, window in SCENARIOS.items():
        # Tune on the first half: personal threshold and prior weight.
        # Best share within 25 km (the median alone barely moves when most photos are far off).
        thr_curve = {th: evaluate(tune, window, personal_thr=th) for th in THRESHOLDS}
        best_thr = max(thr_curve, key=lambda th: (thr_curve[th]["within_25_km"], -thr_curve[th]["median_km"]))
        w_curve = {w: evaluate(tune, window, prior_w=w) for w in PRIOR_WEIGHTS}
        best_w = max(w_curve, key=lambda w: (w_curve[w]["within_25_km"], -w_curve[w]["median_km"]))
        rows = {
            "raw GeoCLIP": dict(),
            "+ crops": dict(view=crops),
            f"+ personal retrieval (thr {best_thr})": dict(personal_thr=best_thr),
            "+ denser gallery": dict(dense=True),
            f"+ library prior (w {best_w})": dict(prior_w=best_w),
            "all (no crops)": dict(personal_thr=best_thr, dense=True, prior_w=best_w),
            "all + crops": dict(personal_thr=best_thr, dense=True, prior_w=best_w, view=crops),
        }
        table = {k: {"report": evaluate(report, window, **kw), "all_500": evaluate(test, window, **kw)}
                 for k, kw in rows.items()}
        result["scenarios"][name] = {"tuned": {"personal_threshold": best_thr, "prior_weight": best_w,
                                               "threshold_curve_tune_half": {str(k): v for k, v in thr_curve.items()},
                                               "prior_curve_tune_half": {str(k): v for k, v in w_curve.items()}},
                                     "rows": table}
        print(f"\n### {name}  (report half, {len(report)} photos; tuned thr {best_thr}, prior w {best_w})")
        print("| method | median km | <=1 km | <=25 km | <=200 km | <=750 km |")
        print("|---|---|---|---|---|---|")
        for k, v in table.items():
            r = v["report"]
            print(f"| {k} | {r['median_km']:.1f} | {r['within_1_km']:.0%} | {r['within_25_km']:.0%} | "
                  f"{r['within_200_km']:.0%} | {r['within_750_km']:.0%} |")
    (out / "accuracy.json").write_text(json.dumps(result, indent=1))
    print(f"\nwrote {out / 'accuracy.json'}")


# ----------------------------------------------------------------------------- baseline


def baseline(a):
    """FindMyPhoto's path: the geoclip package's own CPU predict(), photo by photo."""
    import torch
    from geoclip import GeoCLIP

    out = Path(a.out)
    meta = json.loads((out / "photos.json").read_text())
    paths = [meta["photos"][i]["thumb"] for i in meta["test"][: a.n]]
    paths = [p.replace("C:\\", "/mnt/c/").replace("\\", "/") if p[1:2] == ":" else p for p in paths]
    torch.set_num_threads(a.threads)
    model = GeoCLIP().to("cpu")
    model.predict(paths[0], top_k=1)  # warm-up
    t0 = time.time()
    errs = []
    for p, i in zip(paths, meta["test"][: a.n]):
        gps, _ = model.predict(p, top_k=1)
        la, lo = [float(v) for v in gps[0]]
        ph = meta["photos"][i]
        errs.append(float(haversine(ph["lat"], ph["lon"], la, lo)))
    dt = time.time() - t0
    res = {"photos": len(paths), "seconds": dt, "photos_per_s": len(paths) / dt, "threads": a.threads,
           **stats(errs)}
    (out / "baseline.json").write_text(json.dumps(res, indent=1))
    print(json.dumps(res, indent=1))


# ----------------------------------------------------------------------------- challenger


def plonk(a):
    """PLONK_YFCC (Dufour et al., CVPR 2025) on the same test photos, CUDA PyTorch (`pip install
    diff-plonk` in a venv). The model samples places, so each photo gets `--samples` draws and the
    answer is the draw with most neighbours within 25 km (a density mode); `--samples 1` is one draw."""
    import torch
    from PIL import Image
    from plonk import PlonkPipeline

    out = Path(a.out)
    meta = json.loads((out / "photos.json").read_text())
    idx = meta["test"][: a.n] if a.n else meta["test"]
    fix = lambda p: p.replace("C:\\", "/mnt/c/").replace("\\", "/") if p[1:2] == ":" else p
    pipe = PlonkPipeline("nicolas-dufour/PLONK_YFCC")
    pipe.to("cuda")
    gen = torch.Generator(device="cuda").manual_seed(24)

    def locate(img, k):
        c = np.asarray(pipe([img], batch_size=k, generator=gen)).reshape(-1, 2)
        if k == 1:
            return c[0]
        d = haversine(c[:, None, 0], c[:, None, 1], c[None, :, 0], c[None, :, 1])
        return c[int(np.argmax((d <= 25).sum(1)))]

    locate(Image.open(fix(meta["photos"][idx[0]]["thumb"])).convert("RGB"), a.samples)  # warm-up
    errs, t0 = [], time.time()
    for i in idx:
        ph = meta["photos"][i]
        la, lo = locate(Image.open(fix(ph["thumb"])).convert("RGB"), a.samples)
        errs.append(float(haversine(ph["lat"], ph["lon"], la, lo)))
    dt = time.time() - t0
    half = len(meta["test"]) // 2
    in_report = [e for e, i in zip(errs, idx) if i in set(meta["test"][half:])]
    res = {"samples": a.samples, "photos": len(idx), "photos_per_s": len(idx) / dt,
           "all": stats(errs), "report_half": stats(in_report) if in_report else None}
    (out / f"plonk-{a.samples}.json").write_text(json.dumps(res, indent=1))
    print(json.dumps(res, indent=1))


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("prep")
    p.add_argument("--db-copy", required=True)
    p.add_argument("--library", required=True)
    p.add_argument("--out", required=True)
    p.add_argument("--test", type=int, default=500)
    p.set_defaults(fn=prep)
    p = sub.add_parser("score")
    p.add_argument("--out", required=True)
    p.add_argument("--models", required=True)
    p.set_defaults(fn=score)
    p = sub.add_parser("baseline")
    p.add_argument("--out", required=True)
    p.add_argument("--n", type=int, default=50)
    p.add_argument("--threads", type=int, default=8)
    p.set_defaults(fn=baseline)
    p = sub.add_parser("plonk")
    p.add_argument("--out", required=True)
    p.add_argument("--n", type=int, default=0, help="first n test photos (0 = all)")
    p.add_argument("--samples", type=int, default=16)
    p.set_defaults(fn=plonk)
    args = ap.parse_args()
    args.fn(args)
