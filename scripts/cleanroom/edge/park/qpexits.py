"""Exit speed off the quarter pipe (plain, ollies at the lip) by approach speed, port vs sk8."""
import sys, os, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from park import *
if __name__ == "__main__":
    for brake in (0, 15, 25, 35, 45):
        pre = f"{T0-55} {T0-55+brake} 0 0 0 0 0x2000\n" if brake else ""
        for name, act in (("ollie@24", "16 24 0 0 0 -1 0\n25 31 0 0 0 1 0"), ("ollie@30", "22 30 0 0 0 -1 0\n31 37 0 0 0 1 0"), ("ollie@36", "28 36 0 0 0 -1 0\n37 43 0 0 0 1 0")):
            pr, sk = run(act, 200, pre=pre)
            ev = sorted(set(e.split(':')[1] for r in sk for e in r['events'].split('|') if ':' in e) - {''})
            print(f"brake {brake:2d} v {sp(pr[0]):.2f} {name} exit P {sp(pr[-1]):5.2f} S {sp(sk[-1]):5.2f}  peak P {max(float(r['deck_py']) for r in pr):.2f} S {max(float(r['deck_py']) for r in sk):.2f} {ev}")
