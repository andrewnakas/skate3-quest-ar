from edge import *
S = feel.ON_RAIL
RX = -7.0
def caught(rows, port):
    for i, r in enumerate(rows):
        if (r["phys_state"].startswith("Grind") if port else r["mode"] == "Grind"):
            return i
    return None
if __name__ == "__main__":
  for s in [0, 0.15, 0.3, 0.45, 0.6, -0.15, -0.3, -0.45, -0.6]:
      act = f"0 8 {s} 0 0 -1 0\n9 15 0 0 0 1 0"
      pr, sk = pair(act, 110, S)
      out = []
      for rows, port in ((pr, True), (sk, False)):
          c = caught(rows, port)
          x17 = float(rows[17]["deck_px"]) - RX
          # lateral offset at the lowest point near the rail height, before catch
          lat = [round(float(rows[i]["deck_px"]) - RX, 2) for i in (30, 45, 55)]
          out.append(f"x@17 {x17:5.2f} x@30/45/55 {lat} catch {c}")
      print(f"steer {s:5.2f}  port {out[0]} | sk8 {out[1]}")
