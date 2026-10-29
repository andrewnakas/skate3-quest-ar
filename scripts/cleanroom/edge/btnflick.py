"""Each button held through an ollie flick (and through a kickflip): what trick each game makes."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from controls import *
FL = {"ollie": "0 12 0 0 0 -1 {b} {lt} {rt}\n13 19 0 0 0 1 {b} {lt} {rt}\n20 60 0 0 0 0 {b} {lt} {rt}\n",
      "kick": "0 12 0 0 0 -1 {b} {lt} {rt}\n13 13 0 0 0 0 {b} {lt} {rt}\n14 17 0 0 -0.707 0.707 {b} {lt} {rt}\n18 60 0 0 0 0 {b} {lt} {rt}\n"}
if __name__ == "__main__":
    for fl, tmpl in FL.items():
        for n, (b, lt, rt) in {"-": (0, 0, 0), **{k: (v, 0, 0) for k, v in B.items() if k not in ("BACK", "START")}, "LT": (0, 1, 0), "RT": (0, 0, 1)}.items():
            pr, sk = pair(tmpl.format(b=b, lt=lt, rt=rt), 110, extra=("--host-map",))
            pn = sorted(set(r["trick_name"] for r in pr) - {""})
            sn = sorted(set(e[4:] for r in sk for e in r["events"].split("|") if e[:4] in ("EV0:", "EV1:")))
            pb = any(r["wiping_out"] == "1" for r in pr); sb = any(r["mode"] == "Bail" for r in sk)
            ph = max(float(r["deck_py"]) for r in pr) - float(pr[0]["deck_py"]); sh = max(float(r["deck_py"]) for r in sk)
            print(f"{fl:5s} +{n:4s} P {pn} h{ph:.2f}{' BAIL' if pb else ''} | S {sn} h{sh:.2f}{' BAIL' if sb else ''}")
