"""Braking after pushing up to speed (held push to tick 119), B held from tick 120+delay for 40 ticks: speed every 4 ticks."""
from rest import *
if __name__ == "__main__":
    for delay in (0, 20, 60):
        b = 120 + delay
        act = f"0 119 0 0 0 0 0x1000\n{b} {b+40} 0 0 0 0 0x2000\n"
        pr, sk = rest_pair(act, b + 60)
        print(f"delay {delay:2d} P " + " ".join(f"{hs3(pr[b+i]):4.2f}" for i in range(0, 56, 4)))
        print(f"         S " + " ".join(f"{hs3(sk[b+i]):4.2f}" for i in range(0, 56, 4)) + "  " + " ".join(sorted(set(sk[b+i]['mode'] for i in range(40)))))
