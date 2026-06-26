#!/usr/bin/env python3
"""Generate assets/banner.svg — a synthwave pixel-art banner for toptop.

Pure stdlib, deterministic. Re-run after tweaking to regenerate the asset:
    python3 scripts/make_banner.py
"""
import os

W, H = 880, 300
PX = 13  # pixel size for the title font

# 5x7 pixel font (only the glyphs we need).
FONT = {
    "T": ["11111", "00100", "00100", "00100", "00100", "00100", "00100"],
    "O": ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
    "P": ["11110", "10001", "10001", "11110", "10000", "10000", "10000"],
}
WORD = "TOPTOP"


def rects_for_title(x0, y0):
    cols = len(WORD) * 5 + (len(WORD) - 1)  # 1-col gap between letters
    width = cols * PX
    out = []
    cx = x0
    for ch in WORD:
        glyph = FONT[ch]
        for r, row in enumerate(glyph):
            for c, bit in enumerate(row):
                if bit == "1":
                    x = cx + c * PX
                    y = y0 + r * PX
                    out.append(
                        f'<rect x="{x}" y="{y}" width="{PX-1}" height="{PX-1}" '
                        f'rx="1" fill="url(#title)"/>'
                    )
        cx += 6 * PX  # 5 cols + 1 gap
    return out, width


def grid_lines():
    """A neon perspective floor in the lower third."""
    vp_x, vp_y = W / 2, 175          # vanishing point
    horizon, bottom = 205, H
    out = ['<g stroke="#00f0ff" stroke-width="1.5" opacity="0.55">']
    # Verticals converging to the vanishing point.
    for i in range(-9, 10):
        bx = vp_x + i * 95
        out.append(f'<line x1="{vp_x}" y1="{horizon}" x2="{bx:.1f}" y2="{bottom}"/>')
    # Horizontals getting denser toward the horizon.
    y = bottom
    step = 46.0
    while y > horizon:
        out.append(f'<line x1="0" y1="{y:.1f}" x2="{W}" y2="{y:.1f}"/>')
        y -= step
        step *= 0.62
    out.append("</g>")
    return out


def sun():
    """Classic banded synthwave sun behind the title."""
    cx, cy, r = W / 2, 150, 92
    out = [f'<clipPath id="sun"><circle cx="{cx}" cy="{cy}" r="{r}"/></clipPath>']
    out.append(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="url(#sun)"/>')
    out.append(f'<g clip-path="url(#sun)">')
    # Horizontal cut-out bands, widening toward the bottom.
    y = cy + 8
    gap = 5
    while y < cy + r:
        out.append(
            f'<rect x="{cx-r}" y="{y:.1f}" width="{2*r}" height="{gap}" fill="#150726"/>'
        )
        y += gap + 6
        gap += 2
    out.append("</g>")
    return out


def scanlines():
    out = ['<g stroke="#000000" stroke-width="1" opacity="0.10">']
    y = 0
    while y < H:
        out.append(f'<line x1="0" y1="{y}" x2="{W}" y2="{y}"/>')
        y += 3
    out.append("</g>")
    return out


def main():
    title_cols = len(WORD) * 6 - 1
    title_w = title_cols * PX
    x0 = (W - title_w) / 2
    y0 = 42
    title_rects, real_w = rects_for_title(x0, y0)

    svg = []
    svg.append(
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" '
        f'width="{W}" height="{H}" font-family="monospace" '
        f'shape-rendering="crispEdges">'
    )
    svg.append("<defs>")
    svg.append(
        '<linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">'
        '<stop offset="0" stop-color="#150726"/>'
        '<stop offset="0.65" stop-color="#2a0a4a"/>'
        '<stop offset="1" stop-color="#3a0d5e"/></linearGradient>'
    )
    svg.append(
        '<linearGradient id="sun" x1="0" y1="0" x2="0" y2="1">'
        '<stop offset="0" stop-color="#ffd319"/>'
        '<stop offset="0.5" stop-color="#ff7847"/>'
        '<stop offset="1" stop-color="#ff2975"/></linearGradient>'
    )
    svg.append(
        f'<linearGradient id="title" gradientUnits="userSpaceOnUse" '
        f'x1="{x0}" y1="0" x2="{x0+real_w}" y2="0">'
        '<stop offset="0" stop-color="#00f0ff"/>'
        '<stop offset="0.5" stop-color="#7b61ff"/>'
        '<stop offset="1" stop-color="#ff2e97"/></linearGradient>'
    )
    svg.append("</defs>")

    svg.append(f'<rect width="{W}" height="{H}" fill="url(#sky)"/>')
    svg += sun()
    svg += grid_lines()
    # Neon backing glow: an offset, dim-magenta copy of the title.
    glow_rects, _ = rects_for_title(x0, y0 + 3)
    glow_rects = [
        r.replace('fill="url(#title)"', 'fill="#ff2e97" opacity="0.40"')
        for r in glow_rects
    ]
    svg += glow_rects
    # Crisp gradient title on top.
    svg += title_rects
    svg += scanlines()

    # Tagline with a dark backing strip for guaranteed contrast over the grid.
    svg.append(f'<rect x="0" y="262" width="{W}" height="26" fill="#0d0418" opacity="0.78"/>')
    svg.append(
        f'<text x="{W/2}" y="280" fill="#9bf6ff" font-size="14" '
        f'letter-spacing="3" text-anchor="middle">'
        f'S Y S T E M   M O N I T O R  ·  htop power · btop looks · built in Rust · local-LLM brain</text>'
    )
    svg.append("</svg>")

    os.makedirs("assets", exist_ok=True)
    with open("assets/banner.svg", "w") as f:
        f.write("\n".join(svg) + "\n")
    print("wrote assets/banner.svg")


if __name__ == "__main__":
    main()
