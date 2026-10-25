from edge import *
S = feel.ON_RAIL
def tl(pr, sk):
    out=[]; pg=pt=None
    for i,r in enumerate(pr):
        g=r["phys_state"]
        if g!=pg: out.append(f"{i}:{g[:12]}"); pg=g
        if r["trick_name"] and r["trick_name"]!=pt: out.append(f"{i}:T:{r['trick_name']}"); pt=r["trick_name"]
    o2=[]; pm=None
    for i,r in enumerate(sk):
        if r["mode"]!=pm: o2.append(f"{i}:{r['mode']}"); pm=r["mode"]
        if r["events"]: o2.append(f"{i}:{r['events']}")
    return " ".join(out), " ".join(o2)
def railspeed(rows, port):
    g=[(i,hs(r)) for i,r in enumerate(rows) if (r["phys_state"].startswith("Grind") if port else r["mode"]=="Grind")]
    if not g: return "no grind"
    return f"on {g[0][0]}-{g[-1][0]} v {g[0][1]:.2f}->{g[-1][1]:.2f} decel {(g[0][1]-g[-1][1])/max(1,(g[-1][0]-g[0][0]))*60:.2f}"
if __name__ == "__main__":
    kinds = {"50-50":(0,0), "5-0":(0,-1), "nose":(0,1), "crook":(0.8,0.8), "overcrook":(-0.8,0.8), "lip(-1,0)":(-1,0), "(1,0)":(1,0), "willy":(-0.8,-0.8), "salad?(0.8,-0.8)":(0.8,-0.8)}
    for k,(x,y) in kinds.items():
        act = f"0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n30 140 0 0 {x} {y} 0"
        pr, sk = pair(act, 160, S)
        a,b = tl(pr, sk)
        print(f"== {k}\n  port {railspeed(pr,True)} | {a}\n  sk8  {railspeed(sk,False)} | {b}")
