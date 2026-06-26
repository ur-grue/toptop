#!/usr/bin/env python3
"""Generate assets/banner.svg — a synthwave pixel-art banner for toptop.

Pure stdlib, deterministic. Re-run after tweaking to regenerate the asset:
    python3 scripts/make_banner.py
"""
import os

W, H = 880, 300
PX = 11              # pixel size for the title font
HORIZON = 206        # where the sky meets the grid

# 5x7 pixel font (only the glyphs we need).
FONT = {
    "T": ["11111", "00100", "00100", "00100", "00100", "00100", "00100"],
    "O": ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
    "P": ["11110", "10001", "10001", "11110", "10000", "10000", "10000"],
}
WORD = "TOPTOP"


def rects_for_title(x0, y0, fill='url(#title)', extra=''):
    out = []
    cx = x0
    for ch in WORD:
        for r, row in enumerate(FONT[ch]):
            for c, bit in enumerate(row):
                if bit == "1":
                    out.append(
                        f'<rect x="{cx + c*PX}" y="{y0 + r*PX}" width="{PX-1}" '
                        f'height="{PX-1}" rx="1.5" fill="{fill}"{extra}/>'
                    )
        cx += 6 * PX  # 5 cols + 1 gap
    return out


def title_width():
    return (len(WORD) * 6 - 1) * PX


def stars():
    pts = [(52, 52), (95, 92), (150, 38), (40, 124), (190, 150),
           (728, 44), (800, 80), (842, 120), (690, 150), (608, 34), (270, 28)]
    out = ['<g fill="#eaf7ff">']
    for i, (x, y) in enumerate(pts):
        s = 4 if i % 3 == 0 else 3
        op = 1.0 if i % 2 == 0 else 0.65
        out.append(f'<rect x="{x}" y="{y}" width="{s}" height="{s}" opacity="{op}"/>')
        if s == 4:  # plus-twinkle on the brighter stars
            out.append(f'<rect x="{x-2}" y="{y+1}" width="{s+4}" height="1" opacity="{op*0.5}"/>')
            out.append(f'<rect x="{x+1}" y="{y-2}" width="1" height="{s+4}" opacity="{op*0.5}"/>')
    out.append("</g>")
    return out


def sun():
    """A banded synthwave sun setting *behind* the horizon (clipped above it)."""
    cx, cy, r = W / 2, 190, 76
    out = [
        f'<clipPath id="sky-clip"><rect x="0" y="0" width="{W}" height="{HORIZON}"/></clipPath>',
        f'<clipPath id="sun-clip"><circle cx="{cx}" cy="{cy}" r="{r}"/></clipPath>',
        f'<g clip-path="url(#sky-clip)"><g clip-path="url(#sun-clip)">',
        f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="url(#sun)"/>',
    ]
    # Horizontal gap-bands, widening toward the bottom (classic Outrun look).
    y, gap = cy - 26, 3
    while y < HORIZON:
        out.append(f'<rect x="{cx-r}" y="{y:.1f}" width="{2*r}" height="{gap}" fill="#1b0934"/>')
        y += gap + 7
        gap += 1.4
    out.append("</g></g>")
    return out


def grid_lines():
    vp_x = W / 2
    out = ['<g stroke="#19e6ff" stroke-width="1.4" opacity="0.6">']
    for i in range(-9, 10):
        bx = vp_x + i * 96
        out.append(f'<line x1="{vp_x}" y1="{HORIZON}" x2="{bx:.1f}" y2="{H}"/>')
    y, step = float(H), 44.0
    while y > HORIZON:
        out.append(f'<line x1="0" y1="{y:.1f}" x2="{W}" y2="{y:.1f}"/>')
        y -= step
        step *= 0.62
    out.append("</g>")
    return out


def scanlines():
    out = ['<g stroke="#000000" stroke-width="1" opacity="0.08">']
    y = 0
    while y < H:
        out.append(f'<line x1="0" y1="{y}" x2="{W}" y2="{y}"/>')
        y += 3
    out.append("</g>")
    return out


def main():
    x0 = (W - title_width()) / 2
    y0 = 30

    svg = [
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="{W}" '
        f'height="{H}" font-family="ui-monospace,monospace" shape-rendering="crispEdges">',
        "<defs>",
        '<linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">'
        '<stop offset="0" stop-color="#120522"/><stop offset="0.7" stop-color="#26093f"/>'
        '<stop offset="1" stop-color="#3a0d5e"/></linearGradient>',
        '<linearGradient id="sun" x1="0" y1="0" x2="0" y2="1">'
        '<stop offset="0" stop-color="#ffe24a"/><stop offset="0.45" stop-color="#ff8a3d"/>'
        '<stop offset="1" stop-color="#ff2975"/></linearGradient>',
        f'<linearGradient id="title" gradientUnits="userSpaceOnUse" x1="{x0}" y1="0" '
        f'x2="{x0+title_width()}" y2="0">'
        '<stop offset="0" stop-color="#22f0ff"/><stop offset="0.5" stop-color="#8a6bff"/>'
        '<stop offset="1" stop-color="#ff3ca0"/></linearGradient>',
        # Fades the grid out toward the horizon for depth.
        '<linearGradient id="gridfade" x1="0" y1="0" x2="0" y2="1">'
        '<stop offset="0" stop-color="#26093f" stop-opacity="0.95"/>'
        '<stop offset="1" stop-color="#26093f" stop-opacity="0"/></linearGradient>',
        # Soft neon glow for the title.
        '<filter id="glow" x="-15%" y="-40%" width="130%" height="180%">'
        '<feGaussianBlur stdDeviation="3.2" result="b"/>'
        '<feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge></filter>',
        "</defs>",
        f'<rect width="{W}" height="{H}" fill="url(#sky)"/>',
    ]
    svg += stars()
    svg += sun()
    # Grid, then a horizon-ward fade, then the bright neon horizon line on top.
    svg += grid_lines()
    svg.append(f'<rect x="0" y="{HORIZON}" width="{W}" height="62" fill="url(#gridfade)"/>')
    svg.append(f'<rect x="0" y="{HORIZON-2}" width="{W}" height="3" fill="#5ff4ff" opacity="0.9"/>')

    # Title: magenta drop-glow copy, then the crisp gradient title with a soft blur glow.
    svg += rects_for_title(x0, y0 + 3, fill="#ff2e97", extra=' opacity="0.35"')
    svg.append('<g filter="url(#glow)">')
    svg += rects_for_title(x0, y0)
    svg.append("</g>")
    svg += scanlines()

    # Tagline — a clear word gap and a subtle backing strip.
    svg.append(f'<rect x="0" y="262" width="{W}" height="30" fill="#0c0418" opacity="0.66"/>')
    svg.append(
        f'<text x="{W/2}" y="283" fill="#a7f4ff" font-size="15" font-weight="bold" '
        f'letter-spacing="8" text-anchor="middle">S Y S T E M&#160;&#160;·&#160;&#160;M O N I T O R</text>'
    )
    svg.append("</svg>")

    os.makedirs("assets", exist_ok=True)
    with open("assets/banner.svg", "w") as f:
        f.write("\n".join(svg) + "\n")
    print("wrote assets/banner.svg")


if __name__ == "__main__":
    main()
