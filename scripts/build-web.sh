#!/usr/bin/env bash
# Assemble the PDF Edit website in site/dist: the public pages of site/src rendered in every language
# of site/i18n (French at /, /en-ligne/, /questions/, /mentions-legales/; English at /en/, /en/online/…;
# one 404.html; /stats/ in French), the in-browser app under app/ (the pages of extension/ui and the
# WASM engine, shown framed in /en-ligne/ and its translations, with anonymous counts), and the
# release facts the download function answers with. See docs/WEB.md.
#
# Usage: scripts/build-web.sh [options]      (--help lists them)
#        pnpm web:build      a preview: the working tree's pages, the example release facts
#        a release:          scripts/build-web.sh --ui-rev HEAD --dmg dist/PDF-Edit-<version>.dmg
#        some languages:     LANGS=es,pt scripts/build-web.sh …   (French and English are always built)
#        then:               cd site && netlify deploy --prod --dir dist --functions netlify/functions
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/build-web.sh [options]

Builds the PDF Edit website into site/dist, ready for:
  cd site && netlify deploy --prod --dir dist --functions netlify/functions

The pages are the templates of site/src (/, /en-ligne/, /questions/, /mentions-legales/, /404.html)
filled in with the dictionaries of site/i18n (<code>.json), one copy per language (French at the
root, the others under /<code>/), plus /stats/ (French). The in-browser app goes to app/: /en-ligne/
shows it in a frame (<iframe src="/app/">); opened on its own, /app/ sends the visitor to /en-ligne/.
The build stops on a dictionary that lacks a key, a broken link, an image that is missing or has no
width/height, a page that is not counted, hreflang links that do not answer each other, an FAQ
whose structured data differs from the page, or an incomplete sitemap.

Options:
  --ui-rev <rev>          Take the app pages (extension/ui) and, without --pkg, the engine from this
                          git revision (default: the working tree, uncommitted changes included)
  --pkg <dir>             Prebuilt wasm-bindgen output (p2d_wasm.js + p2d_wasm_bg.wasm) instead of
                          building p2d-wasm with cargo, wasm-bindgen and wasm-opt
  --dmg <file>            The notarized release disk image, PDF-Edit-<version>.dmg (or the older
                          Modifier-mes-PDF-<version>.dmg): version, size and SHA-256 come from it.
                          Without it the download facts are those of netlify/lib/release.example.mts
  --site-url <url>        Public address, no trailing slash (default: https://pdf-edit-mac.netlify.app)
  --github-public         The GitHub repository is public: keep the <!--IF_GITHUB--> blocks
                          (same as GITHUB_PUBLIC=1; by default they are removed)
  --dmg-hosting site|github
                          Where /telecharger/mac sends to: the DMG copied into dist/downloads/ (site),
                          or the GitHub release v<version> (github, needs --github-public).
                          Default: github with --github-public, site otherwise
  --out <dir>             Output folder (default: site/dist). Emptied first
  --langs <codes>         The languages to build, comma-separated (e.g. es,pt): French and English are
                          always built. Default: every dictionary in site/i18n (same as LANGS=…)
  --site-dir <dir>        The site sources (src/, i18n/, app-overlay/, netlify/lib/; default: site/).
                          For tests on a copy; the build writes netlify/lib/release.generated.mts there
                          (and, without --out, the site into <dir>/dist)
  -h, --help              Show this help

Environment:
  GITHUB_PUBLIC=1         Same as --github-public
  LANGS=es,pt             Same as --langs es,pt
  CARGO_TARGET_DIR        Cargo's target folder; the engine is built in <it>/build-web (default target/)
EOF
}

step() { printf '\n==> %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
warn() { printf 'Warning: %s\n' "$*" >&2; }
die() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}

# ---------------------------------------------------------------------------------------------
# Options
# ---------------------------------------------------------------------------------------------

UI_REV=""
PKG=""
DMG=""
SITE_URL="https://pdf-edit-mac.netlify.app"
GITHUB_PUBLIC="${GITHUB_PUBLIC:-0}"
DMG_HOSTING=""
OUT=""
SITE_DIR=""
LANGS="${LANGS:-}"

need_value() {
  if [[ $# -lt 2 || -z "$2" || "$2" == --* ]]; then
    die "$1 needs a value (see --help)"
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ui-rev | --pkg | --dmg | --site-url | --dmg-hosting | --out | --site-dir | --langs)
      need_value "$@"
      case "$1" in
        --ui-rev) UI_REV="$2" ;;
        --pkg) PKG="$2" ;;
        --dmg) DMG="$2" ;;
        --site-url) SITE_URL="$2" ;;
        --dmg-hosting) DMG_HOSTING="$2" ;;
        --out) OUT="$2" ;;
        --site-dir) SITE_DIR="$2" ;;
        --langs) LANGS="$2" ;;
      esac
      shift 2
      ;;
    --ui-rev=* | --pkg=* | --dmg=* | --site-url=* | --dmg-hosting=* | --out=* | --site-dir=* | --langs=*)
      value="${1#*=}"
      [[ -n "$value" ]] || die "${1%%=*} needs a value (see --help)"
      case "$1" in
        --ui-rev=*) UI_REV="$value" ;;
        --pkg=*) PKG="$value" ;;
        --dmg=*) DMG="$value" ;;
        --site-url=*) SITE_URL="$value" ;;
        --dmg-hosting=*) DMG_HOSTING="$value" ;;
        --out=*) OUT="$value" ;;
        --site-dir=*) SITE_DIR="$value" ;;
        --langs=*) LANGS="$value" ;;
      esac
      shift
      ;;
    --github-public)
      GITHUB_PUBLIC=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      usage >&2
      die "unknown option: $1"
      ;;
  esac
done

case "$GITHUB_PUBLIC" in
  1 | true | yes) GITHUB_PUBLIC=1 ;;
  0 | false | no | "") GITHUB_PUBLIC=0 ;;
  *) die "GITHUB_PUBLIC must be 1 or 0 (got \"$GITHUB_PUBLIC\")" ;;
esac
if [[ -z "$DMG_HOSTING" ]]; then
  if [[ $GITHUB_PUBLIC -eq 1 ]]; then DMG_HOSTING=github; else DMG_HOSTING=site; fi
fi
case "$DMG_HOSTING" in
  site | github) ;;
  *) die "--dmg-hosting must be site or github (got \"$DMG_HOSTING\")" ;;
esac
if [[ "$DMG_HOSTING" == github && $GITHUB_PUBLIC -ne 1 ]]; then
  die "--dmg-hosting github sends visitors to a GitHub release, which nobody can open while the
  repository is private. Add --github-public once it is public, or keep --dmg-hosting site."
fi

SITE_URL="${SITE_URL%/}"
site_url_re='^https?://[A-Za-z0-9.-]+(:[0-9]+)?$'
[[ "$SITE_URL" =~ $site_url_re ]] ||
  die "--site-url must be an address such as https://pdf-edit-mac.netlify.app, with no path (got \"$SITE_URL\")"

command -v python3 >/dev/null 2>&1 || die "python3 not found (it comes with Apple's command line tools: xcode-select --install)"
# Paths given on the command line are relative to where the script was started: make them
# absolute before moving to the repository. (No symlink resolution: the path may not exist yet.)
abspath() { python3 -I -c 'import os, sys; print(os.path.abspath(sys.argv[1]))' "$1"; }
if [[ -n "$PKG" ]]; then PKG="$(abspath "$PKG")"; fi
if [[ -n "$DMG" ]]; then DMG="$(abspath "$DMG")"; fi
if [[ -n "$OUT" ]]; then OUT="$(abspath "$OUT")"; fi
if [[ -n "$SITE_DIR" ]]; then SITE_DIR="$(abspath "$SITE_DIR")"; fi

# ---------------------------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------------------------

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

SITE_DIR="${SITE_DIR:-$REPO/site}"
SRC="$SITE_DIR/src"
I18N="$SITE_DIR/i18n"
VALIDATE_MTS="$SITE_DIR/netlify/lib/validate.mts"
OVERLAY="$SITE_DIR/app-overlay/wasm-client.js"
RELEASE_EXAMPLE="$SITE_DIR/netlify/lib/release.example.mts"
RELEASE_GENERATED="$SITE_DIR/netlify/lib/release.generated.mts"
OUT="${OUT:-$SITE_DIR/dist}"
APP_OUT="$OUT/app"
GITHUB_URL="https://github.com/Makio64/pdf-edit"
DOWNLOAD_URL="/telecharger/mac"
UI_FILES=(drop.html drop.css drop.js edit.html edit.css edit.js wasm-client.js)
BUILD_DATE="$(date -u +%Y-%m-%d)"

WORK=""
TOUCHED_OUT=0
on_exit() {
  local rc=$?
  if [[ -n "$WORK" ]]; then
    rm -rf "$WORK"
  fi
  if [[ $rc -ne 0 && $TOUCHED_OUT -eq 1 ]]; then
    printf '\nThe build stopped before the end: what is in %s is incomplete. Do not deploy it.\n' "$OUT" >&2
  fi
}
trap on_exit EXIT
tmp_root="${TMPDIR:-/tmp}"
WORK="$(mktemp -d "${tmp_root%/}/pdf-edit-web.XXXXXX")"

# The page rewriting, the placeholders and the checks: in Python rather than sed, whose in-place
# and multi-line behaviour differs between macOS and Linux.
cat >"$WORK/build_web.py" <<'PY'
import json, os, re, sys

def fail(msg):
    sys.stderr.write(f"Error: {msg}\n")
    sys.exit(1)

def warn(msg):
    sys.stderr.write(f"Warning: {msg}\n")

def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()

def write(path, text):
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)

APP_TITLE = "PDF Edit en ligne — modifier ou convertir un PDF dans le navigateur"
TRACK_TAG = '<script type="module" src="/assets/track.js"></script>'

# The app is shown inside /en-ligne/ (an <iframe src="/app/"> under the site's menu). Opened on its
# own, its first screen (/app/, /app/index.html, /app/drop.html, or /app/drop, Netlify's address
# without the extension) goes to /en-ligne/ with the same query and hash; the editor (edit.html)
# stays where it is. Before leaving, the page says data-no-pageview (track.js sends nothing from a
# page that is going away) and no-referrer, so /en-ligne/ arrives without a referrer and counts an
# entry ("direct") instead of an internal visit from /app/. Framed, <html> gets the class in-frame:
# the site's header above it already shows the brand, so the app's own brand link is hidden (the
# style of head_block).
FRAME_SCRIPT = (
    "<script>(function(d){try{if(top===self){"
    "if(/\\/app\\/(index\\.html|drop(\\.html)?)?$/.test(location.pathname)){"
    "d.setAttribute('data-no-pageview','');"
    "var m=document.createElement('meta');m.name='referrer';m.content='no-referrer';document.head.appendChild(m);"
    "location.replace('/en-ligne/'+location.search+location.hash)}"
    "}else d.classList.add('in-frame')}catch(e){}})(document.documentElement)</script>"
)

def head_block(kind):
    # The same screens as /en-ligne/, which is the page search engines list.
    lines = [
        '<meta name="robots" content="noindex" />',
        '<link rel="canonical" href="{{SITE_URL}}/en-ligne/" />',
        '<link rel="icon" href="/favicon.svg" type="image/svg+xml" />',
        '<link rel="icon" href="/favicon-32.png" sizes="32x32" type="image/png" />',
        '<link rel="apple-touch-icon" href="/apple-touch-icon.png" />',
        '<link rel="manifest" href="/site.webmanifest" />',
    ]
    # The header brand becomes a link home that looks like the text it replaces. Opened on its own
    # (only the editor, in practice) on a phone, where the app hides its brand (drop.css, max-width
    # 560px), the link stays, centred, as the way back to the site. In the frame of /en-ligne/ the
    # site's header is the way back: the brand is hidden and the phone layout is the app's own.
    style = (
        "a.brand{color:inherit;text-decoration:none;border-radius:6px}"
        "a.brand:focus-visible{outline:4px solid var(--accent,#33558f);outline-offset:3px}"
        "@media (max-width:560px){html:not(.in-frame) .top{grid-template-columns:1fr auto 1fr}"
        "html:not(.in-frame) a.brand{display:block;grid-column:2}html:not(.in-frame) .step-count{grid-column:3}}"
        ".in-frame .brand{visibility:hidden}"
    )
    if kind != "edit":
        # The app's example PDFs are not on the website.
        style += "#samplesBox{display:none!important}"
    else:
        # Phones: edit.css stacks the fixed save bar in a column, where .save-info keeps
        # flex: 1 1 200px and so takes 200 px of height for one line, hiding most of the PDF (worse
        # in the frame, under the site's menu). Remove once extension/ui/edit.css has the same fix.
        style += "@media (max-width:560px){.save-bar .save-info{flex:none}}"
    lines += [f"<style>{style}</style>", TRACK_TAG]
    return "".join(f"    {line}\n" for line in lines)

def app_pages(app_dir):
    """Rewrites app/index.html, app/drop.html and app/edit.html in place."""
    pages = {"index.html": "index", "drop.html": "drop", "edit.html": "edit"}
    for name, kind in pages.items():
        path = os.path.join(app_dir, name)
        html = read(path)
        title = APP_TITLE if kind != "edit" else "Corriger le PDF — PDF Edit"
        html, n = re.subn(r"<title>.*?</title>", f"<title>{title}</title>", html, count=1, flags=re.S)
        if n != 1:
            fail(f"app/{name}: no <title> to replace")
        html, n = re.subn(r'<span class="brand">[^<]*</span>', '<a class="brand" href="/">PDF Edit</a>', html)
        if n != 1:
            fail(f'app/{name}: expected one <span class="brand">…</span> in the header, found {n}; update scripts/build-web.sh')
        if kind != "edit" and 'id="samplesBox"' in html:
            # Keep the element (drop.js uses it), drop the buttons that name example PDFs.
            html, n = re.subn(r'(<details id="samplesBox"[^>]*>).*?(</details>)', r"\1\2", html, count=1, flags=re.S)
            if n != 1:
                fail(f"app/{name}: could not empty #samplesBox; update scripts/build-web.sh")
        if "data-sample" in html:
            fail(f"app/{name}: an example PDF button (data-sample) is still there")
        if re.search(r'<meta\s+name="(robots|referrer)"|<link\s+rel="canonical"', html):
            fail(f"app/{name}: already has a robots, referrer or canonical tag; update scripts/build-web.sh")
        # The frame script first in <head>, right after the charset: it runs before the app's
        # stylesheet loads, so a page opened on its own leaves at once, and framed it is never
        # painted with its brand.
        html, n = re.subn(r"(<meta charset=\"[^\"]*\"\s*/?>[ \t]*\n?)", lambda m: m.group(1) + "    " + FRAME_SCRIPT + "\n",
                          html, count=1, flags=re.I)
        if n != 1:
            html, n = re.subn(r"(<head>[ \t]*\n?)", lambda m: m.group(1) + "    " + FRAME_SCRIPT + "\n", html, count=1)
            if n != 1:
                fail(f"app/{name}: no <head> for the frame script")
        if html.count("</head>") != 1:
            fail(f"app/{name}: expected one </head>")
        html, n = re.subn(r"^([ \t]*)</head>", lambda m: head_block(kind) + m.group(1) + "</head>",
                          html, count=1, flags=re.M)
        if n == 0:
            html = html.replace("</head>", "\n" + head_block(kind) + "</head>", 1)
        write(path, html)

def check_pkg(core_js, pkg_js):
    """The engine must export every wasm_* function the app pages call."""
    used = set(re.findall(r"\bwasm\.(wasm_[A-Za-z0-9_]+)\s*\(", read(core_js)))
    exported = set(re.findall(r"export function (wasm_[A-Za-z0-9_]+)\s*\(", read(pkg_js)))
    missing = sorted(used - exported)
    if missing:
        fail("the WASM package does not match the app pages: it lacks " + ", ".join(missing)
             + ". Build it from the same revision (or leave out --pkg).")
    print(f"    The engine exports the {len(used)} functions the pages call")

def read_release(path):
    """version, file, url, bytes, sha256 from release.example.mts."""
    text = read(path)
    out = {}
    for key in ("version", "file", "url", "sha256"):
        m = re.search(r"\b" + key + r"""\s*:\s*(?:'([^']*)'|"([^"]*)")""", text)
        if not m:
            fail(f"{path}: no {key}: '…' in it")
        out[key] = m.group(1) if m.group(1) is not None else m.group(2)
    m = re.search(r"\bbytes\s*:\s*([0-9][0-9_]*)", text)
    if not m:
        fail(f"{path}: no bytes: <number> in it")
    out["bytes"] = int(m.group(1).replace("_", ""))
    print(json.dumps(out))

FRENCH_MONTHS = ("janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août",
                 "septembre", "octobre", "novembre", "décembre")

def french_date(day):
    # YYYY-MM-DD → « 1er novembre 2026 », with no-break spaces so it never wraps inside.
    y, m, d = (int(x) for x in day.split("-"))
    return ("1er" if d == 1 else str(d)) + "\u00a0" + FRENCH_MONTHS[m - 1] + "\u00a0" + str(y)

def french_size(n):
    # Decimal megabytes, as the Finder counts them: 7 340 032 bytes → « 7,3 Mo ».
    return f"{n / 1_000_000:.1f}".replace(".", ",") + " Mo"

TEXT_EXTS = (".html", ".xml", ".txt", ".json", ".webmanifest", ".js")
IF_GITHUB = re.compile(r"<!--IF_GITHUB-->(.*?)<!--/IF_GITHUB-->", re.S)

def substitute(out_dir, github_public):
    values = {name: os.environ["WEB_" + name] for name in (
        "SITE_URL", "VERSION", "DMG_FILE", "DMG_BYTES", "DMG_SHA256", "RELEASE_DATE",
        "BUILD_DATE", "GITHUB_URL", "DOWNLOAD_URL")}
    values["DMG_SIZE"] = french_size(int(values["DMG_BYTES"]))
    values["BUILD_DATE_FR"] = french_date(values["BUILD_DATE"])
    changed = blocks = 0
    for root, _, files in os.walk(out_dir):
        for name in files:
            if not name.endswith(TEXT_EXTS):
                continue
            path = os.path.join(root, name)
            text = read(path)
            new = text
            for key, value in values.items():
                new = new.replace("{{" + key + "}}", value)
            blocks += len(IF_GITHUB.findall(new))
            new = IF_GITHUB.sub((lambda m: m.group(1)) if github_public else "", new)
            if new != text:
                write(path, new)
                changed += 1
    print(f"    {changed} files filled in, {blocks} GitHub blocks " + ("kept" if github_public else "removed"))
    print(f"    Size shown on the site: {values['DMG_SIZE']}")

def scan(out_dir, github_public):
    """Leftover placeholders or IF_GITHUB markers, and the repository's address while it is private."""
    placeholder = re.compile(rb"\{\{[A-Z][A-Z0-9_]*\}\}")
    repo = re.compile(rb"github\.com/makio64/pdf-edit", re.I)
    problems = []
    for root, _, files in os.walk(out_dir):
        for name in files:
            path = os.path.join(root, name)
            with open(path, "rb") as f:
                data = f.read()
            if b"\0" in data[:8192]:
                continue  # images, the engine, the disk image
            rel = os.path.relpath(path, out_dir)
            for m in sorted(set(placeholder.findall(data))):
                problems.append(f"{rel}: placeholder {m.decode()} left")
            if b"IF_GITHUB" in data:
                problems.append(f"{rel}: an IF_GITHUB marker is left (unclosed or nested block?)")
            if not github_public and repo.search(data):
                problems.append(f"{rel}: names the GitHub repository outside an IF_GITHUB block")
    if problems:
        fail("the site is not ready:\n  " + "\n  ".join(problems))

def check_links(app_dir):
    """Every relative href/src of the app pages points at a file of app/."""
    missing = []
    for name in ("index.html", "drop.html", "edit.html"):
        html = read(os.path.join(app_dir, name))
        for ref in re.findall(r'(?:href|src)="([^"]+)"', html):
            if re.match(r"^(?:[a-z]+:|/|#)", ref):
                continue
            target = ref.split("#")[0].split("?")[0]
            if target and not os.path.isfile(os.path.join(app_dir, target)):
                missing.append(f"app/{name} → {ref}")
    if missing:
        fail("links to files that are not there:\n  " + "\n  ".join(missing))

from html.parser import HTMLParser
from urllib.parse import unquote

class Tags(HTMLParser):
    """Every start tag of a page: (name, attributes, line)."""
    def __init__(self, html):
        super().__init__(convert_charrefs=True)
        self.tags = []
        self.feed(html)
        self.close()
    def handle_starttag(self, tag, attrs):
        self.tags.append((tag, {k: (v or "") for k, v in attrs}, self.getpos()[0]))
    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)

# ---------------------------------------------------------------------------------------------
# Languages: the dictionaries of site/i18n and the page templates of site/src (docs/WEB.md, « Languages »)
# ---------------------------------------------------------------------------------------------

import html as htmllib
from collections import Counter

# The switcher's order; a dictionary whose code is not listed comes after, alphabetically.
LANG_ORDER = ("fr", "en", "es", "de", "it", "pt", "zh", "ja")
BASE_LANGS = ("fr", "en")  # always built: French is the site, English is x-default
META_KEYS = ("lang", "og_locale", "name", "slugs")
SLUG_KEYS = ("online", "faq", "privacy")
FR_SLUGS = {"online": "en-ligne", "faq": "questions", "privacy": "mentions-legales"}  # indexed: never change
# The public pages: id, template in site/src (its French address), page-view name (data-pv).
TEMPLATES = (
    ("home", "index.html", "/"),
    ("online", "en-ligne/index.html", "/en-ligne/"),
    ("faq", "questions/index.html", "/questions/"),
    ("privacy", "mentions-legales/index.html", "/mentions-legales/"),
)
NOT_FOUND = ("404.html", "/404")  # one page, in French (with a line in English)
# Names a slug may not take under /<code>/ (or at the root for French): folders of the site.
RESERVED = {"app", "assets", "img", "stats", "downloads", "telecharger", "api", "og"}
GENERATED = ("hreflang_links", "og_locale_alternates", "lang_switcher", "lang_list", "faq_jsonld")
GLOBE = ('<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" />'
         '<path d="M3 12h18M12 3c2.4 2.5 3.6 5.5 3.6 9s-1.2 6.5-3.6 9c-2.4-2.5-3.6-5.5-3.6-9S9.6 5.5 12 3z" /></svg>')
CJK = re.compile(r"^(zh|ja|ko)\b")

def lang_sort(codes):
    return sorted(codes, key=lambda c: (LANG_ORDER.index(c) if c in LANG_ORDER else len(LANG_ORDER), c))

def available_langs(i18n_dir):
    if not os.path.isdir(i18n_dir):
        fail(f"no dictionaries: {i18n_dir} is missing")
    return lang_sort(f[:-5] for f in os.listdir(i18n_dir) if f.endswith(".json"))

def select_langs(i18n_dir, wanted):
    """LANGS / --langs (comma or space separated), always with fr and en; default: every dictionary."""
    have = available_langs(i18n_dir)
    if wanted.strip():
        codes = [c for c in re.split(r"[\s,]+", wanted.strip().lower()) if c]
        unknown = [c for c in codes if c not in have]
        if unknown:
            fail(f"LANGS / --langs: no dictionary for {', '.join(unknown)} in {i18n_dir} (there are: {', '.join(have)})")
        codes = lang_sort(set(codes) | set(BASE_LANGS))
    else:
        codes = have
    for c in BASE_LANGS:
        if c not in have:
            fail(f"{i18n_dir}/{c}.json is missing: French and English are always built (English is the x-default page)")
    return codes

def server_langs(validate_mts):
    m = re.search(r"export const LANGS\s*=\s*\[([^\]]*)\]", read(validate_mts))
    if not m:
        fail(f"{validate_mts}: no `export const LANGS = [...]` (the language codes the server accepts)")
    return [a or b for a, b in re.findall(r"'([a-z]+)'|\"([a-z]+)\"", m.group(1))]

def load_dict(i18n_dir, code):
    path = os.path.join(i18n_dir, code + ".json")
    try:
        data = json.loads(read(path))
    except json.JSONDecodeError as e:
        fail(f"{path}: not valid JSON, line {e.lineno} column {e.colno}: {e.msg}")
    if not isinstance(data, dict):
        fail(f"{path}: must be one JSON object")
    return data

def text_keys(d):
    return {k for k in d if k not in META_KEYS and not k.startswith("_")}

def markup(value):
    """The tags and placeholders of a string, which every translation must keep (a nowrap span is free)."""
    starts = re.findall(r"<([a-zA-Z][a-zA-Z0-9]*)((?:\s[^<>]*)?)>", value)
    nw = sum(1 for name, attrs in starts if name == "span" and attrs.strip() == 'class="nw"')
    tokens = Counter(re.findall(r"\{\{[^{}]*\}\}", value))
    tokens += Counter(f"<{name}{attrs}>" for name, attrs in starts if not (name == "span" and attrs.strip() == 'class="nw"'))
    tokens += Counter(f"</{name}>" for name in re.findall(r"</([a-zA-Z][a-zA-Z0-9]*)\s*>", value))
    tokens -= Counter({"</span>": nw})
    return tokens

def check_dicts(i18n_dir, codes, validate_mts, src_dir):
    """Every dictionary: its meta, the same keys as fr.json, the same tags and placeholders, its formats
    and its social image. All the problems at once, then fail."""
    allowed = server_langs(validate_mts)
    fr = load_dict(i18n_dir, "fr")
    fr_keys = text_keys(fr)
    problems = []
    seen_slugs = {}
    for code in codes:
        d = fr if code == "fr" else load_dict(i18n_dir, code)
        where = f"site/i18n/{code}.json"
        p = []
        if not re.fullmatch(r"[a-z]{2}", code):
            p.append(f"the file name must be a two-letter language code (got {code})")
        if code not in allowed:
            p.append(f"{code} is not in LANGS of netlify/lib/validate.mts (the server would refuse its page views)")
        for key in META_KEYS:
            if key not in d:
                p.append(f"no \"{key}\"")
        if not re.fullmatch(r"[a-z]{2,3}(-[A-Za-z0-9]{2,8})*", str(d.get("lang", ""))) or not str(d.get("lang", "")).startswith(code):
            p.append(f"\"lang\" must be a BCP 47 tag starting with {code} (e.g. {code} or zh-Hans), got {d.get('lang')!r}")
        if not re.fullmatch(r"[a-z]{2}_[A-Z]{2}", str(d.get("og_locale", ""))):
            p.append(f"\"og_locale\" must look like en_US, got {d.get('og_locale')!r}")
        if not isinstance(d.get("name"), str) or not d.get("name", "").strip() or re.search(r'["<>]', d.get("name", "")):
            p.append("\"name\" must be the language's own name (no quotes or tags)")
        for key in ("lang.label", "lang.list"):
            if re.search(r'["<>]', str(d.get(key, ""))):
                p.append(f"{key} may not contain straight quotes (\") or tags")
        slugs = d.get("slugs")
        if not isinstance(slugs, dict) or set(slugs) != set(SLUG_KEYS):
            p.append(f"\"slugs\" must have exactly {', '.join(SLUG_KEYS)}")
        else:
            for k, v in slugs.items():
                if not isinstance(v, str) or not re.fullmatch(r"[a-z0-9]+(-[a-z0-9]+)*", v):
                    p.append(f"slugs.{k} must be lowercase ASCII words joined by - (got {v!r})")
                elif v in RESERVED or v in LANG_ORDER or v in codes:
                    p.append(f"slugs.{k}: {v!r} is a folder of the site or a language code")
            if len(set(slugs.values())) != len(slugs):
                p.append("two slugs are the same")
            if code == "fr" and slugs != FR_SLUGS:
                p.append(f"the French slugs are the indexed addresses and cannot change: {FR_SLUGS}")
        missing = sorted(fr_keys - text_keys(d))
        extra = sorted(text_keys(d) - fr_keys)
        if missing:
            p.append(f"{len(missing)} key(s) missing: " + ", ".join(missing))
        if extra:
            p.append(f"{len(extra)} key(s) that fr.json does not have: " + ", ".join(extra))
        for key in sorted(fr_keys & text_keys(d)):
            v = d[key]
            if not isinstance(v, str) or not v.strip():
                p.append(f"{key}: must be a non-empty string")
                continue
            if code != "fr" and markup(v) != markup(fr[key]):
                want, got = markup(fr[key]), markup(v)
                diff = [f"lacks {t}" for t in (want - got)] + [f"has an extra {t}" for t in (got - want)]
                p.append(f"{key}: keep the same tags and placeholders as fr.json ({'; '.join(diff)})")
            if re.fullmatch(r"faq\.\d+\.q", key) and "<" in v:
                p.append(f"{key}: no tag in a FAQ question, not even <span class=\"nw\"> (the question is one flex item of <summary>)")
        if not p:
            months = d["fmt.months"].split()
            if len(months) != 12:
                p.append(f"fmt.months must be the 12 month names separated by spaces (got {len(months)})")
            if d["fmt.decimal"] not in (",", "."):
                p.append("fmt.decimal must be , or .")
            if "{n}" not in d["fmt.size"]:
                p.append("fmt.size must contain {n}, the number of megabytes")
            if "{d}" not in d["fmt.date"] or "{y}" not in d["fmt.date"] or not re.search(r"\{m(onth)?\}", d["fmt.date"]):
                p.append("fmt.date must contain {d}, {y}, and {month} or {m}")
            if not re.search(r"\{\{date\}\}", d["privacy.updated"]):
                p.append("privacy.updated must contain {{date}}")
        if code != "fr" and not os.path.isfile(os.path.join(src_dir, f"og-{code}.png")):
            p.append(f"its social image site/src/og-{code}.png is missing: run site/og/render.sh {code}")
        if p:
            problems.append(f"{where}:\n    " + "\n    ".join(p))
    if problems:
        fail("the dictionaries are not ready:\n  " + "\n  ".join(problems)
             + "\n  To build only some languages: LANGS=fr,en,<code> scripts/build-web.sh … (or --langs).")
    print(f"    Dictionaries: {', '.join(codes)} ({len(fr_keys)} strings each)")

class Site:
    """The languages being built and their addresses."""
    def __init__(self, i18n_dir, codes):
        self.codes = codes
        self.d = {c: load_dict(i18n_dir, c) for c in codes}
    def url(self, code, page):
        prefix = "" if code == "fr" else "/" + code
        if page == "home":
            return prefix + "/"
        return f"{prefix}/{self.d[code]['slugs'][page]}/"
    def path(self, code, page):
        """The file of a page in the output folder."""
        return self.url(code, page).lstrip("/") + "index.html"

def link_to(site, code, current, page):
    d = site.d[code]
    cur = ' aria-current="true"' if code == current else ""
    return f'<a href="{site.url(code, page)}" hreflang="{d["lang"]}" lang="{d["lang"]}"{cur}>{d["name"]}</a>'

def gen_block(name, site, code, page, indent):
    """The generated pieces of a page; lines after the first take the placeholder's indentation."""
    d = site.d[code]
    links_page = page if page != "404" else "home"
    if name == "hreflang_links":
        lines = [f'<link rel="alternate" hreflang="{site.d[c]["lang"]}" href="{{{{SITE_URL}}}}{site.url(c, page)}" />' for c in site.codes]
        lines.append(f'<link rel="alternate" hreflang="x-default" href="{{{{SITE_URL}}}}{site.url("en", page)}" />')
    elif name == "og_locale_alternates":
        lines = [f'<meta property="og:locale:alternate" content="{site.d[c]["og_locale"]}" />' for c in site.codes if c != code]
    elif name == "lang_switcher":
        lines = ['<details class="lang-menu" name="site-menu">',
                 f'  <summary>{GLOBE}<span class="lang-code" aria-hidden="true">{code.upper()}</span><span class="sr-only">{d["lang.label"]}</span></summary>',
                 '  <ul class="lang-panel">']
        lines += [f"    <li>{link_to(site, c, code, links_page)}</li>" for c in site.codes]
        lines += ["  </ul>", "</details>"]
    elif name == "lang_list":
        lines = [f'<nav class="foot-langs" aria-label="{d["lang.list"]}">', "  <ul>"]
        lines += [f"    <li>{link_to(site, c, code, links_page)}</li>" for c in site.codes]
        lines += ["  </ul>", "</nav>"]
    elif name == "faq_jsonld":
        js = lambda s: json.dumps(s, ensure_ascii=False)
        nums = sorted({int(m.group(1)) for k in d for m in [re.fullmatch(r"faq\.(\d+)\.q", k)] if m})
        entries = []
        for i in nums:
            q, a = (plain_text(resolve_inner(d[f"faq.{i}.{x}"], site, code)) for x in ("q", "a"))
            entries.append('{"@type": "Question", "name": ' + js(q) + ', "acceptedAnswer": {"@type": "Answer", "text": ' + js(a) + "}}")
        lines = ['{"@context": "https://schema.org", "@type": "FAQPage", "inLanguage": ' + js(d["lang"]) + ', "mainEntity": [']
        lines += [f"  {e}," for e in entries[:-1]] + [f"  {entries[-1]}", "]}"]
    else:
        fail(f"unknown generated block {name}")
    return ("\n" + indent).join(lines) if lines else ""

def plain_text(value):
    """A string of the page as plain text: no tags, entities decoded, spaces collapsed (ASCII only)."""
    return re.sub(r"[ \t\r\n]+", " ", htmllib.unescape(re.sub(r"<[^>]*>", "", value))).strip()

def localized_size(d, nbytes):
    return d["fmt.size"].replace("{n}", f"{nbytes / 1_000_000:.1f}".replace(".", d["fmt.decimal"]))

def localized_date(d, day):
    y, m, dd = (int(x) for x in day.split("-"))
    months = d["fmt.months"].split()
    return (d["fmt.date"].replace("{d}", str(dd)).replace("{month}", months[m - 1])
            .replace("{m}", str(m)).replace("{y}", str(y)))

def resolve_inner(value, site, code):
    """The placeholders a string of language `code` may hold, resolved in that language."""
    d = site.d[code]
    value = re.sub(r"\{\{url:(home|online|faq|privacy)\}\}", lambda m: site.url(code, m.group(1)), value)
    if code == "fr":
        date = '<time datetime="{{BUILD_DATE}}">{{BUILD_DATE_FR}}</time>'  # « 1er novembre 2026 »
    else:
        date = '<time datetime="{{BUILD_DATE}}">' + localized_date(d, os.environ["WEB_BUILD_DATE"]) + "</time>"
        value = value.replace("{{DMG_SIZE}}", localized_size(d, int(os.environ["WEB_DMG_BYTES"])))
    return value.replace("{{date}}", date)

def context(text, i):
    """Where position i of a page is: json (structured data), script, plain (<title>), attr, or html."""
    s, e = text.rfind("<script", 0, i), text.rfind("</script>", 0, i)
    if s > e:
        return "json" if "application/ld+json" in text[s:text.find(">", s)] else "script"
    if text.rfind("<title", 0, i) > text.rfind("</title>", 0, i):
        return "plain"
    return "attr" if text.rfind("<", 0, i) > text.rfind(">", 0, i) else "html"

def line_of(text, i):
    return text.count("\n", 0, i) + 1

def render_page(template, name, site, code, page):
    """One page of one language. `page` is the id of TEMPLATES, or 404."""
    d = site.d[code]
    text = template
    # 1. <!--IF_NOT_FR-->…<!--/IF_NOT_FR-->: only on the translated pages (the whole line goes on French ones).
    if code == "fr":
        text = re.sub(r"^[ \t]*<!--IF_NOT_FR-->.*?<!--/IF_NOT_FR-->[ \t]*\n", "", text, flags=re.M | re.S)
        text = re.sub(r"<!--IF_NOT_FR-->.*?<!--/IF_NOT_FR-->", "", text, flags=re.S)
    else:
        text = re.sub(r"^[ \t]*<!--/?IF_NOT_FR-->[ \t]*\n", "", text, flags=re.M)
        text = text.replace("<!--IF_NOT_FR-->", "").replace("<!--/IF_NOT_FR-->", "")
    if "IF_NOT_FR" in text:
        fail(f"{name}: an IF_NOT_FR marker is left (unclosed or nested block?)")
    # 2. The generated pieces, at the indentation of their placeholder.
    text = re.sub(r"^([ \t]*)\{\{(" + "|".join(GENERATED) + r")\}\}",
                  lambda m: m.group(1) + gen_block(m.group(2), site, code, page, m.group(1)), text, flags=re.M)
    # 3. The strings: {{t:key}}, or {{t:key@xx}} for another language's.
    def string(m):
        key, other = m.group(1), m.group(2) or code
        where = f"{name} ({code}), line {line_of(text, m.start())}"
        if other not in site.d:
            fail(f"{where}: {m.group(0)}: the language {other} is not built")
        value = site.d[other].get(key)
        if not isinstance(value, str):
            fail(f"{where}: {m.group(0)}: no such key in site/i18n/{other}.json")
        value = resolve_inner(value, site, other)
        kind = context(text, m.start())
        if kind == "script":
            fail(f"{where}: {m.group(0)} inside a script")
        if kind in ("attr", "plain", "json") and re.search(r'["<>]', htmllib.unescape(value) if kind == "json" else value):
            fail(f"{where}: {key} ({other}) is used in {'a title' if kind == 'plain' else 'structured data' if kind == 'json' else 'an attribute'}: "
                 f"it may not contain straight quotes (\") or tags: {value!r}")
        if kind == "json":
            value = json.dumps(htmllib.unescape(value), ensure_ascii=False)[1:-1]
        return value
    text = re.sub(r"\{\{t:([a-z0-9_.]+)(?:@([a-z]{2}))?\}\}", string, text)
    # 4. Addresses and the other per-language values.
    def url(m):
        other = m.group(2) or code
        if other not in site.d:
            fail(f"{name} ({code}): {m.group(0)}: the language {other} is not built")
        return site.url(other, m.group(1))
    text = re.sub(r"\{\{url:(home|online|faq|privacy)(?:@([a-z]{2}))?\}\}", url, text)
    text = re.sub(r"\{\{lang(?:@([a-z]{2}))?\}\}", lambda m: site.d[m.group(1) or code]["lang"], text)
    text = (text.replace("{{og_locale}}", d["og_locale"])
            .replace("{{og_image}}", "og.png" if code == "fr" else f"og-{code}.png")
            .replace("{{dl_lang}}", f"&amp;l={code}"))  # the download counts the page's language (French too)
    text = resolve_inner(text, site, code)  # {{DMG_SIZE}} of this language ({{SITE_URL}} etc. come later)
    left = re.search(r"\{\{(?![A-Z][A-Z0-9_]*\}\})[^}]*\}?\}?", text)
    if left:
        fail(f"{name} ({code}), line {line_of(text, left.start())}: unknown placeholder {left.group(0)}")
    return text

def render(src_dir, out_dir, i18n_dir, codes):
    """Every public page in every language, the 404 page and the sitemap."""
    site = Site(i18n_dir, codes.split())
    count = 0
    for page, rel, _ in TEMPLATES:
        template = read(os.path.join(src_dir, rel))
        for code in site.codes:
            target = os.path.join(out_dir, site.path(code, page))
            os.makedirs(os.path.dirname(target), exist_ok=True)
            write(target, render_page(template, rel, site, code, page))
            count += 1
    write(os.path.join(out_dir, NOT_FOUND[0]), render_page(read(os.path.join(src_dir, NOT_FOUND[0])), NOT_FOUND[0], site, "fr", "404"))
    # The sitemap: every page of every language, each with its alternates (the same set as its hreflang links).
    urls = []
    for code in site.codes:
        for page, _, _ in TEMPLATES:
            alts = [f'    <xhtml:link rel="alternate" hreflang="{site.d[c]["lang"]}" href="{{{{SITE_URL}}}}{site.url(c, page)}" />' for c in site.codes]
            alts.append(f'    <xhtml:link rel="alternate" hreflang="x-default" href="{{{{SITE_URL}}}}{site.url("en", page)}" />')
            urls.append("  <url>\n" f"    <loc>{{{{SITE_URL}}}}{site.url(code, page)}</loc>\n" "    <lastmod>{{BUILD_DATE}}</lastmod>\n"
                        + "\n".join(alts) + "\n  </url>")
    write(os.path.join(out_dir, "sitemap.xml"),
          '<?xml version="1.0" encoding="UTF-8"?>\n'
          '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">\n'
          + "\n".join(urls) + "\n</urlset>\n")
    print(f"    {count} pages in {len(site.codes)} languages ({', '.join(site.codes)}), 404.html, sitemap.xml ({len(urls)} addresses)")

class Faq(HTMLParser):
    """The visible questions and answers of a questions page, as plain text."""
    def __init__(self, html):
        super().__init__(convert_charrefs=True)
        self.items, self.depth, self.field, self.buf = [], 0, None, []
        self.feed(html)
        self.close()
    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag == "div" and "faq" in (a.get("class") or "").split():
            self.depth = 1
        elif self.depth and tag == "div":
            self.depth += 1
        elif self.depth and tag in ("summary", "p") and self.field is None:
            self.field, self.buf = tag, []
    def handle_endtag(self, tag):
        if self.depth and tag == "div":
            self.depth -= 1
        elif self.field == tag:
            text = re.sub(r"[ \t\r\n]+", " ", "".join(self.buf)).strip()
            if tag == "summary":
                self.items.append([text, None])
            elif self.items:
                self.items[-1][1] = text
            self.field = None
    def handle_data(self, data):
        if self.field:
            self.buf.append(data)

def check_i18n(out_dir, site_url, i18n_dir, codes):
    """After the placeholders: every language's pages, their hreflang sets, canonical, data-pv, the
    switcher, the structured FAQ, the length of titles and descriptions."""
    site = Site(i18n_dir, codes.split())
    problems, warnings = [], []
    pv_names = dict((page, pv) for page, _, pv in TEMPLATES)
    expected = {page: {site.d[c]["lang"]: site_url + site.url(c, page) for c in site.codes} | {"x-default": site_url + site.url("en", page)}
                for page, _, _ in TEMPLATES}
    for code in site.codes:
        d = site.d[code]
        cjk = bool(CJK.match(d["lang"]))
        for page, _, _ in TEMPLATES:
            rel = site.path(code, page)
            path = os.path.join(out_dir, rel)
            if not os.path.isfile(path):
                problems.append(f"{rel} is missing ({code}, {page})")
                continue
            text = read(path)
            tags = Tags(text).tags
            where = rel
            html_tag = next((a for t, a, _ in tags if t == "html"), {})
            if html_tag.get("lang") != d["lang"]:
                problems.append(f"{where}: <html lang> is {html_tag.get('lang')!r}, not {d['lang']!r}")
            if html_tag.get("data-pv") != pv_names[page]:
                problems.append(f"{where}: <html data-pv> must be {pv_names[page]!r} (the French page's name in the counts)")
            canon = [a.get("href") for t, a, _ in tags if t == "link" and a.get("rel") == "canonical"]
            if canon != [site_url + site.url(code, page)]:
                problems.append(f"{where}: canonical {canon} is not the page itself")
            og_url = [a.get("content") for t, a, _ in tags if t == "meta" and a.get("property") == "og:url"]
            if og_url != canon:
                problems.append(f"{where}: og:url {og_url} differs from the canonical address")
            alts = {}
            for t, a, _ in tags:
                if t == "link" and a.get("rel") == "alternate" and a.get("hreflang"):
                    if a["hreflang"] in alts:
                        problems.append(f"{where}: two hreflang=\"{a['hreflang']}\" links")
                    alts[a["hreflang"]] = a.get("href")
            if alts != expected[page]:
                problems.append(f"{where}: the hreflang links do not list every language of this page (and x-default → English): "
                                f"missing {sorted(set(expected[page].items()) - set(alts.items()))}, unexpected {sorted(set(alts.items()) - set(expected[page].items()))}")
            locs = [a.get("content") for t, a, _ in tags if t == "meta" and a.get("property") == "og:locale"]
            others = sorted(a.get("content") for t, a, _ in tags if t == "meta" and a.get("property") == "og:locale:alternate")
            if locs != [d["og_locale"]] or others != sorted(site.d[c]["og_locale"] for c in site.codes if c != code):
                problems.append(f"{where}: og:locale {locs} / og:locale:alternate {others} do not match the languages")
            switch = [a.get("href") for t, a, _ in tags if t == "a" and a.get("hreflang") and a.get("lang")]
            want = [site.url(c, page) for c in site.codes]
            if switch[: len(want)] != want or (page != "online" and switch[len(want):] != want):
                problems.append(f"{where}: the language switcher (and the footer's list) should link to {want}, found {switch}")
            current = [a for t, a, _ in tags if t == "a" and a.get("hreflang") and a.get("aria-current") == "true"]
            if not current or any(a.get("lang") != d["lang"] for a in current):
                problems.append(f"{where}: the switcher must mark this language with aria-current")
            if code != "fr" and page in ("home", "online") and 'class="lang-note"' not in text:
                problems.append(f"{where}: the line saying the app's screens are in French is missing")
            title = re.search(r"<title>(.*?)</title>", text, re.S)
            desc = next((a.get("content", "") for t, a, _ in tags if t == "meta" and a.get("name") == "description"), "")
            title = htmllib.unescape(title.group(1)) if title else ""
            limits = (30, 80) if cjk else (60, 155)
            for label, value, limit in (("title", title, limits[0]), ("description", desc, limits[1])):
                if len(value) > limit:
                    (warnings if cjk else problems).append(f"{where}: the {label} has {len(value)} characters (at most {limit}{', about' if cjk else ''}): {value}")
            for m in re.finditer(r'<script type="application/ld\+json">(.*?)</script>', text, re.S):
                try:
                    data = json.loads(m.group(1))
                except json.JSONDecodeError as e:
                    problems.append(f"{where}: structured data that is not valid JSON ({e.msg}, line {line_of(text, m.start()) + e.lineno - 1})")
                    continue
                if data.get("inLanguage") not in (None, d["lang"]):
                    problems.append(f"{where}: structured data with inLanguage {data.get('inLanguage')!r}")
                if data.get("@type") == "FAQPage":
                    ld = [[re.sub(r"[ \t\r\n]+", " ", q.get("name", "")).strip(),
                           re.sub(r"[ \t\r\n]+", " ", q.get("acceptedAnswer", {}).get("text", "")).strip()]
                          for q in data.get("mainEntity", [])]
                    visible = Faq(text).items
                    if not visible or ld != visible:
                        bad = [i + 1 for i, (x, y) in enumerate(zip(ld, visible)) if x != y]
                        problems.append(f"{where}: the FAQPage structured data differs from the visible questions "
                                        f"({len(ld)} vs {len(visible)} questions; different: {bad})")
            if page == "faq" and '"FAQPage"' not in text:
                problems.append(f"{where}: no FAQPage structured data")
    for w in warnings:
        warn(w)
    if problems:
        fail("the languages are not ready:\n  " + "\n  ".join(problems))
    print(f"    {len(site.codes)} languages × {len(TEMPLATES)} pages: hreflang reciprocal, canonical, data-pv, switcher, FAQ = structured data")

# Served by the functions, or the disk image: no file to look for.
NOT_FILES = ("/telecharger/", "/api/", "/downloads/")

def check_site(out_dir, site_url, i18n_dir, codes):
    """The pages of the site: menu targets, screenshots, the frame of the online pages, the sitemap."""
    site = Site(i18n_dir, codes.split())
    # The pages people see, in every language (the app pages under app/ are checked by check_links).
    public_pages = [site.path(c, page) for c in site.codes for page, _, _ in TEMPLATES] + [NOT_FOUND[0]]
    online_pages = [site.path(c, "online") for c in site.codes]
    sitemap_paths = [site.url(c, page) for c in site.codes for page, _, _ in TEMPLATES]
    problems = []
    def target(page_rel, ref):
        """The file a reference of a page means, or None when it is not a file of the site."""
        ref = ref.strip()
        if ref.startswith(site_url + "/"):
            ref = ref[len(site_url):]
        if not ref or ref.startswith(("#", "//")) or re.match(r"^[a-z][a-z0-9+.-]*:", ref, re.I):
            return None
        path = unquote(ref.split("#")[0].split("?")[0])
        if path.startswith("/"):
            if path.startswith(NOT_FILES):
                return None
            full = os.path.join(out_dir, path.lstrip("/"))
        else:
            full = os.path.join(out_dir, os.path.dirname(page_rel), path)
        full = os.path.normpath(full)
        if path.endswith("/") or os.path.isdir(full):
            full = os.path.join(full, "index.html")
        return full

    pages = []
    for root, dirs, files in os.walk(out_dir):
        rel_root = os.path.relpath(root, out_dir)
        if rel_root.split(os.sep)[0] in ("app", "downloads"):
            continue
        pages += [os.path.normpath(os.path.join(rel_root, f)) for f in files if f.endswith(".html")]
    for rel in sorted(pages):
        html = read(os.path.join(out_dir, rel))
        for tag, a, line in Tags(html).tags:
            where = f"{rel}:{line}"
            if tag == "img":
                src = a.get("src", "")
                if not src:
                    problems.append(f"{where}: <img> without src")
                elif re.match(r"^(https?:)?//", src, re.I):
                    problems.append(f"{where}: image from another site ({src}): put it in src/img/")
                elif not src.startswith("data:"):
                    f = target(rel, src)
                    if f and not os.path.isfile(f):
                        problems.append(f"{where}: image {src} is not in the site")
                for dim in ("width", "height"):
                    if not re.fullmatch(r"[1-9][0-9]*", a.get(dim, "")):
                        problems.append(f"{where}: <img src=\"{src}\"> needs {dim}=\"<pixels>\" (no layout shift)")
            if tag in ("img", "source") and a.get("srcset"):
                for item in a["srcset"].split(","):
                    url = item.strip().split(" ")[0]
                    f = target(rel, url) if url else None
                    if f and not os.path.isfile(f):
                        problems.append(f"{where}: srcset image {url} is not in the site")
            if tag == "meta" and (a.get("property") == "og:image" or a.get("name") == "twitter:image"):
                f = target(rel, a.get("content", ""))
                if f and not os.path.isfile(f):
                    problems.append(f"{where}: {a.get('property') or a.get('name')} {a.get('content')} is not in the site")
            ref = a.get("href") if tag in ("a", "link") else a.get("src") if tag in ("script", "iframe") else None
            if ref:
                f = target(rel, ref)
                if f and not os.path.isfile(f):
                    problems.append(f"{where}: <{tag}> points at {ref}, which is not in the site")

    for rel in public_pages:
        path = os.path.join(out_dir, rel)
        if not os.path.isfile(path):
            problems.append(f"{rel} is missing")
            continue
        tags = Tags(read(path)).tags
        if not any(t == "script" and a.get("src") == "/assets/track.js" and a.get("type") == "module" for t, a, _ in tags):
            problems.append(f'{rel} does not load <script type="module" src="/assets/track.js">')
        if rel in online_pages:
            frames = [(a, line) for t, a, line in tags if t == "iframe"]
            if not any(a.get("src") == "/app/" for a, _ in frames):
                problems.append(f'{rel} has no <iframe src="/app/"> (the in-browser app)')
            for a, line in frames:
                if "sandbox" in a:
                    problems.append(f"{rel}:{line}: no sandbox on the app's frame (it blocks the downloads and the folder picker)")
                if not a.get("title"):
                    problems.append(f"{rel}:{line}: the <iframe> needs a title (screen readers)")

    sitemap = os.path.join(out_dir, "sitemap.xml")
    if os.path.isfile(sitemap):
        locs = re.findall(r"<loc>\s*([^<\s]+)\s*</loc>", read(sitemap))
        for path in sitemap_paths:
            if site_url + path not in locs:
                problems.append(f"sitemap.xml does not list {site_url}{path}")
        for loc in locs:
            if re.search(r"/(app|stats|downloads)/", loc):
                problems.append(f"sitemap.xml lists {loc}, which is not for search engines")
    else:
        problems.append("sitemap.xml is missing")

    if problems:
        fail("the pages are not ready:\n  " + "\n  ".join(problems))
    print(f"    {len(pages)} pages: links, images and the frames of the online pages found, sitemap complete ({len(locs)} addresses)")

cmd, args = sys.argv[1], sys.argv[2:]
if cmd == "app-pages":
    app_pages(args[0])
elif cmd == "check-pkg":
    check_pkg(args[0], args[1])
elif cmd == "read-release":
    read_release(args[0])
elif cmd == "substitute":
    substitute(args[0], args[1] == "1")
elif cmd == "scan":
    scan(args[0], args[1] == "1")
elif cmd == "check-links":
    check_links(args[0])
elif cmd == "check-site":
    check_site(args[0], args[1], args[2], args[3])
elif cmd == "langs":
    print(" ".join(select_langs(args[0], args[1])))
elif cmd == "check-dicts":
    check_dicts(args[0], args[1].split(), args[2], args[3])
elif cmd == "render":
    render(args[0], args[1], args[2], args[3])
elif cmd == "check-i18n":
    check_i18n(args[0], args[1], args[2], args[3])
else:
    fail(f"build_web.py: unknown command {cmd}")
PY
py() { python3 -I "$WORK/build_web.py" "$@"; }

# ---------------------------------------------------------------------------------------------
# 1. What goes in, checked before anything is built or deleted
# ---------------------------------------------------------------------------------------------

step "1/7 Checking the sources, the release and the output folder"

for tool in git shasum du find; do
  command -v "$tool" >/dev/null 2>&1 || die "\"$tool\" not found"
done
[[ -d "$SRC" ]] || die "no static sources: $SRC"
[[ -f "$SRC/index.html" ]] || die "$SRC/index.html is missing (the home page)"
[[ -f "$SRC/en-ligne/index.html" ]] || die "$SRC/en-ligne/index.html is missing (the page that shows the in-browser app)"
[[ -f "$SRC/questions/index.html" ]] || die "$SRC/questions/index.html is missing (the questions page)"
[[ -f "$OVERLAY" ]] || die "missing file: $OVERLAY"
[[ -f "$SRC/mentions-legales/index.html" && -f "$SRC/404.html" ]] || die "$SRC/mentions-legales/index.html or $SRC/404.html is missing"
[[ ! -e "$SRC/sitemap.xml" ]] || die "$SRC/sitemap.xml: the sitemap is generated by this script (every language), remove it from the sources"
[[ -f "$VALIDATE_MTS" ]] || die "missing file: $VALIDATE_MTS (its LANGS are the languages the counts accept)"

# The languages: every dictionary of site/i18n, or LANGS / --langs (French and English always).
BUILD_LANGS="$(py langs "$I18N" "$LANGS")"
py check-dicts "$I18N" "$BUILD_LANGS" "$VALIDATE_MTS" "$SRC"
info "Languages: $BUILD_LANGS"

# The app pages: a git revision, or the working tree.
if [[ -n "$UI_REV" ]]; then
  UI_COMMIT="$(git -C "$REPO" rev-parse --verify --quiet "$UI_REV^{commit}")" ||
    die "--ui-rev: \"$UI_REV\" is not a commit of this repository"
  UI_LABEL="$UI_REV ($(git -C "$REPO" rev-parse --short "$UI_COMMIT"))"
  for f in "${UI_FILES[@]}"; do
    git -C "$REPO" cat-file -e "$UI_COMMIT:extension/ui/$f" 2>/dev/null ||
      die "extension/ui/$f is not in $UI_LABEL"
  done
else
  UI_COMMIT=""
  UI_LABEL="working tree"
  for f in "${UI_FILES[@]}"; do
    [[ -f "$REPO/extension/ui/$f" ]] || die "missing file: extension/ui/$f"
  done
  if [[ -n "$(git -C "$REPO" status --porcelain -- extension/ui 2>/dev/null)" ]]; then
    UI_LABEL="working tree, with uncommitted changes"
    warn "extension/ui has uncommitted changes and this build includes them. For a release, use --ui-rev HEAD."
  fi
fi
info "App pages: extension/ui, $UI_LABEL"

if [[ -n "$PKG" ]]; then
  for f in p2d_wasm.js p2d_wasm_bg.wasm; do
    [[ -f "$PKG/$f" ]] || die "--pkg: $PKG has no $f (it must be the wasm-bindgen --target web output)"
  done
  info "Engine: prebuilt package $PKG"
fi

# The release: from the disk image, or the example facts.
if [[ -n "$DMG" ]]; then
  [[ -f "$DMG" ]] || die "--dmg: no such file: $DMG"
  DMG_FILE="$(basename "$DMG")"
  if [[ "$DMG_FILE" == *NON-NOTARISE* ]]; then
    die "$DMG_FILE is not notarized: macOS would refuse to open it on other Macs. Publish the image of pnpm mac:release."
  fi
  dmg_re='^(PDF-Edit|Modifier-mes-PDF)-([0-9][0-9a-z.-]{0,15})\.dmg$'
  [[ "$DMG_FILE" =~ $dmg_re ]] ||
    die "--dmg: \"$DMG_FILE\" is not named PDF-Edit-<version>.dmg (or Modifier-mes-PDF-<version>.dmg)"
  VERSION="${BASH_REMATCH[2]}"
  DMG_BYTES="$(wc -c <"$DMG" | tr -d '[:space:]')"
  DMG_SHA256="$(shasum -a 256 "$DMG" | awk '{print $1}')"
  RELEASE_DATE="$(date -u -r "$DMG" +%Y-%m-%d)"
  if [[ "$(uname -s)" == Darwin ]] && command -v xcrun >/dev/null 2>&1; then
    # Apple's ticket service sometimes answers with a transient error (68): retry before giving up.
    attempt=1
    until xcrun stapler validate -q "$DMG" >/dev/null 2>&1; do
      ((attempt >= 5)) &&
        die "$DMG_FILE carries no notarization ticket (xcrun stapler validate failed). Publish the image of pnpm mac:release."
      attempt=$((attempt + 1))
      sleep 3
    done
    info "Notarization ticket: stapled"
  else
    warn "cannot check the notarization of $DMG_FILE on this system"
  fi
  if [[ "$DMG_HOSTING" == github ]]; then
    DMG_URL="$GITHUB_URL/releases/download/v$VERSION/$DMG_FILE"
  else
    DMG_URL="/downloads/$DMG_FILE"
  fi
  RELEASE_SOURCE="$DMG_FILE"
else
  [[ -f "$RELEASE_EXAMPLE" ]] || die "missing file: $RELEASE_EXAMPLE (the release facts used without --dmg)"
  example="$(py read-release "$RELEASE_EXAMPLE")"
  json_field() { python3 -I -c 'import json, sys; print(json.loads(sys.argv[1])[sys.argv[2]])' "$example" "$1"; }
  VERSION="$(json_field version)"
  DMG_FILE="$(json_field file)"
  DMG_URL="$(json_field url)"
  if [[ "$DMG_HOSTING" == github ]]; then
    DMG_URL="$GITHUB_URL/releases/download/v$VERSION/$DMG_FILE"
  fi
  DMG_BYTES="$(json_field bytes)"
  DMG_SHA256="$(json_field sha256)"
  RELEASE_DATE="$BUILD_DATE"
  RELEASE_SOURCE="the example facts ($(basename "$RELEASE_EXAMPLE"))"
  warn "no --dmg: version, size and SHA-256 on the site are the example's, and the download link
  ($DMG_URL) leads to no file. Fine for a preview, not for production."
fi
version_re='^[0-9][0-9a-z.-]{0,15}$'
[[ "$VERSION" =~ $version_re ]] || die "the release version \"$VERSION\" is not a version such as 1.2.0"
[[ "$DMG_BYTES" =~ ^[0-9]+$ ]] || die "the release size \"$DMG_BYTES\" is not a number of bytes"
info "Mac release: $VERSION, $DMG_FILE, from $RELEASE_SOURCE"
info "Download: $DOWNLOAD_URL → $DMG_URL"
if [[ $GITHUB_PUBLIC -eq 1 ]]; then
  info "GitHub: public, the <!--IF_GITHUB--> blocks are kept"
else
  info "GitHub: private, the <!--IF_GITHUB--> blocks are removed"
fi

# The output folder is emptied: refuse anything that is not one.
case "$OUT" in
  / | "$HOME" | "$REPO" | "$SITE_DIR" | "$SRC" | "$REPO/extension" | "$REPO/site/src")
    die "--out $OUT: refusing to empty this folder"
    ;;
esac
case "$REPO/" in
  "$OUT"/*) die "--out $OUT contains the repository: refusing to empty it" ;;
esac
if [[ -e "$OUT/.git" ]]; then
  die "--out $OUT is a git repository: refusing to empty it"
fi
if [[ -d "$OUT" && -n "$(ls -A "$OUT")" && ! -e "$OUT/_headers" && ! -e "$OUT/app/wasm-client.core.js" ]]; then
  die "--out $OUT is not empty and does not look like an earlier build of the site: delete it yourself, or choose another folder"
fi

# ---------------------------------------------------------------------------------------------
# 2. The WASM engine
# ---------------------------------------------------------------------------------------------

if [[ -n "$PKG" ]]; then
  step "2/7 Engine: using the prebuilt package (no cargo)"
  PKG_DIR="$PKG"
else
  step "2/7 Building the engine (p2d-wasm: cargo release-wasm, wasm-bindgen, wasm-opt)"
  # Homebrew's cargo lacks the wasm32 target: use rustup's toolchain (rust-toolchain.toml).
  export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
  for tool in cargo wasm-bindgen wasm-opt; do
    command -v "$tool" >/dev/null 2>&1 ||
      die "\"$tool\" not found. cargo: brew install rustup && rustup-init; wasm-bindgen: cargo install wasm-bindgen-cli; wasm-opt: brew install binaryen. Or pass --pkg <dir>."
  done
  # Its own target folder: never the one scripts/build-extension.sh copies into extension/pkg.
  WASM_TARGET="${CARGO_TARGET_DIR:-$REPO/target}/build-web"
  if [[ -n "$UI_COMMIT" ]]; then
    # The engine of the same revision as the pages, whatever the working tree holds.
    ENGINE_SRC="$WORK/src"
    mkdir -p "$ENGINE_SRC"
    git -C "$REPO" archive --format=tar "$UI_COMMIT" | tar -xf - -C "$ENGINE_SRC"
    info "Sources: $UI_LABEL"
    (cd "$ENGINE_SRC" && CARGO_TARGET_DIR="$WASM_TARGET" \
      cargo build --locked --profile release-wasm --target wasm32-unknown-unknown -p p2d-wasm)
  else
    info "Sources: working tree"
    CARGO_TARGET_DIR="$WASM_TARGET" cargo build --profile release-wasm --target wasm32-unknown-unknown -p p2d-wasm
  fi
  PKG_DIR="$WORK/pkg"
  wasm-bindgen --target web --out-dir "$PKG_DIR" "$WASM_TARGET/wasm32-unknown-unknown/release-wasm/p2d_wasm.wasm"
  wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-nontrapping-float-to-int \
    "$PKG_DIR/p2d_wasm_bg.wasm" -o "$PKG_DIR/p2d_wasm_bg.wasm"
fi
info "p2d_wasm_bg.wasm: $(wc -c <"$PKG_DIR/p2d_wasm_bg.wasm" | awk '{printf "%.1f MB", $1 / 1000000}')"

# ---------------------------------------------------------------------------------------------
# 3. The static pages
# ---------------------------------------------------------------------------------------------

step "3/7 Copying the static files and rendering the pages in every language into $OUT"
TOUCHED_OUT=1
rm -rf "$OUT"
mkdir -p "$OUT"
cp -R "$SRC/." "$OUT/"
find "$OUT" -name .DS_Store -delete
info "$(find "$OUT" -type f | wc -l | tr -d '[:space:]') files from $SRC"
# The templates (copied above at their French addresses) become the pages of every language.
WEB_DMG_BYTES="$DMG_BYTES" WEB_BUILD_DATE="$BUILD_DATE" py render "$SRC" "$OUT" "$I18N" "$BUILD_LANGS"

# ---------------------------------------------------------------------------------------------
# 4. The in-browser app (app/)
# ---------------------------------------------------------------------------------------------

step "4/7 Assembling the in-browser app (app/)"
[[ ! -e "$APP_OUT" ]] || die "$SRC/app exists: app/ is assembled by this script, remove it from the sources"
mkdir -p "$APP_OUT/pkg"
for f in "${UI_FILES[@]}"; do
  if [[ -n "$UI_COMMIT" ]]; then
    git -C "$REPO" show "$UI_COMMIT:extension/ui/$f" >"$APP_OUT/$f"
  else
    cp "$REPO/extension/ui/$f" "$APP_OUT/$f"
  fi
done
# /app/ is the first screen; drop.html stays, the editor's « Retour » and links go to it.
cp "$APP_OUT/drop.html" "$APP_OUT/index.html"
# The pages import ./wasm-client.js: it becomes the overlay that counts, around the original.
mv "$APP_OUT/wasm-client.js" "$APP_OUT/wasm-client.core.js"
cp "$OVERLAY" "$APP_OUT/wasm-client.js"
# The engine, without its TypeScript declarations.
cp "$PKG_DIR/p2d_wasm.js" "$PKG_DIR/p2d_wasm_bg.wasm" "$APP_OUT/pkg/"
py check-pkg "$APP_OUT/wasm-client.core.js" "$APP_OUT/pkg/p2d_wasm.js"
py app-pages "$APP_OUT"
info "index.html, drop.html, edit.html: noindex, canonical /en-ligne/, frame script, link home, /assets/track.js, no example PDFs"

# ---------------------------------------------------------------------------------------------
# 5. The release facts for the download function (and the disk image itself while private)
# ---------------------------------------------------------------------------------------------

step "5/7 Writing the release facts"
mkdir -p "$(dirname "$RELEASE_GENERATED")"
# Written from the values above in both cases (without --dmg they are release.example.mts's, with
# the GitHub release address when --dmg-hosting is github), so the function sends where the summary says.
cat >"$RELEASE_GENERATED" <<EOF
// Written by scripts/build-web.sh on $BUILD_DATE from $RELEASE_SOURCE: what /telecharger/mac sends to.
// Every build replaces it; do not edit or commit it (see docs/WEB.md).
export const release = {
  version: '$VERSION',
  file: '$DMG_FILE',
  url: '$DMG_URL',
  bytes: $DMG_BYTES,
  sha256: '$DMG_SHA256'
};
EOF
if [[ -n "$DMG" ]]; then
  if [[ "$DMG_HOSTING" == site ]]; then
    mkdir -p "$OUT/downloads"
    cp "$DMG" "$OUT/downloads/$DMG_FILE"
    info "Disk image copied to downloads/$DMG_FILE"
  else
    info "Disk image: upload $DMG_FILE to the GitHub release v$VERSION before deploying"
  fi
fi
info "${RELEASE_GENERATED#"$REPO"/}"

# ---------------------------------------------------------------------------------------------
# 6. Placeholders and GitHub blocks
# ---------------------------------------------------------------------------------------------

step "6/7 Filling in the placeholders"
WEB_SITE_URL="$SITE_URL" WEB_VERSION="$VERSION" WEB_DMG_FILE="$DMG_FILE" WEB_DMG_BYTES="$DMG_BYTES" \
  WEB_DMG_SHA256="$DMG_SHA256" WEB_RELEASE_DATE="$RELEASE_DATE" WEB_BUILD_DATE="$BUILD_DATE" \
  WEB_GITHUB_URL="$GITHUB_URL" WEB_DOWNLOAD_URL="$DOWNLOAD_URL" \
  py substitute "$OUT" "$GITHUB_PUBLIC"

# ---------------------------------------------------------------------------------------------
# 7. Checks, then the summary
# ---------------------------------------------------------------------------------------------

step "7/7 Checking the result"
for f in index.html en-ligne/index.html questions/index.html sitemap.xml _headers assets/track.js \
  app/index.html app/drop.html app/edit.html \
  app/wasm-client.js app/wasm-client.core.js app/pkg/p2d_wasm.js app/pkg/p2d_wasm_bg.wasm; do
  [[ -s "$OUT/$f" ]] || die "$f is missing from the site"
done
for f in app/index.html app/drop.html app/edit.html; do
  grep -qF '<script type="module" src="/assets/track.js"></script>' "$OUT/$f" ||
    die "$f does not load /assets/track.js"
  grep -qF "location.replace('/en-ligne/'" "$OUT/$f" || die "$f lacks the frame script"
  grep -qF "<link rel=\"canonical\" href=\"$SITE_URL/en-ligne/\" />" "$OUT/$f" || die "$f lacks its canonical /en-ligne/"
done
pdfs="$(find "$OUT" -type f -iname '*.pdf')"
[[ -z "$pdfs" ]] || die "the site must not contain PDF files:
$pdfs"
# The app knows it runs in the browser because /api/app-info is not served.
[[ ! -e "$OUT/api" ]] || die "$OUT/api exists: /api/app-info must not be served (the app would think it runs in the Mac app)"
py scan "$OUT" "$GITHUB_PUBLIC"
py check-links "$APP_OUT"
py check-site "$OUT" "$SITE_URL" "$I18N" "$BUILD_LANGS"
py check-i18n "$OUT" "$SITE_URL" "$I18N" "$BUILD_LANGS"
if [[ -n "$DMG" && "$DMG_HOSTING" == site ]]; then
  [[ "$(wc -c <"$OUT/downloads/$DMG_FILE" | tr -d '[:space:]')" == "$DMG_BYTES" ]] ||
    die "downloads/$DMG_FILE is not the same size as $DMG"
fi
info "No placeholder or IF_GITHUB marker left, no PDF, every page counted, links and images found"

step "Done"
info "Site:        $OUT ($(du -sh "$OUT" | awk '{print $1}'), $(find "$OUT" -type f | wc -l | tr -d '[:space:]') files)"
info "Address:     $SITE_URL"
info "Pages:       / · /en-ligne/ (the app of /app/, framed) · /questions/ · /mentions-legales/ · /404.html · /stats/"
info "Languages:   $BUILD_LANGS (French at /, the others under /<code>/)"
info "App pages:   $UI_LABEL"
info "Mac release: $VERSION ($DMG_FILE, $DMG_BYTES bytes, released $RELEASE_DATE)"
info "SHA-256:     $DMG_SHA256"
info "DMG hosting: $DMG_HOSTING → $DMG_URL"
if [[ $GITHUB_PUBLIC -eq 1 ]]; then
  info "GitHub:      public"
else
  info "GitHub:      private (no link to the repository)"
fi
if [[ -z "$DMG" ]]; then
  warn "preview build (no --dmg): do not deploy it to production."
fi
ALL_LANGS="$(py langs "$I18N" "")"
if [[ "$BUILD_LANGS" != "$ALL_LANGS" ]]; then
  warn "only some languages were built ($BUILD_LANGS of $ALL_LANGS, LANGS / --langs): do not deploy it to production."
fi
if [[ "$OUT" == "$SITE_DIR/dist" ]]; then
  printf '\nDeploy with: cd %s && netlify deploy --prod --dir dist --functions netlify/functions\n' "${SITE_DIR#"$REPO"/}"
else
  printf '\nDeploy with: cd %s && netlify deploy --prod --dir %s --functions netlify/functions\n' "${SITE_DIR#"$REPO"/}" "$OUT"
fi
