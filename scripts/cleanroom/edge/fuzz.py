"""Random plausible play on the flat (pushes, steering, pops, flips, spins, grabs, manuals, brakes),
the same input through both games: flags runs whose tricks or bails differ.
Usage: fuzz.py [seeds] [first_seed]"""
import sys, os, random, re
from edge import _rows
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from edge import *
D_ = 0.707
FLICKS = {"ollie": [(0, 0), (0, 1)], "kick": [(0, 0), (-D_, D_)], "heel": [(0, 0), (D_, D_)],
          "shove": [(D_, -D_), (1, 0), (D_, D_), (0, 1)], "fsshove": [(-D_, -D_), (-1, 0), (-D_, D_), (0, 1)],
          "nollie": None}
def gen(rng, ticks=150):
    lines, t = [], 0
    while t < ticks - 20:
        a = rng.choice(["push", "push", "steer", "pop", "pop", "pop", "manual", "brake", "coast"])
        if a == "push":
            n = rng.choice([3, 12]); lines.append(f"{t} {t+n} 0 0 0 0 0x1000"); t += n + rng.randint(20, 50)
        elif a == "steer":
            n = rng.randint(10, 40); lines.append(f"{t} {t+n} {rng.choice([-1,-0.6,0.6,1])} 0 0 0 0"); t += n
        elif a == "pop":
            c = rng.choice([4, 10, 25]); kind = rng.choice(["ollie", "kick", "heel", "shove", "fsshove", "ollie"])
            lines.append(f"{t} {t+c} 0 0 0 -1 0"); t += c + 1
            for x, y in FLICKS[kind]:
                lines.append(f"{t} {t} 0 0 {x} {y} 0"); t += 1
            hold = FLICKS[kind][-1]
            if rng.random() < 0.3:
                lines.append(f"{t} {t+15} 0 0 {hold[0]} {hold[1]} 0"); t += 16
            extra = rng.choice(["none", "spin", "grab", "none"])
            if extra == "spin":
                lines.append(f"{t+3} {t+40} {rng.choice([-1,-0.8,0.8,1])} 0 0 0 0")
            elif extra == "grab":
                lines.append(f"{t+6} {t+30} 0 0 0 {rng.choice([1,-1])} 0 {rng.choice(['1 0','0 1'])}")
            t += 60
        elif a == "manual":
            n = rng.randint(30, 70); lines.append(f"{t} {t+n} 0 0 0 {rng.choice([-0.5,0.5])} 0"); t += n + 5
        elif a == "brake":
            n = rng.randint(10, 30); lines.append(f"{t} {t+n} 0 0 0 0 0x2000"); t += n + 30
        else:
            t += rng.randint(10, 30)
    return "\n".join(lines) + "\n"
SYN = [("360flip", "treflip"), ("fspopshuvit", "fsshoveit"), ("popshuvit", "shoveit"), ("frontside", "fs"),
       ("backside", "bs"), ("shuvit", "shoveit"), ("pop", "")]
def canon(n):
    n = re.sub(r"\s+(180|360|540|720)$", "", n)
    n = re.sub(r"^(Fakie|Switch|Nollie)\s+(?=\S)", lambda m: "" if m.group(1) != "Nollie" else "Nollie ", n)
    w = sorted(x for x in re.split(r"[^a-z0-9]+", n.lower()) if x)
    s = "".join(w)
    for a, b in SYN: s = s.replace(a, b)
    return "".join(sorted(s))  # (order-free)
def tricks(rows, port_):
    out = []
    for r in rows:
        names = [r["trick_name"]] if port_ else [e[4:] for e in r["events"].split("|") if e[:4] in ("EV0:", "EV1:")]
        for n in names:
            if n and (not out or out[-1] != n): out.append(n)
    return out
def bailed(rows, port_):
    return any((r["wiping_out"] == "1") if port_ else r["mode"] == "Bail" for r in rows)
if __name__ == "__main__":
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 30
    s0 = int(sys.argv[2]) if len(sys.argv) > 2 else 0
    bad = 0
    for seed in range(s0, s0 + n):
        act = gen(random.Random(seed))
        pr, sk = pair(act, 160, extra=("--host-map",))
        a, b = tricks(pr[:160], True), tricks(sk[:160], False)
        skip = ("grab", "manual", "grind", "slide", "big air", "long air", "50-50", "5-0", "smith", "crook", "salad", "feeble", "willy")
        pa = sorted(set(canon(x) for x in a if not x.startswith(("BS_", "FS_")) and "MANUAL" not in x))
        sb = sorted(set(canon(x) for x in b if not any(k in x.lower() for k in skip)))
        ba, bb = bailed(pr[:160], True), bailed(sk[:160], False)
        same = pa == sb and ba == bb
        bad += not same
        if not same or "-v" in sys.argv:
            print(f"seed {seed}: port {a}{' BAIL' if ba else ''}\n         sk8  {b}{' BAIL' if bb else ''}")
    print(f"{bad}/{n} runs differ")
