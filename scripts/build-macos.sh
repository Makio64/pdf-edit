#!/usr/bin/env bash
# Ship « PDF Edit » for macOS: build p2d-cli (universal by default), assemble
# dist/PDF Edit.app, sign it with the Developer ID certificate, smoke-test it, notarize it
# with Apple, then wrap it in a signed, notarized disk image. See docs/RELEASE-macOS.md.
#
# Usage: scripts/build-macos.sh [options]      (--help lists them)
#        pnpm mac:app       signed, not notarized: for trying on this Mac
#        pnpm mac:release   signed, notarized, stapled .app and .dmg: what gets shared
#
# Never add `set -x`: notarization credentials are passed to notarytool as arguments.
set -euo pipefail
set +x

usage() {
  cat <<'EOF'
Usage: scripts/build-macos.sh [options]

Builds "dist/PDF Edit.app" and "dist/PDF-Edit-<version>.dmg", signed with the
Developer ID certificate and notarized by Apple.

Options:
  --arch universal|arm64|x86_64  Architectures to build (default: universal)
  --no-sign           Ad-hoc signature only, runs on this Mac only (implies --no-notarize)
  --no-notarize       Sign but skip Apple's notarization (other Macs will refuse to open it)
  --no-dmg            Stop after the .app, no disk image
  --skip-self-test    Do not run "<app> --self-test" after signing
  --prebuilt <file>   Package this p2d-cli binary instead of running cargo (packaging tests)
  --open              Show the result in the Finder at the end
  -h, --help          Show this help

Environment:
  APPLE_SIGNING_IDENTITY  Signing identity (default: "Developer ID Application: Makio (86E98RSTFN)")
  Notarization credentials, first match wins:
    NOTARY_PROFILE                      a notarytool keychain profile (recommended)
    APPLE_API_KEY + APPLE_API_ISSUER    an App Store Connect API key; the .p8 file is
      [+ APPLE_API_KEY_PATH]            APPLE_API_KEY_PATH, by default
                                        ~/.appstoreconnect/private_keys/AuthKey_<APPLE_API_KEY>.p8
    APPLE_ID + APPLE_PASSWORD + APPLE_TEAM_ID   an Apple ID with an app-specific password
  ICON_PATH               Test-only: use this .icns instead of packaging/macos/AppIcon.icns
  SELF_TEST_TIMEOUT       Seconds before a hanging self-test fails the build (default: 90)
EOF
}

step() { printf '\n==> %s\n' "$*"; }
info() { printf '    %s\n' "$*"; }
warn() { printf 'Warning: %s\n' "$*" >&2; }
die() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}
indent() { sed 's/^/      /'; }
# Runs a command, shows its output indented, and stops with message $1 when it fails.
checked() {
  local what="$1" out
  shift
  if out="$("$@" 2>&1)"; then
    if [[ -n "$out" ]]; then indent <<<"$out"; fi
  else
    if [[ -n "$out" ]]; then indent <<<"$out" >&2; fi
    die "$what"
  fi
}

# ---------------------------------------------------------------------------------------------
# Options
# ---------------------------------------------------------------------------------------------

ARCH="universal"
SIGN=1
NOTARIZE=1
MAKE_DMG=1
SELF_TEST=1
PREBUILT=""
OPEN_AT_END=0

need_value() {
  if [[ $# -lt 2 || -z "$2" || "$2" == --* ]]; then
    die "$1 needs a value (see --help)"
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch)
      need_value "$@"
      ARCH="$2"
      shift 2
      ;;
    --arch=*)
      ARCH="${1#--arch=}"
      shift
      ;;
    --no-sign)
      SIGN=0
      NOTARIZE=0
      shift
      ;;
    --no-notarize)
      NOTARIZE=0
      shift
      ;;
    --no-dmg)
      MAKE_DMG=0
      shift
      ;;
    --skip-self-test)
      SELF_TEST=0
      shift
      ;;
    --prebuilt)
      need_value "$@"
      PREBUILT="$2"
      shift 2
      ;;
    --prebuilt=*)
      PREBUILT="${1#--prebuilt=}"
      shift
      ;;
    --open)
      OPEN_AT_END=1
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

case "$ARCH" in
  universal) ARCHS=(arm64 x86_64) ;;
  arm64 | aarch64)
    ARCH=arm64
    ARCHS=(arm64)
    ;;
  x86_64 | x86-64 | intel)
    ARCH=x86_64
    ARCHS=(x86_64)
    ;;
  *) die "--arch must be universal, arm64 or x86_64 (got \"$ARCH\")" ;;
esac

# Paths given on the command line or in the environment are relative to where the script was
# started: make them absolute before moving to the repository.
abspath() { printf '%s/%s\n' "$(cd "$(dirname "$1")" && pwd)" "$(basename "$1")"; }
if [[ -n "$PREBUILT" ]]; then
  [[ -f "$PREBUILT" ]] || die "--prebuilt: no such file: $PREBUILT"
  PREBUILT="$(abspath "$PREBUILT")"
fi
ICON_OVERRIDE="${ICON_PATH:-}"
if [[ -n "$ICON_OVERRIDE" && -f "$ICON_OVERRIDE" ]]; then
  ICON_OVERRIDE="$(abspath "$ICON_OVERRIDE")"
fi

# ---------------------------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------------------------

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO"

APP_NAME="PDF Edit"
BUNDLE_ID="fr.doc2pdf.modifier-mes-pdf"
DEFAULT_IDENTITY="Developer ID Application: Makio (86E98RSTFN)"
SIGN_IDENTITY="${APPLE_SIGNING_IDENTITY:-$DEFAULT_IDENTITY}"
PACKAGING="$REPO/packaging/macos"
PLIST_TEMPLATE="$PACKAGING/Info.plist.in"
ENTITLEMENTS="$PACKAGING/entitlements.plist"
STRINGS="$PACKAGING/fr.lproj/InfoPlist.strings"
NOTICES="$REPO/THIRD_PARTY_NOTICES"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target}"
DIST="$REPO/dist"
APP="$DIST/$APP_NAME.app"
EXE="$APP/Contents/MacOS/$APP_NAME"
SELF_TEST_TIMEOUT="${SELF_TEST_TIMEOUT:-90}"
[[ "$SELF_TEST_TIMEOUT" =~ ^[0-9]+$ ]] || die "SELF_TEST_TIMEOUT must be a number of seconds"

triple_for() {
  case "$1" in
    arm64) echo aarch64-apple-darwin ;;
    x86_64) echo x86_64-apple-darwin ;;
  esac
}
# The oldest macOS each slice must start on: LSMinimumSystemVersion, 11.0 (the PDF editor needs
# Safari 16's container units, which macOS 10.15 never gets).
min_macos_for() {
  case "$1" in
    arm64) echo 11.0 ;;
    x86_64) echo 11.0 ;;
  esac
}

# True when the space-separated list $1 contains the word $2.
has_word() {
  case " $1 " in
    *" $2 "*) return 0 ;;
    *) return 1 ;;
  esac
}

# True when the dotted version $1 is greater than $2 (10.15 > 10.9, 11.0 > 10.15).
version_gt() {
  local IFS=.
  local -a a b
  a=($1)
  b=($2)
  local i x y
  for i in 0 1 2; do
    x="${a[i]:-0}"
    y="${b[i]:-0}"
    if ((10#$x > 10#$y)); then return 0; fi
    if ((10#$x < 10#$y)); then return 1; fi
  done
  return 1
}

WORK=""
SELF_TEST_PID=""
TOUCHED_DIST=0
on_exit() {
  local rc=$?
  if [[ -n "$SELF_TEST_PID" ]]; then
    kill -KILL "$SELF_TEST_PID" 2>/dev/null || true
  fi
  if [[ -n "$WORK" ]]; then
    rm -rf "$WORK"
  fi
  if [[ $rc -ne 0 && $TOUCHED_DIST -eq 1 ]]; then
    printf '\nThe build stopped before the end: what is in dist/ is incomplete. Do not share it.\n' >&2
  fi
}
trap on_exit EXIT

# ---------------------------------------------------------------------------------------------
# 1. Version, build number and everything needed before a long build
# ---------------------------------------------------------------------------------------------

step "1/9 Checking the version, the tools and the credentials"

[[ "$(uname -s)" == Darwin ]] || die "this script builds the Mac app: run it on a Mac"

VERSION="$(awk '
  /^[[:space:]]*\[/ { in_pkg = ($0 ~ /^[[:space:]]*\[workspace\.package\][[:space:]]*(#.*)?$/); next }
  in_pkg && /^[[:space:]]*version[[:space:]]*=/ {
    line = $0
    sub(/^[^"]*"/, "", line)
    sub(/".*$/, "", line)
    if (!found) { print line; found = 1 }
  }
' "$REPO/Cargo.toml")"
[[ -n "$VERSION" ]] || die "could not read the [workspace.package] version from Cargo.toml"
version_re='^[0-9]+(\.[0-9]+){0,2}$'
[[ "$VERSION" =~ $version_re ]] ||
  die "Cargo.toml version \"$VERSION\" is not a Mac app version (one to three numbers, such as 1.2.0)"
BUILD="$(git -C "$REPO" rev-list --count HEAD 2>/dev/null || true)"
[[ "$BUILD" =~ ^[0-9]+$ ]] || BUILD=1
if [[ $NOTARIZE -eq 1 ]]; then
  DMG="$DIST/PDF-Edit-$VERSION.dmg"
else
  # Never under the release name: only a notarized disk image may be sent to anyone.
  DMG="$DIST/PDF-Edit-$VERSION-NON-NOTARISE.dmg"
fi
info "Version $VERSION (build $BUILD), architectures: ${ARCHS[*]}"

need_tool() {
  command -v "$1" >/dev/null 2>&1 ||
    die "\"$1\" not found. Install Apple's command line tools with: xcode-select --install"
}
for tool in codesign lipo plutil ditto xattr otool shasum security sed awk; do
  need_tool "$tool"
done
if [[ $MAKE_DMG -eq 1 ]]; then need_tool hdiutil; fi
if [[ $NOTARIZE -eq 1 ]]; then
  need_tool xcrun
  xcrun --find notarytool >/dev/null 2>&1 ||
    die "notarytool not found: install Xcode or the command line tools (xcode-select --install)"
  xcrun --find stapler >/dev/null 2>&1 || die "stapler not found: install Xcode or the command line tools"
fi

for f in "$PLIST_TEMPLATE" "$ENTITLEMENTS" "$STRINGS" "$NOTICES"; do
  [[ -f "$f" ]] || die "missing file: $f"
done

if [[ -n "$ICON_OVERRIDE" ]]; then
  ICON_SRC="$ICON_OVERRIDE"
  warn "ICON_PATH is a test-only override: using $ICON_SRC as the app icon"
else
  ICON_SRC="$PACKAGING/AppIcon.icns"
fi
[[ -f "$ICON_SRC" ]] || die "the app icon is missing: $ICON_SRC
  Add packaging/macos/AppIcon.icns (an .icns made with iconutil from a 1024x1024 image), then run again."
[[ "$(head -c 4 "$ICON_SRC")" == "icns" ]] || die "$ICON_SRC is not an .icns file"

if [[ $SIGN -eq 1 ]]; then
  identities="$(security find-identity -v -p codesigning 2>/dev/null || true)"
  if ! grep -qF "\"$SIGN_IDENTITY\"" <<<"$identities" && ! grep -qiF " $SIGN_IDENTITY " <<<"$identities"; then
    die "the signing certificate \"$SIGN_IDENTITY\" is not in the keychain.
  Import its exported .p12 into the login keychain (double-click it), check with: security find-identity -v -p codesigning
  or set APPLE_SIGNING_IDENTITY, or build an unsigned test app with --no-sign."
  fi
  info "Signing identity: $SIGN_IDENTITY"
else
  info "Signing identity: ad-hoc (--no-sign: runs on this Mac only)"
fi

# Notarization credentials: resolved now so a missing one stops the script before a long build.
# Values are never printed; only which method is used.
NOTARY_AUTH=()
NOTARY_METHOD=""
missing_credentials() {
  cat >&2 <<'EOF'
Error: no notarization credentials. Apple must notarize the app, or macOS refuses to open it on
any other Mac. Set one of these (first match wins):

  1. A notarytool keychain profile (recommended). Set it up once:
       xcrun notarytool store-credentials modifier-mes-pdf \
         --key <path/to/AuthKey_KEYID.p8> --key-id <KEYID> --issuer <issuer-uuid>
     then build with:
       NOTARY_PROFILE=modifier-mes-pdf pnpm mac:release
     The issuer UUID is in App Store Connect > Users and Access > Integrations > Team Keys.

  2. An App Store Connect API key:
       APPLE_API_KEY=<KEYID> APPLE_API_ISSUER=<issuer-uuid> pnpm mac:release
     The key file is APPLE_API_KEY_PATH, by default
     ~/.appstoreconnect/private_keys/AuthKey_<KEYID>.p8

  3. An Apple ID with an app-specific password (appleid.apple.com > Sign-In and Security):
       APPLE_ID=<email> APPLE_PASSWORD=<app-specific password> APPLE_TEAM_ID=86E98RSTFN pnpm mac:release

Or build a signed but NOT notarized app, to try on this Mac only:
       pnpm mac:app        (same as: ./scripts/build-macos.sh --no-notarize)
EOF
}
if [[ $NOTARIZE -eq 1 ]]; then
  if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    NOTARY_AUTH=(--keychain-profile "$NOTARY_PROFILE")
    NOTARY_METHOD="keychain profile \"$NOTARY_PROFILE\""
  elif [[ -n "${APPLE_API_KEY:-}" && -n "${APPLE_API_ISSUER:-}" ]]; then
    api_key_path="${APPLE_API_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_${APPLE_API_KEY}.p8}"
    [[ -f "$api_key_path" ]] ||
      die "the App Store Connect key file was not found (APPLE_API_KEY_PATH, by default
  ~/.appstoreconnect/private_keys/AuthKey_<APPLE_API_KEY>.p8)."
    NOTARY_AUTH=(--key "$api_key_path" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
    NOTARY_METHOD="App Store Connect API key (APPLE_API_KEY)"
  elif [[ -n "${APPLE_ID:-}" && -n "${APPLE_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]]; then
    NOTARY_AUTH=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
    NOTARY_METHOD="Apple ID with an app-specific password (APPLE_ID)"
  else
    if [[ -n "${APPLE_API_KEY:-}" || -n "${APPLE_API_ISSUER:-}" ]]; then
      warn "APPLE_API_KEY and APPLE_API_ISSUER must both be set."
    fi
    if [[ -n "${APPLE_ID:-}" || -n "${APPLE_PASSWORD:-}" || -n "${APPLE_TEAM_ID:-}" ]]; then
      warn "APPLE_ID, APPLE_PASSWORD and APPLE_TEAM_ID must all be set."
    fi
    missing_credentials
    exit 1
  fi
  info "Notarization: $NOTARY_METHOD"
else
  info "Notarization: skipped"
fi

tmp_root="${TMPDIR:-/tmp}"
WORK="$(mktemp -d "${tmp_root%/}/modifier-mes-pdf-build.XXXXXX")"

# ---------------------------------------------------------------------------------------------
# 2. The executable (cargo per architecture, then lipo)
# ---------------------------------------------------------------------------------------------

BIN="$WORK/p2d-cli"

if [[ -n "$PREBUILT" ]]; then
  step "2/9 Using the prebuilt binary (no cargo)"
  info "$PREBUILT"
  prebuilt_archs="$(lipo -archs "$PREBUILT" 2>/dev/null)" || die "--prebuilt: not a Mac executable: $PREBUILT"
  for a in "${ARCHS[@]}"; do
    has_word "$prebuilt_archs" "$a" ||
      die "--prebuilt binary contains [$prebuilt_archs] but --arch $ARCH needs $a (pass --arch $prebuilt_archs?)"
  done
  if [[ "$ARCH" != universal && "$prebuilt_archs" != "$ARCH" ]]; then
    lipo -thin "$ARCH" -output "$BIN" "$PREBUILT"
  else
    cp "$PREBUILT" "$BIN"
  fi
else
  step "2/9 Building p2d-cli (release) for ${ARCHS[*]}"
  # Homebrew's cargo lacks the Mac cross targets: use rustup's toolchain (rust-toolchain.toml).
  export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
  command -v rustup >/dev/null 2>&1 || die "rustup not found. Install it with: brew install rustup && rustup-init"
  command -v cargo >/dev/null 2>&1 || die "cargo not found (rustup is installed but has no toolchain)"
  installed_targets="$(rustup target list --installed)"
  for a in "${ARCHS[@]}"; do
    triple="$(triple_for "$a")"
    if ! grep -qx "$triple" <<<"$installed_targets"; then
      info "Adding the Rust target $triple..."
      rustup target add "$triple"
    fi
  done

  slices=()
  for a in "${ARCHS[@]}"; do
    triple="$(triple_for "$a")"
    info "cargo build --profile mac-app --locked -p p2d-cli --target $triple (macOS $(min_macos_for "$a") or later)"
    if [[ "$a" == x86_64 ]]; then
      # RUSTFLAGS replaces .cargo/config.toml's target-cpu=x86-64-v3 (AVX2, Haswell and later),
      # which would crash on older Intel Macs (the 2013 Mac Pro runs macOS 12 without AVX2).
      # x86-64-v2 runs on every Intel Mac that runs macOS 11.
      env -u CARGO_ENCODED_RUSTFLAGS \
        MACOSX_DEPLOYMENT_TARGET="$(min_macos_for x86_64)" \
        RUSTFLAGS="-C target-cpu=x86-64-v2" \
        cargo build --profile mac-app --locked -p p2d-cli --target "$triple"
    else
      env -u CARGO_ENCODED_RUSTFLAGS -u RUSTFLAGS \
        MACOSX_DEPLOYMENT_TARGET="$(min_macos_for arm64)" \
        cargo build --profile mac-app --locked -p p2d-cli --target "$triple"
    fi
    slice="$TARGET_DIR/$triple/mac-app/p2d-cli"
    [[ -f "$slice" ]] || die "cargo did not produce $slice"
    slices+=("$slice")
  done
  if [[ ${#slices[@]} -gt 1 ]]; then
    lipo -create -output "$BIN" "${slices[@]}"
  else
    cp "${slices[0]}" "$BIN"
  fi
fi

BIN_ARCHS="$(lipo -archs "$BIN")"
for a in "${ARCHS[@]}"; do
  has_word "$BIN_ARCHS" "$a" || die "the executable lacks $a (it has: $BIN_ARCHS)"
done
info "lipo -archs: $BIN_ARCHS"

# Each slice must start on the macOS version promised to Finder, or an old Mac shows a crash
# instead of « this app needs a newer macOS ».
MINOS_SUMMARY=""
for a in "${ARCHS[@]}"; do
  otool -arch "$a" -l "$BIN" >"$WORK/load-commands.txt"
  minos="$(awk '
    /cmd LC_BUILD_VERSION/ { bv = 1; vm = 0; next }
    /cmd LC_VERSION_MIN_MACOSX/ { vm = 1; bv = 0; next }
    /cmd / { bv = 0; vm = 0 }
    !found && bv && $1 == "minos" { print $2; found = 1 }
    !found && vm && $1 == "version" { print $2; found = 1 }
  ' "$WORK/load-commands.txt")"
  [[ -n "$minos" ]] || die "could not read the minimum macOS version of the $a slice"
  if version_gt "$minos" "$(min_macos_for "$a")"; then
    die "the $a slice needs macOS $minos but must run on macOS $(min_macos_for "$a").
  Build it with MACOSX_DEPLOYMENT_TARGET=$(min_macos_for "$a") (the cargo step does; check --prebuilt binaries)."
  fi
  MINOS_SUMMARY="${MINOS_SUMMARY:+$MINOS_SUMMARY, }$a macOS $minos+"
done
info "Minimum macOS per slice: $MINOS_SUMMARY"

# ---------------------------------------------------------------------------------------------
# 3. The .app bundle, from scratch
# ---------------------------------------------------------------------------------------------

step "3/9 Assembling $APP"
TOUCHED_DIST=1
mkdir -p "$DIST"
# The disk image of an earlier run stays until this one has passed every check (step 8).
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/fr.lproj"

sed -e "s/@VERSION@/$VERSION/g" -e "s/@BUILD@/$BUILD/g" "$PLIST_TEMPLATE" >"$APP/Contents/Info.plist"
if grep -q '@[A-Z_][A-Z_]*@' "$APP/Contents/Info.plist"; then
  die "Info.plist still has a @PLACEHOLDER@: update scripts/build-macos.sh or the template"
fi
printf 'APPL????' >"$APP/Contents/PkgInfo"
cp "$BIN" "$EXE"
chmod 755 "$EXE"
cp "$ICON_SRC" "$APP/Contents/Resources/AppIcon.icns"
cp "$STRINGS" "$APP/Contents/Resources/fr.lproj/InfoPlist.strings"
cp "$NOTICES" "$APP/Contents/Resources/THIRD_PARTY_NOTICES.txt"
chmod -R u+rwX,go+rX,go-w "$APP"

checked "a property list is invalid (see above)" \
  plutil -lint "$APP/Contents/Info.plist" "$APP/Contents/Resources/fr.lproj/InfoPlist.strings" "$ENTITLEMENTS"
plist_value() { plutil -extract "$1" raw -o - "$APP/Contents/Info.plist"; }
[[ "$(plist_value CFBundleExecutable)" == "$APP_NAME" ]] || die "CFBundleExecutable must be \"$APP_NAME\""
[[ "$(plist_value CFBundleIdentifier)" == "$BUNDLE_ID" ]] || die "CFBundleIdentifier must be $BUNDLE_ID"
[[ "$(plist_value CFBundleShortVersionString)" == "$VERSION" ]] || die "CFBundleShortVersionString is wrong"
[[ "$(plist_value CFBundleVersion)" == "$BUILD" ]] || die "CFBundleVersion is wrong"

# Finder info and quarantine attributes make codesign fail ("resource fork ... not allowed").
xattr -cr "$APP"

# ---------------------------------------------------------------------------------------------
# 4. Signature
# ---------------------------------------------------------------------------------------------

if [[ $SIGN -eq 1 ]]; then
  step "4/9 Signing with \"$SIGN_IDENTITY\" (hardened runtime)"
  info "If macOS asks for the keychain password, type it and click « Always Allow »."
  checked "codesign could not sign the app (offline? the timestamp needs the internet)" \
    codesign --force --timestamp --options runtime --entitlements "$ENTITLEMENTS" \
    --sign "$SIGN_IDENTITY" "$APP"
else
  step "4/9 Ad-hoc signature (--no-sign)"
  checked "codesign could not sign the app" codesign --force --sign - "$APP"
fi

checked "the app's signature does not verify" codesign --verify --strict --verbose=2 "$APP"
SIGN_DETAILS="$(codesign -d --verbose=4 "$APP" 2>&1)"
FLAGS_LINE="$(grep -m1 -E '^CodeDirectory ' <<<"$SIGN_DETAILS" || true)"
info "${FLAGS_LINE:-no CodeDirectory line}"
if [[ $SIGN -eq 1 ]]; then
  runtime_re='flags=0x[0-9a-f]+\([^)]*runtime'
  [[ "$FLAGS_LINE" =~ $runtime_re ]] || die "the signature lacks the hardened runtime flag"
  grep -m1 '^Authority=' <<<"$SIGN_DETAILS" | indent || true
  grep -m1 '^TeamIdentifier=' <<<"$SIGN_DETAILS" | indent || true
  ents="$(codesign -d --entitlements - --xml "$APP" 2>/dev/null || true)"
  ents_compact="$(tr -d ' \t\r\n' <<<"$ents")"
  [[ "$ents_compact" == *'<key>com.apple.security.automation.apple-events</key><true/>'* ]] ||
    die "the signature lacks the com.apple.security.automation.apple-events entitlement"
  info "Entitlement: com.apple.security.automation.apple-events = true"
fi

# ---------------------------------------------------------------------------------------------
# 5. Smoke test of the signed app
# ---------------------------------------------------------------------------------------------

HOST_ARM64="$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)"
can_run() {
  case "$1" in
    arm64) [[ "$HOST_ARM64" == 1 ]] ;;
    x86_64) [[ "$HOST_ARM64" != 1 ]] || arch -x86_64 /usr/bin/true >/dev/null 2>&1 ;;
    *) return 1 ;;
  esac
}

# Runs "<exe> --self-test" as architecture $1; a hang longer than SELF_TEST_TIMEOUT fails.
run_self_test() {
  local a="$1" out="$WORK/self-test-$1.out" err="$WORK/self-test-$1.err" rc=0 deadline
  arch "-$a" "$EXE" --self-test >"$out" 2>"$err" &
  SELF_TEST_PID=$!
  deadline=$((SECONDS + SELF_TEST_TIMEOUT))
  while kill -0 "$SELF_TEST_PID" 2>/dev/null; do
    if ((SECONDS >= deadline)); then
      {
        kill -TERM "$SELF_TEST_PID" || true
        sleep 2
        kill -KILL "$SELF_TEST_PID" || true
        wait "$SELF_TEST_PID" || true
      } 2>/dev/null
      SELF_TEST_PID=""
      cat "$out" "$err" | indent >&2
      die "the $a self-test did not finish within ${SELF_TEST_TIMEOUT}s (it was stopped)"
    fi
    sleep 0.25
  done
  wait "$SELF_TEST_PID" || rc=$?
  SELF_TEST_PID=""
  if [[ $rc -ne 0 ]]; then
    cat "$out" "$err" | indent >&2
    die "the $a self-test failed (exit code $rc)"
  fi
  local last
  last="$(grep -v '^[[:space:]]*$' "$out" | tail -n 1 || true)"
  if [[ "$last" != "{"*"}" ]]; then
    warn "the $a self-test passed but did not print a JSON line on stdout"
  fi
  info "$a: ${last:-<no output>}"
}

SELF_TEST_SUMMARY="skipped"
if [[ $SELF_TEST -eq 1 ]]; then
  step "5/9 Self-test of the signed app (--self-test)"
  SELF_TEST_SUMMARY=""
  for a in "${ARCHS[@]}"; do
    if can_run "$a"; then
      run_self_test "$a"
      note="ok"
      if [[ "$a" == x86_64 && "$HOST_ARM64" == 1 ]]; then note="ok (Rosetta)"; fi
    else
      note="not run (this Mac cannot run $a)"
      if [[ "$a" == x86_64 ]]; then
        note="not run (Rosetta missing: softwareupdate --install-rosetta --agree-to-license)"
      fi
      warn "$a self-test $note"
    fi
    SELF_TEST_SUMMARY="${SELF_TEST_SUMMARY:+$SELF_TEST_SUMMARY, }$a $note"
  done
else
  step "5/9 Self-test skipped (--skip-self-test)"
fi

# ---------------------------------------------------------------------------------------------
# 6. Notarization of the app
# ---------------------------------------------------------------------------------------------

# Prints the value of key $2 in the JSON file $1, or nothing (plutil prints its errors on stdout).
json_field() {
  local value
  if value="$(plutil -extract "$2" raw -o - "$1" 2>/dev/null)"; then printf '%s' "$value"; fi
}

# Sends $1 to Apple's notary service and waits for the verdict. $2 names it in file names, $3 in
# messages.
notarize() {
  local file="$1" json="$WORK/notary-$2.json" label="$3" status id
  info "Sending $label to Apple (usually 1 to 10 minutes)..."
  if ! xcrun notarytool submit "$file" "${NOTARY_AUTH[@]}" --wait --timeout 1h \
    --output-format json >"$json"; then
    warn "notarytool reported an error for $label"
  fi
  status="$(json_field "$json" status)"
  id="$(json_field "$json" id)"
  if [[ "$status" != "Accepted" ]]; then
    if [[ -s "$json" ]]; then
      printf 'notarytool answered:\n' >&2
      indent <"$json" >&2
    fi
    uuid_re='^[0-9A-Fa-f-]{36}$'
    if [[ "$id" =~ $uuid_re ]]; then
      printf 'Apple log for submission %s:\n' "$id" >&2
      xcrun notarytool log "$id" "${NOTARY_AUTH[@]}" 2>&1 | indent >&2 || true
    fi
    die "Apple did not accept $label (status: ${status:-unknown})"
  fi
  info "Accepted (submission $id)"
}

# Attaches Apple's ticket to $1 and checks it ($2 names it in messages). Both ask Apple's
# servers: right after « Accepted » the ticket can take a minute or two to get there (error 65 or
# 73), and a dropped connection fails the check (error 68). Try again for about three minutes
# before giving up.
staple() {
  local file="$1" label="$2" attempt out
  for attempt in 1 2 3 4 5 6 7 8 9 10 11 12; do
    if out="$(xcrun stapler staple "$file" 2>&1 && xcrun stapler validate "$file" 2>&1)"; then
      indent <<<"$out"
      return 0
    fi
    if ((attempt < 12)); then
      info "Apple's ticket for $label could not be stapled and checked yet (attempt $attempt/12): trying again in 15 s..."
      sleep 15
    fi
  done
  indent <<<"$out" >&2
  die "stapling the ticket to $label failed"
}

NOTARIZED="no"
if [[ $NOTARIZE -eq 1 ]]; then
  step "6/9 Notarizing the app"
  ditto -c -k --keepParent "$APP" "$WORK/PDF-Edit-app.zip"
  notarize "$WORK/PDF-Edit-app.zip" app "the app"
  staple "$APP" "the app"
  checked "the app's signature broke while stapling" codesign --verify --strict "$APP"
  NOTARIZED="yes (stapled)"
else
  step "6/9 Notarization skipped"
fi

# ---------------------------------------------------------------------------------------------
# 7. Disk image
# ---------------------------------------------------------------------------------------------

# Built and checked in the work folder; moved to dist/ only once it passed (step 8).
DMG_BUILD="$WORK/out/$(basename "$DMG")"
if [[ $MAKE_DMG -eq 1 ]]; then
  step "7/9 Creating $(basename "$DMG")"
  stage="$WORK/dmg"
  mkdir -p "$stage" "$WORK/out"
  ditto "$APP" "$stage/$APP_NAME.app"
  ln -s /Applications "$stage/Applications"
  # ULMO (LZMA) downloads a third smaller than UDZO (zlib) and opens on macOS 10.15 and later.
  # hdiutil sometimes fails with "Resource busy" while Spotlight looks at the new files: retry.
  attempt=1
  until hdiutil create -volname "$APP_NAME" -srcfolder "$stage" -fs HFS+ -format ULMO -ov "$DMG_BUILD" 2>&1 | indent; do
    if ((attempt >= 3)); then die "hdiutil could not create the disk image"; fi
    attempt=$((attempt + 1))
    warn "hdiutil failed, trying again ($attempt/3)..."
    sleep 3
  done
  checked "the disk image does not verify" hdiutil verify -quiet "$DMG_BUILD"
  if [[ $SIGN -eq 1 ]]; then
    checked "codesign could not sign the disk image" \
      codesign --force --timestamp --sign "$SIGN_IDENTITY" "$DMG_BUILD"
    checked "the disk image's signature does not verify" codesign --verify --strict --verbose=2 "$DMG_BUILD"
  fi
  if [[ $NOTARIZE -eq 1 ]]; then
    notarize "$DMG_BUILD" dmg "the disk image"
    staple "$DMG_BUILD" "the disk image"
  fi
else
  step "7/9 Disk image skipped (--no-dmg)"
fi

# ---------------------------------------------------------------------------------------------
# 8. Gatekeeper, as another Mac will see the files
# ---------------------------------------------------------------------------------------------

if [[ $NOTARIZE -eq 1 ]]; then
  step "8/9 Gatekeeper checks"
  gk_app="$(spctl -a -vvv -t exec "$APP" 2>&1)" || {
    indent <<<"$gk_app" >&2
    die "Gatekeeper refuses the app"
  }
  indent <<<"$gk_app"
  grep -q 'source=Notarized Developer ID' <<<"$gk_app" || die "Gatekeeper does not see the app as notarized"
  if [[ $MAKE_DMG -eq 1 ]]; then
    gk_dmg="$(spctl -a -vvv -t open --context context:primary-signature "$DMG_BUILD" 2>&1)" || {
      indent <<<"$gk_dmg" >&2
      die "Gatekeeper refuses the disk image"
    }
    indent <<<"$gk_dmg"
    grep -q 'source=Notarized Developer ID' <<<"$gk_dmg" ||
      die "Gatekeeper does not see the disk image as notarized"
  fi
elif [[ $SIGN -eq 1 ]]; then
  step "8/9 Gatekeeper (for information: refused until notarized)"
  spctl -a -vvv -t exec "$APP" 2>&1 | indent || true
else
  step "8/9 Gatekeeper checks skipped (ad-hoc signature)"
fi

# ---------------------------------------------------------------------------------------------
# 9. Summary
# ---------------------------------------------------------------------------------------------

if [[ $MAKE_DMG -eq 1 ]]; then
  mv -f "$DMG_BUILD" "$DMG"
fi
TOUCHED_DIST=0
step "9/9 Done"
info "App:          $APP"
if [[ $MAKE_DMG -eq 1 ]]; then
  info "Disk image:   $DMG"
  info "SHA-256:      $(shasum -a 256 "$DMG" | awk '{print $1}')"
fi
info "Version:      $VERSION (build $BUILD)"
info "Archs:        $BIN_ARCHS ($MINOS_SUMMARY)"
if [[ -n "$PREBUILT" ]]; then
  info "Binary:       --prebuilt $PREBUILT (packaging test, not a release build)"
fi
if [[ $SIGN -eq 1 ]]; then
  info "Signed by:    $SIGN_IDENTITY"
else
  info "Signed by:    ad-hoc (this Mac only)"
fi
info "Notarized:    $NOTARIZED"
info "Self-test:    $SELF_TEST_SUMMARY"
if [[ $NOTARIZE -eq 0 ]]; then
  info "Not notarized: fine on this Mac, but do not send it to anyone (pnpm mac:release does)."
fi

if [[ $OPEN_AT_END -eq 1 ]]; then
  if [[ $MAKE_DMG -eq 1 ]]; then open -R "$DMG"; else open -R "$APP"; fi
fi
