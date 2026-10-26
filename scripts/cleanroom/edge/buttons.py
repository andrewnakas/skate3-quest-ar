from edge import *
BTN = {"A": 0x1000, "B": 0x2000, "X": 0x4000, "Y": 0x8000, "LB": 0x0100, "RB": 0x0200, "LS": 0x0040, "RS": 0x0080}
def states(rows, a, b):
    out = []
    for r in rows[a:b]:
        tag = "/".join(r["motion_state"].split("/")[2:6]) + " | " + "/".join(r["action_state"].split("/")[3:6]) + (" T:" + r["trick_name"] if r["trick_name"] else "")
        if not out or out[-1] != tag: out.append(tag)
    return out
def sk8_tags(rows, a, b):
    out = []
    for r in rows[a:b]:
        tag = r["mode"] + (" " + r["events"] if r["events"] else "")
        if not out or out[-1] != tag: out.append(tag)
    return out
if __name__ == "__main__":
    import sys
    where = sys.argv[1] if len(sys.argv) > 1 else "ground"
    for name, bit in BTN.items():
        if where == "ground":
            act = f"0 60 0 0 0 0 {hex(bit)}"
        elif where == "steer":
            act = f"0 60 1 0 0 0 {hex(bit)}"
        else:  # in the air: ollie, then hold
            act = f"0 8 0 0 0 -1 0\n9 15 0 0 0 1 0\n18 60 0 0 0 0 {hex(bit)}"
        pr, sk = pair(act, 80)
        sk2 = None
        print(f"== {name} ({where})\n  port: {states(pr, 0, 80)[:6]}\n  sk8:  {sk8_tags(sk, 0, 80)[:6]}  speed {hs(pr[60]):.2f}/{hs(sk[60]):.2f}")
