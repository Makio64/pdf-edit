#!/usr/bin/env python3
"""Turns the raw screenshots of capture.js into the website's images (see README.md here).

raw/<shot>.png (2200 x 1440, 2x) -> ../src/img/<name>.webp, 1600 px wide (1600 x 1047), WebP
quality 82. The pages give every screenshot width="1600" height="1047": if a new capture has
another shape, this script says so and the pages must be changed with it.

CROPS adds close-ups for phones, cut from the raw shot at full resolution (the home page shows
app-corriger-dans-le-pdf-crop.webp below 561 px, where the whole window is too small to read).

Usage: python3 shots/convert.py [--raw <dir>] [--out <dir>]      (needs Pillow: pip3 install Pillow)
"""
import argparse
import os
import sys

try:
    from PIL import Image
except ImportError:
    sys.exit("Pillow is missing: python3 -m pip install --user Pillow")

HERE = os.path.dirname(os.path.abspath(__file__))
# capture.js name -> published name (src/img/<name>.webp). 5-editor and 6-typing are not shown.
NAMES = {
    "1-choose": "app-1-choisir",
    "2-action": "app-2-que-faire",
    "3-place": "app-3-ou-enregistrer",
    "4-done": "app-4-pret",
    "7-corrected": "app-corriger-dans-le-pdf",
    "8-compare": "app-comparer",
}
WIDTH, HEIGHT = 1600, 1047
QUALITY = 82
# capture.js name -> (published name, box in raw pixels: left, top, right, bottom). The box of
# 7-corrected is the letter's heading, the address and the retyped date, framed. The page gives
# the crop width="1306" height="825" (on its <source>): change both together.
CROPS = {
    "7-corrected": ("app-corriger-dans-le-pdf-crop", (360, 240, 1666, 1065)),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--raw", default=os.path.join(HERE, "raw"), help="capture.js output (default: shots/raw)")
    parser.add_argument("--out", default=os.path.join(HERE, "..", "src", "img"), help="default: site/src/img")
    args = parser.parse_args()
    out = os.path.normpath(args.out)
    os.makedirs(out, exist_ok=True)

    missing = [n for n in NAMES if not os.path.isfile(os.path.join(args.raw, n + ".png"))]
    if missing:
        sys.exit(f"Missing in {args.raw}: " + ", ".join(n + ".png" for n in missing) + " (run capture.js first)")

    other_shape = []
    for shot, name in NAMES.items():
        with Image.open(os.path.join(args.raw, shot + ".png")) as im:
            rgb = im.convert("RGB")
        height = round(rgb.height * WIDTH / rgb.width)
        small = rgb.resize((WIDTH, height), Image.LANCZOS)
        path = os.path.join(out, name + ".webp")
        small.save(path, "WEBP", quality=QUALITY, method=6)
        shown = os.path.relpath(path)
        print(f"{shot}.png -> {path if shown.startswith('..') else shown} ({WIDTH}x{height}, {os.path.getsize(path) // 1024} KB)")
        if height != HEIGHT:
            other_shape.append(f"{name}.webp is {WIDTH}x{height}")
    for shot, (name, box) in CROPS.items():
        with Image.open(os.path.join(args.raw, shot + ".png")) as im:
            if im.size != (2200, 1440):
                print(f"Warning: {shot}.png is {im.width}x{im.height}, not 2200x1440: check the box of {name} in CROPS",
                      file=sys.stderr)
            crop = im.convert("RGB").crop(box)
        path = os.path.join(out, name + ".webp")
        crop.save(path, "WEBP", quality=QUALITY, method=6)
        shown = os.path.relpath(path)
        print(f"{shot}.png -> {path if shown.startswith('..') else shown} ({crop.width}x{crop.height}, crop, "
              f"{os.path.getsize(path) // 1024} KB)")
    if other_shape:
        print("Warning: the pages give the screenshots width=\"1600\" height=\"1047\"; change them too: "
              + "; ".join(other_shape), file=sys.stderr)


if __name__ == "__main__":
    main()
