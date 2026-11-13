"""The first push from standing at the spawn, by how long A is held; and after braking to a full stop."""
from rest import *
def rise(rows, t, n=80):
    v0 = hs3(rows[t]); top = max(hs3(rows[i]) for i in range(t, t + n))
    c = next((i - t for i in range(t, t + n) if hs3(rows[i]) > v0 + 0.05), None)
    return f"+{top - v0:4.2f}@{c}"
if __name__ == "__main__":
    for hold in (0, 1, 3, 6, 10, 20, 40):
        pr, sk = rest_pair(f"0 {hold} 0 0 0 0 0x1000\n", 90)
        print(f"spawn hold {hold:2d}: P {rise(pr, 0)}  S {rise(sk, 0)}")
    # held push to speed, brake till stopped, wait, then tap / hold
    for wait, hold in ((60, 3), (60, 30), (200, 3), (200, 30)):
        t = 120 + 150 + wait
        act = f"0 119 0 0 0 0 0x1000\n120 269 0 0 0 0 0x2000\n{t} {t+hold} 0 0 0 0 0x1000\n"
        pr, sk = rest_pair(act, t + 90)
        print(f"stopped(wait {wait}) hold {hold:2d}: P v{hs3(pr[t]):4.2f} {rise(pr, t)}  S v{hs3(sk[t]):4.2f} {rise(sk, t)}")
