# App icon « PDF Edit »

`make_icon.py` draws the icon (white page, ink-blue line being written by a pencil, on the ink-blue macOS squircle) and writes `../AppIcon.icns`, `../AppIcon-1024.png` and `preview.png` (16/32/64/128 px on light and dark backgrounds, plus 16/32 px enlarged).

Regenerate from the repository root: `python3 packaging/macos/icon/make_icon.py` (needs Python 3 with Pillow and numpy, and macOS `iconutil`; the temporary `.iconset` is deleted automatically).
Colours and geometry are at the top of the script; 16, 32 and 64 px use a simplified, pixel-snapped layout.
