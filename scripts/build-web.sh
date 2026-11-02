#!/usr/bin/env bash
# Assemble the PDF Edit website in site/dist: the static pages of site/src (/, /en-ligne/,
# /questions/, /mentions-legales/, /404.html, /stats/), the in-browser app under app/ (the pages of
# extension/ui and the WASM engine, shown framed in /en-ligne/, with anonymous counts), and the
# release facts the download function answers with. See docs/WEB.md.
#
# Usage: scripts/build-web.sh [options]      (--help lists them)
#        pnpm web:build      a preview: the working tree's pages, the example release facts
#        a release:          scripts/build-web.sh --ui-rev HEAD --dmg dist/PDF-Edit-<version>.dmg
#        then:               cd site && netlify deploy --prod --dir dist --functions netlify/functions
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/build-web.sh [options]

Builds the PDF Edit website into site/dist, ready for:
  cd site && netlify deploy --prod --dir dist --functions netlify/functions

The pages come from site/src (/, /en-ligne/, /questions/, /mentions-legales/, /404.html, /stats/).
The in-browser app goes to app/: /en-ligne/ shows it in a frame (<iframe src="/app/">); opened on
its own, /app/ sends the visitor to /en-ligne/. The build stops on a broken link, an image that
is missing or has no width/height, a page that is not counted, or an incomplete sitemap.

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
  --site-dir <dir>        The site sources (src/, app-overlay/, netlify/lib/; default: site/).
                          For tests on a copy; the build writes netlify/lib/release.generated.mts there
                          (and, without --out, the site into <dir>/dist)
  -h, --help              Show this help

Environment:
  GITHUB_PUBLIC=1         Same as --github-public
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

need_value() {
  if [[ $# -lt 2 || -z "$2" || "$2" == --* ]]; then
    die "$1 needs a value (see --help)"
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ui-rev | --pkg | --dmg | --site-url | --dmg-hosting | --out | --site-dir)
      need_value "$@"
      case "$1" in
        --ui-rev) UI_REV="$2" ;;
        --pkg) PKG="$2" ;;
        --dmg) DMG="$2" ;;
        --site-url) SITE_URL="$2" ;;
        --dmg-hosting) DMG_HOSTING="$2" ;;
        --out) OUT="$2" ;;
        --site-dir) SITE_DIR="$2" ;;
      esac
      shift 2
      ;;
    --ui-rev=* | --pkg=* | --dmg=* | --site-url=* | --dmg-hosting=* | --out=* | --site-dir=*)
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

# The pages people see (the app pages under app/ are checked by check_links).
PUBLIC_PAGES = ("index.html", "en-ligne/index.html", "questions/index.html", "mentions-legales/index.html", "404.html")
SITEMAP_PATHS = ("/", "/en-ligne/", "/questions/", "/mentions-legales/")
# Served by the functions, or the disk image: no file to look for.
NOT_FILES = ("/telecharger/", "/api/", "/downloads/")

def check_site(out_dir, site_url):
    """The pages of the site: menu targets, screenshots, the frame of /en-ligne/, the sitemap."""
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

    for rel in PUBLIC_PAGES:
        path = os.path.join(out_dir, rel)
        if not os.path.isfile(path):
            problems.append(f"{rel} is missing")
            continue
        tags = Tags(read(path)).tags
        if not any(t == "script" and a.get("src") == "/assets/track.js" and a.get("type") == "module" for t, a, _ in tags):
            problems.append(f'{rel} does not load <script type="module" src="/assets/track.js">')
        if rel == "en-ligne/index.html":
            frames = [(a, line) for t, a, line in tags if t == "iframe"]
            if not any(a.get("src") == "/app/" for a, _ in frames):
                problems.append('en-ligne/index.html has no <iframe src="/app/"> (the in-browser app)')
            for a, line in frames:
                if "sandbox" in a:
                    problems.append(f"en-ligne/index.html:{line}: no sandbox on the app's frame (it blocks the downloads and the folder picker)")
                if not a.get("title"):
                    problems.append(f"en-ligne/index.html:{line}: the <iframe> needs a title (screen readers)")

    sitemap = os.path.join(out_dir, "sitemap.xml")
    if os.path.isfile(sitemap):
        locs = re.findall(r"<loc>\s*([^<\s]+)\s*</loc>", read(sitemap))
        for path in SITEMAP_PATHS:
            if site_url + path not in locs:
                problems.append(f"sitemap.xml does not list {site_url}{path}")
        for loc in locs:
            if re.search(r"/(app|stats|downloads)/", loc):
                problems.append(f"sitemap.xml lists {loc}, which is not for search engines")
    else:
        problems.append("sitemap.xml is missing")

    if problems:
        fail("the pages are not ready:\n  " + "\n  ".join(problems))
    print(f"    {len(pages)} pages: links, images and the frame of /en-ligne/ found, sitemap complete")

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
    check_site(args[0], args[1])
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

step "3/7 Copying the static pages into $OUT"
TOUCHED_OUT=1
rm -rf "$OUT"
mkdir -p "$OUT"
cp -R "$SRC/." "$OUT/"
find "$OUT" -name .DS_Store -delete
info "$(find "$OUT" -type f | wc -l | tr -d '[:space:]') files from $SRC"

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
py check-site "$OUT" "$SITE_URL"
if [[ -n "$DMG" && "$DMG_HOSTING" == site ]]; then
  [[ "$(wc -c <"$OUT/downloads/$DMG_FILE" | tr -d '[:space:]')" == "$DMG_BYTES" ]] ||
    die "downloads/$DMG_FILE is not the same size as $DMG"
fi
info "No placeholder or IF_GITHUB marker left, no PDF, every page counted, links and images found"

step "Done"
info "Site:        $OUT ($(du -sh "$OUT" | awk '{print $1}'), $(find "$OUT" -type f | wc -l | tr -d '[:space:]') files)"
info "Address:     $SITE_URL"
info "Pages:       / · /en-ligne/ (the app of /app/, framed) · /questions/ · /mentions-legales/ · /404.html · /stats/"
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
if [[ "$OUT" == "$SITE_DIR/dist" ]]; then
  printf '\nDeploy with: cd %s && netlify deploy --prod --dir dist --functions netlify/functions\n' "${SITE_DIR#"$REPO"/}"
else
  printf '\nDeploy with: cd %s && netlify deploy --prod --dir %s --functions netlify/functions\n' "${SITE_DIR#"$REPO"/}" "$OUT"
fi
