from edge import *
S = feel.ON_RAIL
for k,(x,y) in {"50-50":(0,0),"5-0":(0,-1),"nose":(0,1),"crook":(0.71,0.71),"board":(1,0),"lip":(-1,0),"overcrook":(-0.71,0.71)}.items():
    pr, sk = pair(f"0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n30 140 0 0 {x} {y} 0", 160, S)
    f = lambda rows, a, b: (hs(rows[a]) - hs(rows[b])) / (b - a) * 60
    print(f"{k:10s} port v70 {hs(pr[70]):.2f} decel70-100 {f(pr,70,100):.2f} | sk8 v70 {hs(sk[70]):.2f} decel {f(sk,70,100):.2f}  [{pr[85]['phys_state'][:12]} | {sk[85]['mode']}]")
