"""Round 4I, Task 1: why People misses photos, orphans faces and forms no new people.

Run on a COPY of library.db (never the library's own file):
    python scripts/ml/people_diagnose.py <copy of library.db> [--me <person id>] [--album <album id>]

Answers Julian's reports of 2026-09-26 with numbers:
  3. faces pinned to nobody (taken out of a person), and the photos that hold them;
  5. the face scan after the latest imports: scanned or not, and where their faces went
     (joined, doubtful, pool, a new person);
  4. with --me: the person's faces and the unplaced faces whose best match is them, by
     cosine; with --album (photos Julian says he is in): detected or not, placed or not;
  1/6. which people lost the most faces to corrections, at what similarity those faces had
     joined, and how many suggestions wait per person.
Prints ids and numbers only, never names.
"""
import sqlite3
import sys
import time
from collections import Counter, defaultdict

import numpy as np

args = sys.argv[1:]
DB = args[0]
ME = int(args[args.index("--me") + 1]) if "--me" in args else None
ALBUM = int(args[args.index("--album") + 1]) if "--album" in args else None
# The thresholds shipped in Round 4B (`people::Thresholds::MEASURED`).
JOIN, MARGIN, SUGGEST, CLUSTER = 0.45, 0.05, 0.363, 0.45
EXEMPLARS, DISTINCT, QUALITY = 12, 0.8, 0.6

c = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)


def unit(b):
    v = np.frombuffer(b, dtype=np.int8).astype(np.float32)
    return v / (np.linalg.norm(v) + 1e-9)


faces = c.execute(
    "SELECT f.id, f.asset_id, f.person_id, f.pinned, f.rejected, COALESCE(f.quality, f.score), f.vec, "
    "a.state, a.concealed, COALESCE(a.imported_at, 0) FROM faces f JOIN assets a ON a.id = f.asset_id"
).fetchall()
F = {
    fid: dict(asset=aid, person=pid, pinned=bool(pin), rejected=rej, q=q, v=unit(vec), visible=(st == 0 and conc == 0), imported=imp)
    for fid, aid, pid, pin, rej, q, vec, st, conc, imp in faces
}
people = {pid: dict(named=nm is not None, status=status) for pid, nm, status in c.execute("SELECT id, name, status FROM people")}
refused = defaultdict(set)
for f, p in c.execute("SELECT face_id, person_id FROM face_rejections"):
    refused[f].add(p)

# Exemplars exactly as Round 4B builds them.
by_person = defaultdict(list)
for fid, f in F.items():
    if f["person"] is not None and f["rejected"] is None:
        by_person[f["person"]].append((f["q"], fid))
EX = {}
for pid, lst in by_person.items():
    lst.sort(key=lambda t: (-t[0], t[1]))
    best = lst[0][0]
    ex = []
    for q, fid in lst:
        if len(ex) == EXEMPLARS or (ex and q < best * QUALITY):
            break
        v = F[fid]["v"]
        if all(float(e @ v) < DISTINCT for e in ex):
            ex.append(v)
    EX[pid] = np.stack(ex)


def ranked(fid):
    v = F[fid]["v"]
    out = [(float((ex @ v).max()), pid) for pid, ex in EX.items() if pid not in refused[fid]]
    out.sort(reverse=True)
    return out


def fate(fid):
    """What Round 4B's recluster does with an unplaced face: join / doubtful / pool."""
    r = ranked(fid)
    if not r:
        return "pool", None, 0.0
    s, p = r[0]
    second = r[1][0] if len(r) > 1 else -1
    if s >= JOIN and s - second >= MARGIN:
        return "join", p, s
    if s >= SUGGEST:
        return "doubtful", p, s
    return "pool", p, s


def section(title):
    print(f"\n=== {title}")


live = {fid: f for fid, f in F.items() if f["rejected"] is None and f["visible"]}
section("totals")
print("faces", len(F), "live (visible, not rejected)", len(live), "placed", sum(f["person"] is not None for f in live.values()))
print("people", len(people), "listed", sum(p["status"] == "listed" for p in people.values()), "named", sum(p["named"] for p in people.values()))
print("rejected by reason", Counter(f["rejected"] for f in F.values() if f["rejected"]))

section("report 3: faces pinned to nobody")
orph = [fid for fid, f in live.items() if f["person"] is None and f["pinned"]]
print("orphans", len(orph), "with a rejection row", sum(bool(refused[f]) for f in orph), "photos", len({F[f]["asset"] for f in orph}))
print("what 4B would do with them if unpinned:", Counter(fate(f)[0] for f in orph))
for fid in orph[:20]:
    k, p, s = fate(fid)
    print(f"  face {fid} asset {F[fid]['asset']} refused {sorted(refused[fid])} -> {k} person {p} sim {s:.3f}")

section("report 5: the scan and the latest imports")
assets = c.execute("SELECT id, COALESCE(imported_at, 0) FROM assets WHERE state = 0 AND concealed = 0 AND media_type = 0").fetchall()
scan = dict(c.execute("SELECT asset_id, model FROM face_scan"))
print("photos", len(assets), "scan keys", Counter(scan.values()))
print("photos never scanned", sum(a not in scan for a, _ in assets))
latest = sorted({t for _, t in assets}, reverse=True)
cut = latest[0] - 14 * 86400 if latest else 0
recent = [a for a, t in assets if t >= cut]
print(f"imported in the last 14 days of the library ({time.strftime('%Y-%m-%d', time.gmtime(cut))} on):", len(recent), "scanned", sum(a in scan for a in recent))
rf = [fid for fid, f in live.items() if f["asset"] in set(recent)]
placed_new = Counter()
for fid in rf:
    f = F[fid]
    if f["person"] is not None:
        created = c.execute("SELECT created_at FROM people WHERE id = ?", (f["person"],)).fetchone()[0]
        placed_new["in a person created then" if created >= cut else "in an older person"] += 1
    else:
        placed_new["unplaced: " + fate(fid)[0]] += 1
print("their faces", len(rf), dict(placed_new))
pool = [fid for fid, f in live.items() if f["person"] is None and not f["pinned"] and fate(fid)[0] == "pool"]
doubt = [fid for fid, f in live.items() if f["person"] is None and not f["pinned"] and fate(fid)[0] == "doubtful"]
print("all unplaced, unpinned faces: pool", len(pool), "doubtful", len(doubt))
# How tight are doubtful faces among themselves? A group of them may be a new person.
if doubt:
    D = np.stack([F[f]["v"] for f in doubt])
    S = D @ D.T
    np.fill_diagonal(S, 0)
    close = (S >= CLUSTER).sum(1)
    print("doubtful faces with >=2 doubtful neighbours at >= cluster:", int((close >= 2).sum()), "of", len(doubt))

section("reports 1 and 6: corrections and suggestions")
lost = Counter()
for fid, ps in refused.items():
    for p in ps:
        lost[p] += 1
print("people by faces taken out of them (top 10: person, faces lost, faces now):", [(p, n, len(by_person.get(p, []))) for p, n in lost.most_common(10)])
pending = Counter(fate(f)[1] for f in doubt)
print("pending suggestions per person (top 10):", pending.most_common(10))
# At what similarity did the faces the user removed sit to the person they were taken from?
sims = []
for fid, ps in refused.items():
    if fid not in F:
        continue
    for p in ps:
        if p in EX:
            sims.append(float((EX[p] @ F[fid]["v"]).max()))
if sims:
    print("removed faces' similarity to the person they were taken from: pct 10/25/50/75/90", np.percentile(sims, [10, 25, 50, 75, 90]).round(3))

if ME is not None:
    section(f"report 4: person {ME}")
    mine = [fid for fid, f in live.items() if f["person"] == ME]
    print("faces placed", len(mine), "photos", len({F[f]["asset"] for f in mine}), "exemplars", len(EX.get(ME, [])))
    cands = defaultdict(int)
    for fid, f in live.items():
        if f["person"] is not None or ME in refused[fid]:
            continue
        v = f["v"]
        s = float((EX[ME] @ v).max()) if ME in EX else 0
        r = ranked(fid)
        if r and r[0][1] == ME:
            b = "<.25" if s < 0.25 else "<.30" if s < 0.30 else "<.363" if s < 0.363 else "<.40" if s < 0.40 else "<.45" if s < 0.45 else ">=.45 (no margin)"
            cands[b] += 1
    print("unplaced faces whose best match is this person, by similarity:", dict(sorted(cands.items())))

if ALBUM is not None:
    section(f"report 4: album {ALBUM} (photos the person is in)")
    ids = [a for (a,) in c.execute("SELECT asset_id FROM album_assets WHERE album_id = ?", (ALBUM,))]
    by_asset = defaultdict(list)
    for fid, f in F.items():
        by_asset[f["asset"]].append(fid)
    out = Counter()
    for a in ids:
        fs = by_asset.get(a, [])
        if a not in scan:
            out["not scanned"] += 1
        elif not fs:
            out["scanned, no face found"] += 1
        elif ME is not None and any(F[f]["person"] == ME for f in fs):
            out["placed on the person"] += 1
        elif all(F[f]["rejected"] for f in fs):
            out["faces found but all filtered (" + ",".join(sorted({F[f]['rejected'] for f in fs})) + ")"] += 1
        else:
            live_fs = [f for f in fs if F[f]["rejected"] is None]
            kinds = Counter(("placed elsewhere" if F[f]["person"] is not None else fate(f)[0]) for f in live_fs)
            out["faces found: " + ", ".join(f"{k} {n}" for k, n in sorted(kinds.items()))] += 1
    for k, n in out.most_common():
        print(f"  {n:4d}  {k}")
