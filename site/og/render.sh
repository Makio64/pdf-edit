#!/usr/bin/env bash
# Renders the social images of the website from site/og/og.html (1200 × 630 PNG, headless Chrome):
# site/src/og-<code>.png for every dictionary of site/i18n with its tagline (og.title, og.sub, og.chip1-3),
# and, only when asked (fr), site/src/og.png. The pages of each language use their own image; the build
# refuses a language whose image is missing. Look at every image before committing it.
#
# Usage: site/og/render.sh            every language but French
#        site/og/render.sh en es      only these languages (fr: site/src/og.png)
#        CHROME=/path/to/chrome site/og/render.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SITE="$(dirname "$HERE")"
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
die() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}
[[ -x "$CHROME" ]] || die "Google Chrome not found at $CHROME (set CHROME=<path>)"
command -v python3 >/dev/null 2>&1 || die "python3 not found (xcode-select --install)"

codes=("$@")
if [[ ${#codes[@]} -eq 0 ]]; then
  for f in "$SITE"/i18n/*.json; do
    code="$(basename "$f" .json)"
    [[ "$code" == fr ]] || codes+=("$code")
  done
fi
[[ ${#codes[@]} -gt 0 ]] || die "no dictionary in $SITE/i18n"

PROFILE="$(mktemp -d "${TMPDIR:-/tmp}/pdf-edit-og.XXXXXX")"
trap 'rm -rf "$PROFILE"' EXIT

for code in "${codes[@]}"; do
  dict="$SITE/i18n/$code.json"
  [[ -f "$dict" ]] || die "no dictionary $dict"
  # The tagline as a query string (the French image is og.html as it is).
  query="$(python3 -I -c '
import json, sys, urllib.parse
d = json.load(open(sys.argv[1], encoding="utf-8"))
keys = ["og.title", "og.sub", "og.chip1", "og.chip2", "og.chip3"]
missing = [k for k in keys if not isinstance(d.get(k), str) or not d[k].strip()]
if missing:
    sys.exit(sys.argv[1] + ": missing " + ", ".join(missing))
if any("|" in d[k] for k in keys[2:]):
    sys.exit(sys.argv[1] + ": a chip may not contain |")
print(urllib.parse.urlencode({"lang": d.get("lang", ""), "title": d["og.title"], "sub": d["og.sub"],
                              "chips": "|".join(d[k] for k in keys[2:])}, quote_via=urllib.parse.quote))
' "$dict")"
  if [[ "$code" == fr ]]; then
    out="$SITE/src/og.png"
    url="file://$HERE/og.html"
  else
    out="$SITE/src/og-$code.png"
    url="file://$HERE/og.html?$query"
  fi
  rm -f "$out"
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 --window-size=1200,630 \
    --user-data-dir="$PROFILE" --virtual-time-budget=3000 --screenshot="$out" "$url" >/dev/null 2>&1 || true
  [[ -s "$out" ]] || die "Chrome wrote no $out"
  if command -v sips >/dev/null 2>&1; then
    size="$(sips -g pixelWidth -g pixelHeight "$out" | awk '/pixel(Width|Height)/ {printf "%s ", $2}')"
    [[ "$size" == "1200 630 " ]] || die "$out is ${size% }, not 1200 630"
  fi
  printf '    %s\n' "${out#"$SITE"/}"
done
