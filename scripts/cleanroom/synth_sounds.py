#!/usr/bin/env python3
"""Synthesise the clean tier's sounds that have no CC0 recording.

Every sample is generated here from noise, sines and filters (seeded, so the
output is reproducible); nothing is read from any game or recording. Writes
32 kHz mono 16-bit WAVs into crates/skate-game/clean-audio/ and records them
in its CREDITS.json as original synthesis.

Usage: scripts/cleanroom/synth_sounds.py
"""
import json
import wave
from pathlib import Path

import numpy as np

RATE = 32000
OUT = Path(__file__).resolve().parents[2] / "crates/skate-game/clean-audio"
rng = np.random.default_rng(20261017)


def t(seconds):
    return np.arange(int(seconds * RATE)) / RATE


def one_pole(x, cutoff):
    """Simple low-pass."""
    a = np.exp(-2 * np.pi * cutoff / RATE)
    y = np.empty_like(x)
    acc = 0.0
    for i, v in enumerate(x):
        acc = (1 - a) * v + a * acc
        y[i] = acc
    return y


def band(x, lo, hi):
    return one_pole(x, hi) - one_pole(x, lo)


def resonator(x, freq, q):
    """Two-pole resonant band-pass (a ringing mode)."""
    w = 2 * np.pi * freq / RATE
    r = np.exp(-w / (2 * q))
    b1, b2 = 2 * r * np.cos(w), -r * r
    y = np.zeros_like(x)
    y1 = y2 = 0.0
    for i, v in enumerate(x):
        out = (1 - r) * v + b1 * y1 + b2 * y2
        y2, y1 = y1, out
        y[i] = out
    return y


def normalise(x, peak):
    m = np.max(np.abs(x)) or 1.0
    return x * (peak / m)


def write(name, x, peak=0.6):
    x = normalise(x, peak)
    pcm = np.clip(np.round(x * 32767), -32768, 32767).astype("<i2")
    with wave.open(str(OUT / f"{name}.wav"), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(pcm.tobytes())
    return name


def flip(seed_shift, length, centre):
    """Board whoosh: band noise swelling and fading, with a faint spin flutter."""
    tt = t(length)
    noise = rng.standard_normal(tt.size)
    swish = band(noise, centre * 0.5, centre * 1.6)
    env = np.sin(np.pi * np.clip(tt / length, 0, 1)) ** 1.6
    flutter = 0.75 + 0.25 * np.sin(2 * np.pi * (11 + seed_shift * 3) * tt)
    return swish * env * flutter


def footstep(weight):
    """Shoe on concrete: a soft low thump plus a short grit scuff."""
    tt = t(0.22)
    thump = np.sin(2 * np.pi * (70 + 25 * weight) * tt) * np.exp(-tt * 38)
    grit = band(rng.standard_normal(tt.size), 900, 5200) * np.exp(-tt * (55 - 10 * weight))
    return thump * 0.9 + grit * (0.35 + 0.15 * weight)


def chime():
    """Bank chime: two bell partials a fifth apart, bright attack."""
    tt = t(0.9)
    out = np.zeros_like(tt)
    for f, a, d in [(1046.5, 1.0, 4.5), (1568.0, 0.6, 5.5), (2093.0, 0.25, 8.0), (3136.0, 0.12, 11.0)]:
        out += a * np.sin(2 * np.pi * f * tt) * np.exp(-tt * d)
    attack = np.clip(tt / 0.004, 0, 1)
    return out * attack


def roll_metal(seconds=2.5):
    """Wheels on a metal ramp: rumble, ringing plate modes and joint clicks; loops seamlessly."""
    n = int(seconds * RATE)
    noise = rng.standard_normal(n)
    rumble = band(noise, 60, 420) * 0.7
    ring = sum(resonator(noise * 0.02, f, 40) * g for f, g in [(611.0, 1.0), (1287.0, 0.7), (2333.0, 0.45)])
    clicks = np.zeros(n)
    for c in range(0, n, int(RATE * 0.31)):
        k = np.arange(min(300, n - c))
        clicks[c:c + k.size] += np.sin(2 * np.pi * 1900 * k / RATE) * np.exp(-k / 40) * 0.6
    x = rumble + ring * 3.0 + clicks
    # Crossfade the end into the start so the loop has no seam.
    fade = int(0.08 * RATE)
    w = np.linspace(0, 1, fade)
    x[:fade] = x[:fade] * w + x[-fade:] * (1 - w)
    return x[:-fade]


def main():
    made = []
    for i, (length, centre) in enumerate([(0.32, 1700.0), (0.27, 2300.0), (0.36, 1400.0)], 1):
        made.append(write(f"flip_{i}", flip(i, length, centre), 0.55))
    for i, weight in enumerate([0.2, 0.6, 0.4, 0.9], 1):
        made.append(write(f"footstep_{i}", footstep(weight), 0.5))
    made.append(write("chime", chime(), 0.5))
    made.append(write("roll_metal", roll_metal(), 0.35))

    credits_path = OUT / "CREDITS.json"
    credits = [c for c in json.loads(credits_path.read_text()) if c["sound"] not in made]
    for name in made:
        credits.append({
            "sound": name,
            "url": "scripts/cleanroom/synth_sounds.py",
            "author": "SK8 AR (original synthesis)",
            "license": "Original work, generated from noise and sine synthesis; no recorded or game audio",
        })
    credits.sort(key=lambda c: c["sound"])
    credits_path.write_text(json.dumps(credits, indent=1) + "\n")
    print(f"synthesised {len(made)} sounds into {OUT}")


if __name__ == "__main__":
    main()
