#!/usr/bin/env python3
"""App icon of « PDF Edit »: a white page whose ink-blue line is being
written by a pencil, on a calm ink-blue macOS (Big Sur grid) squircle.

Writes, into packaging/macos/:
  AppIcon.icns       every size from 16 to 1024 px (built with iconutil)
  AppIcon-1024.png   the 1024x1024 master
  icon/preview.png   contact sheet: 16/32/64/128 px on light and dark backgrounds

Usage:  python3 packaging/macos/icon/make_icon.py
Needs:  Python 3 with Pillow and numpy, and macOS `iconutil`.

Everything is drawn at 4x and reduced with LANCZOS. The 16, 32 and 64 px
renderings use a simplified, pixel-snapped layout (fewer, thicker lines,
a wider pencil) so they stay legible in the Finder and the Dock.
"""

import math
import os
import shutil
import subprocess
import sys
import tempfile

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
MACOS_DIR = os.path.dirname(HERE)

CANVAS = 1024          # design units: the 1024 px master
SS = 4                 # supersampling factor
BODY = (100, 100, 924, 924)   # macOS grid: 824x824 rounded square, centred
SQUIRCLE_N = 5.0       # superellipse exponent ~ Apple's continuous corners

COL = {
    "body_top": "#5d82bf",
    "body_bottom": "#2d4c82",
    "page_top": "#ffffff",
    "page_bottom": "#eef2f8",       # --accent-soft of the app
    "fold": "#d3dcea",
    "line": "#c2ccdb",
    "line_small": "#8d9fbb",        # darker grey lines for 16-64 px
    "ink": "#33558f",               # --accent of the app
    "shadow": "#0d1b36",
    # pencil
    "graphite": "#2b3142",
    "wood": "#f4dbb0",
    "wood_dark": "#e4c48e",
    "paint_light": "#f9d26a",
    "paint": "#f2b83d",
    "paint_dark": "#da9a25",
    "metal_light": "#e1e5ec",
    "metal": "#c3cad5",
    "metal_dark": "#a3acba",
    "ring": "#8e98a8",
    "eraser_light": "#f4a99c",
    "eraser": "#ea8f80",
    "eraser_dark": "#d27465",
}


def rgba(hex_color, alpha=255):
    h = hex_color.lstrip("#")
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), alpha)


# --------------------------------------------------------------------------
# Layouts (design units, 1024 canvas)
# --------------------------------------------------------------------------

def snap(v, unit):
    return round(v / unit) * unit


def layout_for(target_px):
    """Geometry for a given output size. Small sizes get a hinted variant."""
    if target_px >= 128:
        return {
            "page": (186, 196, 630, 812),
            "page_radius": 30,
            "fold": 118,
            "line_thick": 28,
            "line_color": COL["line"],
            # (y centre, x end, is_ink)
            "rows": [(344, 566, False), (416, 540, False), (488, 566, False),
                     (560, 520, False), (632, 456, True), (704, 404, False)],
            "line_x0": 250,
            "tip": (474, 636),
            "pencil_w": 92,
            "pencil": dict(cone=100, graphite=36, body=262, ferrule=52, eraser=60),
            "detail": True,
        }
    if target_px >= 64:
        u = CANVAS / target_px          # 16 design units = 1 px at 64
        t = 3 * u                       # 3 px lines
        return {
            "page": (snap(186, u), snap(196, u), snap(630, u), snap(812, u)),
            "page_radius": 2 * u,
            "fold": 8 * u,
            "line_thick": t,
            "line_color": COL["line_small"],
            "rows": [(20 * u + t / 2, 35 * u, False),
                     (25 * u + t / 2, 33 * u, False),
                     (30 * u + t / 2, 34 * u, False),
                     (35 * u + t / 2, 28 * u, True),
                     (40 * u + t / 2, 25 * u, False)],
            "line_x0": 16 * u,
            "tip": (29.5 * u, 35 * u + t / 2),
            "pencil_w": 116,
            "pencil": dict(cone=112, graphite=44, body=250, ferrule=50, eraser=62),
            "detail": False,
        }
    # 16 and 32 px: snap to the 16 px grid (64 units), 1 px lines at 16 px.
    u = 64
    t = 64
    return {
        "page": (3 * u, 3 * u, 10 * u, 13 * u),
        "page_radius": 24,
        "fold": 2 * u,
        "line_thick": t,
        "line_color": COL["line_small"],
        "rows": [(6 * u + t / 2, 8 * u, False),
                 (8 * u + t / 2, 8 * u, False),
                 (10 * u + t / 2, 6.5 * u, True)],
        "line_x0": 4 * u,
        "tip": (7 * u, 10 * u + t / 2),
        "pencil_w": 150,
        "pencil": dict(cone=150, graphite=60, body=230, ferrule=46, eraser=64),
        "detail": False,
    }


# --------------------------------------------------------------------------
# Drawing helpers (everything at supersampled resolution)
# --------------------------------------------------------------------------

def S(v):
    return v * SS


def box(b):
    return tuple(S(v) for v in b)


def squircle_alpha(size, rect, n):
    """Anti-aliased superellipse mask, `size` px square, rect in design units."""
    x0, y0, x1, y1 = rect
    scale = size / CANVAS
    cx, cy = (x0 + x1) / 2 * scale, (y0 + y1) / 2 * scale
    a, b = (x1 - x0) / 2 * scale, (y1 - y0) / 2 * scale
    coords = np.arange(size, dtype=np.float32) + 0.5
    X = np.abs(coords - cx) / a
    Y = np.abs(coords - cy) / b
    v = X[None, :] ** n + Y[:, None] ** n
    dist = (1.0 - v ** (1.0 / n)) * min(a, b)      # ~pixels inside the edge
    alpha = np.clip(dist + 0.5, 0.0, 1.0)
    return Image.fromarray((alpha * 255).astype(np.uint8), "L")


def vertical_gradient(size, y0, y1, top, bottom):
    """RGBA image (size x size) with a vertical gradient between rows y0..y1 (px)."""
    t = (np.arange(size, dtype=np.float32) - y0) / max(1.0, (y1 - y0))
    t = np.clip(t, 0.0, 1.0)[:, None]
    c0 = np.array(rgba(top), dtype=np.float32)[None, :]
    c1 = np.array(rgba(bottom), dtype=np.float32)[None, :]
    rows = (c0 * (1 - t) + c1 * t).round().astype(np.uint8)       # size x 4
    arr = np.empty((size, size, 4), dtype=np.uint8)
    arr[:] = rows[:, None, :]
    return Image.fromarray(arr, "RGBA")


def solid(size, color, mask):
    layer = Image.new("RGBA", size, rgba(color))
    layer.putalpha(mask)
    return layer


def shadow_from(mask, color, opacity, blur, dx=0, dy=0):
    """Soft shadow layer from an L mask (blur/offset in design units)."""
    m = mask
    if dx or dy:
        shifted = Image.new("L", mask.size, 0)
        shifted.paste(mask, (int(round(S(dx))), int(round(S(dy)))))
        m = shifted
    if blur:
        m = m.filter(ImageFilter.GaussianBlur(S(blur)))
    m = m.point(lambda p: int(p * opacity))
    return solid(mask.size, color, m)


def multiply_alpha(layer, mask):
    a = np.asarray(layer.getchannel("A"), dtype=np.uint16)
    m = np.asarray(mask, dtype=np.uint16)
    out = layer.copy()
    out.putalpha(Image.fromarray(((a * m + 127) // 255).astype(np.uint8), "L"))
    return out


# --------------------------------------------------------------------------
# Pieces
# --------------------------------------------------------------------------

def page_mask(size, L):
    x0, y0, x1, y1 = L["page"]
    f, r = L["fold"], L["page_radius"]
    m = Image.new("L", size, 0)
    d = ImageDraw.Draw(m)
    d.rounded_rectangle(box((x0, y0, x1, y1)), radius=S(r), fill=255)
    # cut the folded corner (a bit beyond the page to remove the rounding)
    d.polygon([(S(x1 - f), S(y0)), (S(x1), S(y0 + f)),
               (S(x1) + SS * 8, S(y0 + f)), (S(x1) + SS * 8, S(y0) - SS * 8),
               (S(x1 - f), S(y0) - SS * 8)], fill=0)
    return m


def fold_mask(size, L):
    x0, y0, x1, y1 = L["page"]
    f = L["fold"]
    r = min(L["page_radius"], f * 0.25)
    m = Image.new("L", size, 0)
    d = ImageDraw.Draw(m)
    # triangle with a softly rounded inner corner
    d.polygon([(S(x1 - f), S(y0)), (S(x1 - f), S(y0 + f - r)),
               (S(x1 - f + r), S(y0 + f)), (S(x1), S(y0 + f))], fill=255)
    d.pieslice((S(x1 - f), S(y0 + f - 2 * r), S(x1 - f + 2 * r), S(y0 + f)),
               90, 180, fill=255)
    return m


def draw_pencil(L):
    """Horizontal pencil (tip on the left) on its own layer; returns (layer, tip_xy)."""
    p = L["pencil"]
    w = L["pencil_w"]
    cone, graphite, body, ferrule, eraser = (p["cone"], p["graphite"], p["body"],
                                             p["ferrule"], p["eraser"])
    length = cone + body + ferrule + eraser
    pad = 20
    W, H = S(length + 2 * pad), S(w + 2 * pad)
    tip = (S(pad), S(pad + w / 2))
    cy = tip[1]
    hw = S(w / 2)
    X = lambda a: tip[0] + S(a)                       # noqa: E731

    layer = Image.new("RGBA", (int(W), int(H)), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    band = hw * 2 / 3                                 # three facets
    facets = [(cy - hw, cy - hw + band), (cy - hw + band, cy + hw - band),
              (cy + hw - band, cy + hw)]

    def bands(xa, xb, colors):
        for (ya, yb), c in zip(facets, colors):
            d.rectangle((xa, ya, xb, yb), fill=rgba(c))

    x_body = X(cone)
    x_ferrule = X(cone + body)
    x_eraser = X(cone + body + ferrule)
    x_end = X(length)

    # wood cone (two tones), body, ferrule, eraser
    d.polygon([tip, (x_body + SS, cy - hw), (x_body + SS, cy)], fill=rgba(COL["wood"]))
    d.polygon([tip, (x_body + SS, cy), (x_body + SS, cy + hw)], fill=rgba(COL["wood_dark"]))
    bands(x_body, x_ferrule, [COL["paint_light"], COL["paint"], COL["paint_dark"]])
    # scalloped paint edge biting into the wood
    if L["detail"]:
        for (ya, yb), c in zip(facets, [COL["paint_light"], COL["paint"], COL["paint_dark"]]):
            rx = S(cone) * 0.16
            d.ellipse((x_body - rx, ya, x_body + rx, yb), fill=rgba(c))
    bands(x_ferrule, x_eraser, [COL["metal_light"], COL["metal"], COL["metal_dark"]])
    if L["detail"]:
        for k in (1, 2):
            xr = x_ferrule + S(ferrule) * k / 3
            d.rectangle((xr - S(3), cy - hw, xr + S(3), cy + hw), fill=rgba(COL["ring"]))
    bands(x_eraser, x_end, [COL["eraser_light"], COL["eraser"], COL["eraser_dark"]])
    # graphite point
    gh = hw * graphite / cone
    d.polygon([tip, (X(graphite), cy - gh), (X(graphite), cy + gh)], fill=rgba(COL["graphite"]))

    # silhouette: cone + body with a rounded eraser end; clip everything to it
    sil = Image.new("L", layer.size, 0)
    sd = ImageDraw.Draw(sil)
    rr = S(min(w * 0.22, eraser * 0.5))
    sd.polygon([tip, (x_body + SS, cy - hw), (x_body + SS, cy + hw)], fill=255)
    sd.rounded_rectangle((x_body, cy - hw, x_end, cy + hw), radius=rr, fill=255)
    sd.rectangle((x_body, cy - hw, x_end - rr, cy + hw), fill=255)
    # sharpen the very tip slightly rounded rather than a hair-thin needle
    layer = multiply_alpha(layer, sil)
    return layer, tip


def place_rotated(canvas_size, layer, tip, angle_deg, target_xy):
    """Rotate `layer` CCW by angle around its centre (expand) and place its tip at target."""
    rot = layer.rotate(angle_deg, resample=Image.BICUBIC, expand=True)
    cx, cy = layer.size[0] / 2, layer.size[1] / 2
    ncx, ncy = rot.size[0] / 2, rot.size[1] / 2
    th = math.radians(angle_deg)
    dx, dy = tip[0] - cx, tip[1] - cy
    tx = ncx + dx * math.cos(th) + dy * math.sin(th)
    ty = ncy - dx * math.sin(th) + dy * math.cos(th)
    out = Image.new("RGBA", canvas_size, (0, 0, 0, 0))
    out.alpha_composite(rot, (int(round(S(target_xy[0]) - tx)), int(round(S(target_xy[1]) - ty))))
    return out


# --------------------------------------------------------------------------
# The icon
# --------------------------------------------------------------------------

def render_master(L):
    """1024x1024 RGBA icon for layout L (drawn at SS x, reduced with LANCZOS)."""
    N = CANVAS * SS
    size = (N, N)

    body_alpha = squircle_alpha(N, BODY, SQUIRCLE_N)
    body = vertical_gradient(N, S(BODY[1]), S(BODY[3]), COL["body_top"], COL["body_bottom"])
    body.putalpha(body_alpha)

    scene = body
    # page with its soft shadow
    pm = page_mask(size, L)
    scene.alpha_composite(shadow_from(pm, COL["shadow"], 0.34, 16, dy=14))
    px0, py0, px1, py1 = L["page"]
    page = vertical_gradient(N, S(py0), S(py1), COL["page_top"], COL["page_bottom"])
    page.putalpha(pm)
    scene.alpha_composite(page)

    # folded corner, with a little shadow cast on the page
    fm = fold_mask(size, L)
    fs = shadow_from(fm, COL["shadow"], 0.22, 7, dx=-5, dy=6)
    scene.alpha_composite(multiply_alpha(fs, pm))
    scene.alpha_composite(solid(size, COL["fold"], fm))

    # text lines; the ink-blue one is the line being corrected
    lines = Image.new("RGBA", size, (0, 0, 0, 0))
    ld = ImageDraw.Draw(lines)
    t = L["line_thick"]
    for y, xe, ink in L["rows"]:
        color = COL["ink"] if ink else L["line_color"]
        ld.rounded_rectangle(box((L["line_x0"], y - t / 2, xe, y + t / 2)),
                             radius=S(t / 2) if L["detail"] else S(t * 0.2),
                             fill=rgba(color))
    scene.alpha_composite(lines)

    # the pencil, writing the ink line, with its shadow on the page
    pencil, tip = draw_pencil(L)
    placed = place_rotated(size, pencil, tip, 45, L["tip"])
    pa = placed.getchannel("A")
    scene.alpha_composite(shadow_from(pa, COL["shadow"], 0.30, 10, dx=10, dy=14))
    scene.alpha_composite(placed)

    # keep everything inside the rounded square
    scene = multiply_alpha(scene, body_alpha)

    master = scene.resize((CANVAS, CANVAS), Image.LANCZOS)

    # outer drop shadow, inside the 100 px margin of the canvas
    a = master.getchannel("A")
    out = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    sh = Image.new("L", (CANVAS, CANVAS), 0)
    sh.paste(a, (0, 10))
    sh = sh.filter(ImageFilter.GaussianBlur(12)).point(lambda p: int(p * 0.32))
    out.alpha_composite(solid((CANVAS, CANVAS), "#000000", sh))
    out.alpha_composite(master)
    return out


def render(target_px, cache):
    key = "full" if target_px >= 128 else ("medium" if target_px >= 64 else "small")
    if key not in cache:
        cache[key] = render_master(layout_for(target_px))
    img = cache[key]
    if target_px == CANVAS:
        return img.copy()
    # The small layouts are snapped to the target pixel grid: an area average
    # (BOX) keeps their edges crisp, where LANCZOS would blur them over 2-3 px.
    resample = Image.LANCZOS if key == "full" else Image.BOX
    return img.resize((target_px, target_px), resample)


ICONSET = [  # (file name, pixels)
    ("icon_16x16.png", 16), ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32), ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128), ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256), ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512), ("icon_512x512@2x.png", 1024),
]


def contact_sheet(cache):
    sizes = [16, 32, 64, 128]
    zoom_sizes = [16, 32]
    pad = 24
    zoom = 6
    row_h = 128 + 2 * pad
    zoom_w = sum(s * zoom for s in zoom_sizes) + pad * (len(zoom_sizes) + 1)
    width = sum(sizes) + pad * (len(sizes) + 1) + zoom_w
    height = max(row_h, max(s * zoom for s in zoom_sizes) + 2 * pad)
    sheet = Image.new("RGBA", (width, height * 2), (0, 0, 0, 0))
    for row, bg in enumerate(["#f2f2f2", "#1e1e1e"]):
        band = Image.new("RGBA", (width, height), rgba(bg))
        x = pad
        for s in sizes:
            band.alpha_composite(render(s, cache), (x, (height - s) // 2))
            x += s + pad
        x += pad
        for s in zoom_sizes:
            big = render(s, cache).resize((s * zoom, s * zoom), Image.NEAREST)
            band.alpha_composite(big, (x, (height - s * zoom) // 2))
            x += s * zoom + pad
        sheet.alpha_composite(band, (0, row * height))
    return sheet


def main():
    cache = {}
    master = render(CANVAS, cache)
    master.save(os.path.join(MACOS_DIR, "AppIcon-1024.png"), optimize=True)

    tmp = tempfile.mkdtemp(prefix="appicon-")
    try:
        iconset = os.path.join(tmp, "AppIcon.iconset")
        os.mkdir(iconset)
        for name, px in ICONSET:
            render(px, cache).save(os.path.join(iconset, name), optimize=True)
        icns = os.path.join(MACOS_DIR, "AppIcon.icns")
        subprocess.run(["iconutil", "-c", "icns", iconset, "-o", icns], check=True)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    contact_sheet(cache).save(os.path.join(HERE, "preview.png"), optimize=True)
    print("wrote", os.path.join(MACOS_DIR, "AppIcon.icns"))
    print("wrote", os.path.join(MACOS_DIR, "AppIcon-1024.png"))
    print("wrote", os.path.join(HERE, "preview.png"))


if __name__ == "__main__":
    sys.exit(main())
