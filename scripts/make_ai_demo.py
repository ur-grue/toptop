#!/usr/bin/env python3
"""Generate assets/ai-demo.svg — an animated (SMIL) demo of toptop's AI view.

GitHub renders SMIL-animated SVGs inline, so this acts as the hero "GIF":
tokens/sec tick, the memory-bandwidth bar pulses, the VRAM meter fills to red,
and the spill alert flashes. Re-run after edits:
    python3 scripts/make_ai_demo.py
"""
import os

W, H = 760, 396
# Palette mirrors toptop's default `gruvbox` theme (src/theme.rs THEMES[0]) so
# the hero graphic shows the colors users actually see on first launch.
BG, BORDER = "#282828", "#fe8019"    # dark bg, accent-orange border
TEXT, DIM = "#ebdbb2", "#928374"      # body text, dimmed labels
ACCENT, AQUA = "#fe8019", "#83a598"   # titles/headings (orange), secondary (aqua)
GREEN, YELLOW, RED = "#b8bb26", "#fabd2f", "#fb4934"  # load-gradient stops
TRACK = "#3c3836"                     # meter track (selection bg)

BAR_X, BAR_W, BAR_H = 188, 372, 16
VAL_X = BAR_X + BAR_W + 14


def text(x, y, s, fill=TEXT, size=17, weight="normal", anchor="start", extra=""):
    return (f'<text x="{x}" y="{y}" fill="{fill}" font-size="{size}" '
            f'font-weight="{weight}" text-anchor="{anchor}" '
            f'font-family="ui-monospace,Menlo,Consolas,monospace" {extra}>{s}</text>')


def bar(y, frac, color, animate_w=None, animate_fill=None):
    """A meter: dark track + colored fill, optionally animated."""
    out = [f'<rect x="{BAR_X}" y="{y}" width="{BAR_W}" height="{BAR_H}" rx="3" fill="{TRACK}"/>']
    w = int(BAR_W * frac)
    anim = ""
    if animate_w is not None:
        vals = ";".join(str(int(BAR_W * f)) for f in animate_w[0])
        anim += (f'<animate attributeName="width" values="{vals}" '
                 f'dur="{animate_w[1]}s" keyTimes="{animate_w[2]}" '
                 f'calcMode="spline" keySplines="{animate_w[3]}" repeatCount="indefinite"/>')
    fill = f'fill="{color}"'
    if animate_fill is not None:
        anim += (f'<animate attributeName="fill" values="{animate_fill[0]}" '
                 f'dur="{animate_fill[1]}s" keyTimes="{animate_fill[2]}" repeatCount="indefinite"/>')
    out.append(f'<rect x="{BAR_X}" y="{y}" width="{w}" height="{BAR_H}" rx="3" {fill}>{anim}</rect>')
    return "\n".join(out)


def ticking(x, y, frames, dur=3.0):
    """Stacked text frames whose opacity cycles to fake a changing number."""
    n = len(frames)
    keytimes = ";".join(f"{i/n:.4f}" for i in range(n))
    out = []
    for i, f in enumerate(frames):
        vals = ";".join("1" if k == i else "0" for k in range(n))
        anim = (f'<animate attributeName="opacity" values="{vals}" '
                f'keyTimes="{keytimes}" calcMode="discrete" dur="{dur}s" '
                f'repeatCount="indefinite"/>')
        out.append(text(x, y, f, fill=GREEN, size=20, weight="bold",
                        extra=f'opacity="0"').replace("</text>", anim + "</text>"))
    return "\n".join(out)


def main():
    s = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" '
         f'height="{H}" font-family="ui-monospace,monospace">']
    # Panel.
    s.append(f'<rect x="2" y="2" width="{W-4}" height="{H-4}" rx="12" fill="{BG}" '
             f'stroke="{BORDER}" stroke-width="2"/>')
    # Title bar — the real view is a bordered panel with no window chrome.
    s.append(text(W/2, 36, "AI · local-LLM GPU view · Esc/a to close", fill=ACCENT, size=17,
                  weight="bold", anchor="middle"))

    # Alert banner (flashing).
    s.append(text(28, 74, "⚠ ALERTS", fill=RED, size=17, weight="bold")
             .replace("</text>",
                      '<animate attributeName="opacity" values="1;0.35;1" dur="1.1s" '
                      'repeatCount="indefinite"/></text>'))
    s.append(text(120, 74, "[warn] gpu0 VRAM 96% — risk of spilling to RAM (5–20× slower)",
                  fill=TEXT, size=15))

    # GPU line.
    s.append(text(28, 112, "gpu0  NVIDIA GeForce RTX 4090", fill=AQUA, size=17, weight="bold"))
    s.append(text(W-28, 112, "312 / 450 W    71°C", fill=DIM, size=15, anchor="end"))

    # compute (static), mem b/w (pulsing), vram (filling to red).
    s.append(text(40, 152, "compute", fill=DIM, size=16))
    s.append(bar(140, 0.31, GREEN))
    s.append(text(VAL_X, 152, "31%", fill=GREEN, size=16))

    s.append(text(40, 188, "mem b/w", fill=DIM, size=16))
    s.append(bar(176, 0.78,
                 ACCENT,
                 animate_w=([0.70, 0.86, 0.74, 0.82, 0.70], 3.6,
                            "0;0.25;0.5;0.75;1", ".4 0 .6 1;.4 0 .6 1;.4 0 .6 1;.4 0 .6 1")))
    s.append(text(VAL_X, 188, "← bottleneck", fill=RED, size=15, weight="bold"))

    s.append(text(40, 224, "vram", fill=DIM, size=16))
    s.append(bar(212, 0.84,
                 GREEN,
                 animate_w=([0.84, 0.97, 0.97, 0.84], 6.0, "0;0.5;0.85;1",
                            ".5 0 .5 1;.5 0 .5 1;.5 0 .5 1"),
                 animate_fill=(f"{GREEN};{YELLOW};{RED};{RED};{GREEN}", 6.0, "0;0.35;0.5;0.85;1")))
    s.append(text(VAL_X, 224, "/ 24 GiB", fill=DIM, size=15))

    # Spill warning fades in as VRAM peaks.
    s.append(text(BAR_X, 252, "⚠ near VRAM limit — models will spill to system RAM", fill=RED, size=14)
             .replace("</text>",
                      '<animate attributeName="opacity" values="0.15;0.15;1;1;0.15" '
                      'keyTimes="0;0.35;0.5;0.85;1" dur="6s" repeatCount="indefinite"/></text>'))

    # Server + ticking tokens/sec.
    s.append(f'<line x1="28" y1="278" x2="{W-28}" y2="278" stroke="{TRACK}" stroke-width="1"/>')
    s.append(text(28, 308, "Inference servers (auto-discovered)", fill=ACCENT, size=16, weight="bold"))
    s.append(text(44, 336, "vLLM :8000", fill=AQUA, size=17, weight="bold"))
    s.append(text(160, 336, "meta-llama/Llama-3-8B", fill=DIM, size=15))
    s.append(ticking(44, 372, ["80.6", "82.3", "84.1", "85.0", "83.2", "81.4"]))
    s.append(text(132, 372, "tok/s", fill=GREEN, size=16))
    s.append(text(220, 372, "·  0.29 tok/s/W   ·   kv 64%   ·   req 2/5   ·   ttft 180 ms",
                  fill=DIM, size=15))
    # Blinking cursor.
    s.append(f'<rect x="{W-40}" y="360" width="10" height="18" fill="{AQUA}">'
             '<animate attributeName="opacity" values="1;1;0;0" dur="1s" '
             'repeatCount="indefinite"/></rect>')

    s.append("</svg>")
    os.makedirs("assets", exist_ok=True)
    with open("assets/ai-demo.svg", "w") as f:
        f.write("\n".join(s) + "\n")
    print("wrote assets/ai-demo.svg")


if __name__ == "__main__":
    main()
