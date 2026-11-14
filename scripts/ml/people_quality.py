"""Round 4B, Task 1: face quality measured against the user's own corrections.

Run on a COPY of library.db (never the library's own file):
    python scripts/ml/people_quality.py <copy of library.db>

Groups: faces in named, listed people (kept), faces the user took out of a person
(pinned, no person: "removed"), faces of hidden people, unassigned faces. Prints each
group's detector score, face size in the 1024 px scan, CLIP junk share of the photo and
SFace similarity to the nearest named person; then how many named faces each join rule
keeps and how many removed faces it would put back; then the photos' CLIP labels and tags.
Prints ids and numbers only, never names.
"""
import sys
from pathlib import Path
# Relative to this script, so the checkout can live in any folder.
PROTOTYPES = Path(__file__).resolve().parents[2] / "src-tauri" / "crates" / "photos-ml" / "prototypes.json"
DB = sys.argv[1] if len(sys.argv) > 1 else "library.db"
# --- features ---
import sqlite3, json, numpy as np
from collections import defaultdict
c=sqlite3.connect(DB)
P=json.load(open(PROTOTYPES))["prototypes"]
names=list(P); M=np.array([P[n] for n in names],dtype=np.float32); M/=np.linalg.norm(M,axis=1,keepdims=True)
def unblob(b): return np.frombuffer(b,dtype=np.int8).astype(np.float32)
emb={a:unblob(v) for a,v in c.execute("select asset_id, vec from asset_embeddings")}
def shares(v,prefix,t,extra=()):
    idx=[i for i,n in enumerate(names) if n.startswith(prefix) or n in extra]
    v=v/ (np.linalg.norm(v)+1e-9); s=M[idx]@v/t; s=np.exp(s-s.max()); s/=s.sum()
    return {names[i]:float(x) for i,x in zip(idx,s)}
people={pid:(name,hid) for pid,name,hid in c.execute("select id,name,hidden from people")}
rows=c.execute("select f.id,f.asset_id,f.x,f.y,f.w,f.h,f.score,f.vec,f.person_id,f.pinned,a.width,a.height,a.orientation from faces f join assets a on a.id=f.asset_id").fetchall()
F=[]
for fid,aid,x,y,w,h,score,vec,pid,pinned,aw,ah,o in rows:
    if o in (5,6,7,8): aw,ah=ah,aw
    k=1024/max(aw,ah); side=max(w*aw,h*ah)*k
    if pid is None: lab="removed" if pinned else "none"
    else:
        nm,hid=people[pid]; lab="hidden" if hid else ("named" if nm else "unnamed")
    e=emb.get(aid); j=shares(e,"junk:",0.05,("good",)) if e is not None else {}
    junk=max([v for n,v in j.items() if n.startswith("junk:")],default=np.nan)
    topjunk=max(j,key=j.get) if j else ""
    F.append(dict(id=fid,aid=aid,lab=lab,score=score,side=side,junk=junk,topjunk=topjunk,v=unblob(vec),pid=pid,pinned=bool(pinned)))
# centroids of named people
cent={}
for pid,(nm,hid) in people.items():
    vs=[f["v"] for f in F if f["pid"]==pid and f["lab"]=="named"]
    if vs:
        m=np.mean([v/np.linalg.norm(v) for v in vs],axis=0); cent[pid]=m/np.linalg.norm(m)
for f in F:
    v=f["v"]/np.linalg.norm(f["v"])
    sims=sorted(((float(cc@v),pid) for pid,cc in cent.items() if pid!=f["pid"] or f["lab"]!="named"),reverse=True)
    f["best_other"]=sims[0][0] if sims else np.nan
    f["own"]=float(cent[f["pid"]]@v) if f["lab"]=="named" and f["pid"] in cent else np.nan
import pickle; 
def q(xs,ps=(5,25,50,75,95)): xs=np.array([x for x in xs if x==x]); return (len(xs),[round(float(np.percentile(xs,p)),3) for p in ps]) if len(xs) else (0,[])
for lab in ["named","unnamed","hidden","removed","none"]:
    G=[f for f in F if f["lab"]==lab]
    print(f"== {lab} n={len(G)}")
    for k in ["score","side","junk","own","best_other"]:
        print("  ",k,q([f[k] for f in G]))

# --- join rules ---
import pickle, numpy as np

for f in F: f["u"]=f["v"]/np.linalg.norm(f["v"])
named=[f for f in F if f["lab"]=="named"]; removed=[f for f in F if f["lab"]=="removed"]
by={}
for f in named: by.setdefault(f["pid"],[]).append(f)
def exemplars(fs,k=12):
    fs=sorted(fs,key=lambda f:-f["score"]); out=[]
    for f in fs:
        if all(float(f["u"]@e["u"])<0.8 for e in out): out.append(f)
        if len(out)==k: break
    return out
def match(f, rule, excl=True):
    """best person and runner-up sims for face f under rule"""
    res=[]
    for pid,fs in by.items():
        pool=[g for g in fs if g is not f] if excl else fs
        if not pool: continue
        if rule=="centroid":
            m=np.mean([g["u"] for g in pool],axis=0); s=float(f["u"]@(m/np.linalg.norm(m)))
        else:
            s=max(float(f["u"]@g["u"]) for g in exemplars(pool))
        res.append((s,pid))
    res.sort(reverse=True); return res
for rule in ["centroid","exemplar"]:
    Mn=[(match(f,rule),f["pid"]) for f in named]; Mr=[match(f,rule,False) for f in removed]
    print("==",rule)
    for thr in [0.363,0.40,0.45,0.50,0.55]:
        for margin in [0.0,0.05]:
            ok=sum(1 for r,pid in Mn if r and r[0][1]==pid and r[0][0]>=thr and (len(r)<2 or r[0][0]-r[1][0]>=margin))
            wrong=sum(1 for r,pid in Mn if r and r[0][1]!=pid and r[0][0]>=thr and (len(r)<2 or r[0][0]-r[1][0]>=margin))
            fj=sum(1 for r in Mr if r and r[0][0]>=thr and (len(r)<2 or r[0][0]-r[1][0]>=margin))
            print(f"  thr {thr:.3f} margin {margin:.2f}: named rejoin own {ok}/{len(Mn)} ({ok/len(Mn):.0%}), named->other {wrong}, removed rejoined {fj}/{len(Mr)} ({fj/len(Mr):.0%})")

# --- Task 3: the shipped rule (people::Thresholds::MEASURED) ---
JOIN, MARGIN, SUGGEST = 0.45, 0.05, 0.363
pinned_in = [f for f in named if f["pinned"]]
def fate(r, pid):
    if not r: return "lost"
    lead = len(r) < 2 or r[0][0] - r[1][0] >= MARGIN
    if r[0][1] == pid and r[0][0] >= JOIN and lead: return "joined"
    if r[0][1] != pid and r[0][0] >= JOIN and lead: return "wrong"
    own = next((s for s, p in r if p == pid), -1)
    return "suggested" if own >= SUGGEST else "lost"
from collections import Counter as _C
got = _C(fate(match(f, "exemplar"), f["pid"]) for f in pinned_in)
print(f"== shipped rule on the {len(pinned_in)} faces pinned into named people:", dict(got),
      f"joined or suggested {(got['joined'] + got['suggested']) / max(1, len(pinned_in)):.0%}")
got = _C(fate(match(f, "exemplar"), f["pid"]) for f in named)
print(f"== shipped rule on all {len(named)} named faces:", dict(got),
      f"joined or suggested {(got['joined'] + got['suggested']) / max(1, len(named)):.0%}")

# --- photo labels ---
import sqlite3, json, pickle, numpy as np
from collections import Counter
c=sqlite3.connect(DB)
P=json.load(open(PROTOTYPES))["prototypes"]
names=list(P); M=np.array([P[n] for n in names],dtype=np.float32); M/=np.linalg.norm(M,axis=1,keepdims=True)
emb={a:np.frombuffer(v,dtype=np.int8).astype(np.float32) for a,v in c.execute("select asset_id, vec from asset_embeddings")}
tags={a:t for a,t in c.execute("select id, scene_tags from assets")}

def top(v,pred,t):
    idx=[i for i,n in enumerate(names) if pred(n)]; v=v/np.linalg.norm(v); s=M[idx]@v/t; s=np.exp(s-s.max()); s/=s.sum()
    i=int(np.argmax(s)); return names[idx[i]], float(s[i])
print([n for n in names if n.startswith("junk") or n=="good"])
for lab in ["named","removed","hidden"]:
    G=[f for f in F if f["lab"]==lab and f["aid"] in emb]
    jt=Counter(top(emb[f["aid"]],lambda n:n.startswith("junk:") or n=="good",0.05)[0] for f in G)
    tt=Counter(t for f in G for t in (tags.get(f["aid"]) or "").split())
    small=sum(f["side"]<40 for f in G)/len(G)
    print(f"== {lab} n={len(G)} small<40px={small:.0%}")
    print("   top junk/good:", jt.most_common(6))
    print("   scene tags:", tt.most_common(12))
