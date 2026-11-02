# PDF Edit — macOS Release Pipeline (v1, 2026-10-26)

How « PDF Edit » becomes a Mac app that someone can download and open with a double-click: one universal binary, a **Developer ID** signature with the **hardened runtime**, **Apple notarization** with stapled tickets, and a signed disk image. Everything is done by [`scripts/build-macos.sh`](../scripts/build-macos.sh); the bundle metadata lives in [`packaging/macos/`](../packaging/macos/).

No step uploads documents: notarization sends Apple the app itself (the executable and its resources), never a user file.

---

## 1. What the Pipeline Does

| Step | What happens | Skipped with |
|---|---|---|
| **1. Checks** | Reads the version from `Cargo.toml` (`[workspace.package] version` → `CFBundleShortVersionString`) and the build number (`git rev-list --count HEAD` → `CFBundleVersion`, `1` outside git). Checks the tools, the icon, the signing certificate and the notarization credentials **before** compiling, so a missing piece stops the script in a second rather than after a long build. | — |
| **2. Build** | `cargo build --profile mac-app --locked -p p2d-cli --target <triple>` per architecture (`mac-app` is `release` with fat LTO and stripped symbols, see §7): `x86_64` with `MACOSX_DEPLOYMENT_TARGET=11.0` and `-C target-cpu=x86-64-v2`, `arm64` with `MACOSX_DEPLOYMENT_TARGET=11.0`, both with `--remap-path-prefix` so the binary shows `~/…` and `./…` instead of this Mac's home and checkout paths (checked with `strings` after each build). Then `lipo -create`, checked with `lipo -archs`, and each slice's minimum macOS (`LC_BUILD_VERSION minos`) is checked against what the bundle promises. | `--prebuilt <binary>` |
| **3. Bundle** | Rebuilds `dist/PDF Edit.app` from scratch: `Info.plist` from the template, `PkgInfo`, the executable, `AppIcon.icns`, `fr.lproj/InfoPlist.strings`, `THIRD_PARTY_NOTICES.txt`. `plutil -lint` on every property list, then `xattr -cr`. | — |
| **4. Sign** | `codesign --force --timestamp --options runtime --entitlements packaging/macos/entitlements.plist --sign "Developer ID Application: Makio (86E98RSTFN)"`, then `codesign --verify --strict` and a check that the signature carries the `runtime` flag and the Apple Events entitlement. | `--no-sign` (ad-hoc signature instead) |
| **5. Self-test** | Runs `"<app>/Contents/MacOS/PDF Edit" --self-test` on the **signed** app (hardened runtime included), natively and, for universal builds, again under Rosetta (`arch -x86_64`). A run that hangs longer than 90 s is killed and fails the build. | `--skip-self-test` |
| **6. Notarize the app** | `ditto -c -k --keepParent` to a zip, `xcrun notarytool submit --wait`; anything but `Accepted` prints Apple's log and stops. Then `xcrun stapler staple` + `validate` on the `.app`. | `--no-notarize`, `--no-sign` |
| **7. Disk image** | A folder with the stapled app and an `Applications` shortcut → `hdiutil create -fs HFS+ -format ULMO` → `dist/PDF-Edit-<version>.dmg`, signed with the same identity, then notarized, stapled and validated like the app. It is built in a temporary folder and moved into `dist/` only after every check passed, so a failed run never replaces a good image. Without notarization it is named `PDF-Edit-<version>-NON-NOTARISE.dmg`, never the release name. | `--no-dmg` |
| **8. Gatekeeper** | `spctl -a -t exec` on the app and `spctl -a -t open --context context:primary-signature` on the DMG must both say `source=Notarized Developer ID` (shown for information only when not notarized). | — |
| **9. Summary** | Paths, architectures, version and build, signing identity, notarized or not, self-test results, SHA-256 of the DMG. | — |

If any step fails, the script says that `dist/` is incomplete: never share what a failed run left there.

---

## 2. Prerequisites (Once per Mac)

| What | How to get it | How to check |
|---|---|---|
| **Xcode command line tools** (`codesign`, `lipo`, `notarytool`, `stapler`, `hdiutil`) | `xcode-select --install` (or Xcode itself) | `xcrun --find notarytool` |
| **rustup + both Mac targets** | `brew install rustup && rustup-init`. The script puts `/opt/homebrew/opt/rustup/bin` and `~/.cargo/bin` first in `PATH` (Homebrew's own `cargo` lacks the cross targets) and runs `rustup target add` for any missing target. `rust-toolchain.toml` pins stable and both targets. | `rustup target list --installed` |
| **Developer ID Application certificate** | "Developer ID Application: Makio (86E98RSTFN)". If this Mac lacks it, double-click its exported `.p12` to import it into the login keychain. | `security find-identity -v -p codesigning` |
| **Notarization credentials** | See below. | `xcrun notarytool history --keychain-profile modifier-mes-pdf` |
| **Rosetta 2** (Apple Silicon only, for the `x86_64` self-test) | `softwareupdate --install-rosetta --agree-to-license` | `arch -x86_64 /usr/bin/true` |
| **Free disk space** | A universal release build keeps two full release trees in `target/`: plan for several GB. | `df -h .` |

### Notarization Credentials

The script uses the first of these that is set. Values are never printed.

1. **`NOTARY_PROFILE`** — a `notarytool` keychain profile (**recommended**: nothing secret in the environment or in `ps`). Set it up once with an App Store Connect API key:
   ```sh
   xcrun notarytool store-credentials modifier-mes-pdf \
     --key <path/to/AuthKey_KEYID.p8> --key-id <KEYID> --issuer <issuer-uuid>
   ```
   then build with `NOTARY_PROFILE=modifier-mes-pdf pnpm mac:release` (or put `export NOTARY_PROFILE=modifier-mes-pdf` in `~/.zshrc`). The **issuer UUID** is shown in App Store Connect › Users and Access › Integrations › **Team Keys**, above the list of keys.
2. **`APPLE_API_KEY` + `APPLE_API_ISSUER`** (+ optional **`APPLE_API_KEY_PATH`**) — the `.p8` file defaults to `~/.appstoreconnect/private_keys/AuthKey_${APPLE_API_KEY}.p8`.
3. **`APPLE_ID` + `APPLE_PASSWORD` + `APPLE_TEAM_ID`** — an Apple ID with an **app-specific** password (appleid.apple.com › Sign-In and Security), team `86E98RSTFN`. The password is passed to `notarytool` on its command line, which is why option 1 is preferred.

Without any of them, `pnpm mac:release` stops before building and explains these three options; `pnpm mac:app` (no notarization) still works.

---

## 3. Commands

| Command | Result |
|---|---|
| `pnpm mac:app` | `./scripts/build-macos.sh --no-notarize`: universal, signed, self-tested `.app` + DMG. Opens on this Mac; **other Macs refuse it** (not notarized). For trying a build. |
| `pnpm mac:release` | `./scripts/build-macos.sh`: the same, notarized and stapled. **The only build to send to anyone.** |

Options (after `pnpm mac:app` / `pnpm mac:release`, or on the script directly):

| Option | Effect |
|---|---|
| `--arch universal\|arm64\|x86_64` | Architectures to build (default `universal`). |
| `--no-sign` | Ad-hoc signature (`codesign --sign -`) so the arm64 build still launches; runs on this Mac only. Implies `--no-notarize`. |
| `--no-notarize` | Developer ID signature, no notarization. |
| `--no-dmg` | Stop after the `.app`. |
| `--skip-self-test` | Do not run `--self-test` after signing. |
| `--prebuilt <binary>` | Skip cargo and package this `p2d-cli` binary (thinned with `lipo` if it holds more architectures than `--arch` asks for). For testing the packaging only. |
| `--open` | Reveal the DMG (or the app) in the Finder at the end. |
| `-h`, `--help` | List the options and variables. |

Environment variables besides the credentials:

| Variable | Use |
|---|---|
| `APPLE_SIGNING_IDENTITY` | Another signing identity (name or SHA-1). Default: `Developer ID Application: Makio (86E98RSTFN)`. |
| `ICON_PATH` | **Test-only**: package this `.icns` instead of `packaging/macos/AppIcon.icns`. Without it, a missing `AppIcon.icns` stops the build. |
| `SELF_TEST_TIMEOUT` | Seconds before a hanging self-test is killed (default `90`). |
| `CARGO_TARGET_DIR` | Honoured when looking for the cargo outputs. |

A packaging-only dry run, without compiling and without Apple:

```sh
ICON_PATH=/System/Applications/Preview.app/Contents/Resources/AppIcon.icns \
  ./scripts/build-macos.sh --prebuilt target/release/p2d-cli --arch arm64 --no-notarize --skip-self-test
```

---

## 4. Outputs

Everything lands in `dist/` (ignored by git), rebuilt on each run:

```
dist/
  PDF Edit.app/
    Contents/
      Info.plist                     from packaging/macos/Info.plist.in (@VERSION@, @BUILD@)
      PkgInfo                        APPL????
      MacOS/PDF Edit         p2d-cli, universal (x86_64 + arm64); all UI assets are embedded
      Resources/AppIcon.icns
      Resources/fr.lproj/InfoPlist.strings
      Resources/THIRD_PARTY_NOTICES.txt
      _CodeSignature/                codesign
      CodeResources                  notarization ticket (after stapling)
  PDF-Edit-<version>.dmg     volume « PDF Edit »: the app + an Applications shortcut
```

- **What to send** is the DMG from `pnpm mac:release`. The summary prints its SHA-256.
- **Version**: bump `[workspace.package] version` in `Cargo.toml` before a release (one to three numbers, such as `0.2.0`). The build number follows the commit count by itself.
- Launched from the Finder with no arguments, the executable opens the app's native window; `--self-test` is the only argument the pipeline relies on.

---

## 5. Verifying a Build

The script runs all of these; to check a DMG again later, or on another Mac:

```sh
APP="dist/PDF Edit.app"; DMG=dist/PDF-Edit-1.0.0.dmg

codesign --verify --strict --verbose=2 "$APP"       # valid on disk, satisfies its Designated Requirement
codesign -dvv "$APP"                                # flags=0x10000(runtime), Authority=Developer ID Application: Makio (86E98RSTFN)
codesign -d --entitlements - --xml "$APP"           # only com.apple.security.automation.apple-events
lipo -archs "$APP/Contents/MacOS/PDF Edit"  # x86_64 arm64
xcrun stapler validate "$APP"                       # The validate action worked!
xcrun stapler validate "$DMG"
spctl -a -vvv -t exec "$APP"                        # accepted, source=Notarized Developer ID
spctl -a -vvv -t open --context context:primary-signature "$DMG"   # accepted, source=Notarized Developer ID
hdiutil verify "$DMG"                               # checksum ... is VALID
```

Final check on an Intel Mac: copy the DMG over (AirDrop or Mail, so it gets the quarantine flag like a download), double-click it, drag the app to Applications, open it. There must be no « cannot be opened » dialog, only the usual « downloaded from the Internet » confirmation the first time.

---

## 6. Troubleshooting

| Symptom | Cause and fix |
|---|---|
| A **keychain dialog** appears during signing, or signing seems to hang | `codesign` needs the certificate's private key. Type the login password and click **Always Allow** (once per Mac). Over SSH there is no dialog: run `security unlock-keychain ~/Library/Keychains/login.keychain-db` first, after one **Always Allow** given at the Mac itself. |
| `The timestamp service is not available` | `--timestamp` asks Apple's server: needs the internet. Try again. |
| The script stops at step 1 about **notarization credentials** | None of the three methods in §2 is set. Use `pnpm mac:app` to try a build without notarization. |
| `No Keychain password item found for profile` | `NOTARY_PROFILE` names a profile this Mac does not have: run `xcrun notarytool store-credentials` (§2). |
| **Notarization rejected** (`status: Invalid`) | The script prints `notarytool log <id>`; it lists each problem file. Again later: `xcrun notarytool log <id> --keychain-profile modifier-mes-pdf`, and `xcrun notarytool history ...` for past submissions. Typical causes: a nested executable that is not signed, a signature without `--timestamp` or without the hardened runtime. |
| `stapler staple` or `validate` fails right after `Accepted` (`Error 65`, `73`, or `68` with « The network connection was lost ») | Apple's ticket can take a minute or two to reach its servers, and both commands need them. The script retries for about three minutes; if it still fails, wait, then rerun `pnpm mac:release` (or staple by hand: `xcrun stapler staple "dist/PDF Edit.app"`). |
| `x86_64 self-test not run (Rosetta missing)` | Install Rosetta: `softwareupdate --install-rosetta --agree-to-license`. Until then the Intel slice is built but not smoke-tested on this Mac. |
| `self-test did not finish within 90s` | `--self-test` hung (a window or a dialog waiting, a lock). Run `"dist/PDF Edit.app/Contents/MacOS/PDF Edit" --self-test` by hand to see it. `SELF_TEST_TIMEOUT` raises the limit. |
| `the x86_64 slice needs macOS 1x.y` | A binary was built without `MACOSX_DEPLOYMENT_TARGET=11.0` (typically a `--prebuilt` one). Let the script run cargo. |
| The app quits at once on an older Intel Mac (`Illegal instruction`) | It was built with `.cargo/config.toml`'s `x86-64-v3`. Release builds must come from this script, which overrides it (§7). |
| `hdiutil: create failed - Resource busy` | Retried three times automatically. Otherwise close Finder windows showing a « PDF Edit » volume and eject it (`hdiutil info`, `hdiutil detach`). |
| Gatekeeper says `source=Unnotarized Developer ID` | Expected for `pnpm mac:app`. Use `pnpm mac:release`. |

---

## 7. Why These Choices

| Decision | Why |
|---|---|
| **`x86_64` built with `target-cpu=x86-64-v2`** | `.cargo/config.toml` sets `x86-64-v3` (AVX2, FMA, BMI: Haswell, 2013 and later) for fast local builds. Macs that run macOS 11 include the late-2013 Mac Pro (Ivy Bridge-EP, no AVX2), where a v3 binary dies with `Illegal instruction`. v2 (SSE4.2, POPCNT) runs on every Intel Mac. The `CARGO_ENCODED_RUSTFLAGS` variable replaces the config's `rustflags` entirely, which is the intent. |
| **Minimum macOS 11.0** (`LSMinimumSystemVersion`, `MACOSX_DEPLOYMENT_TARGET`) | The PDF editor positions every line with CSS container units (`cqw`), which WebKit only has from Safari 16; macOS 11 Big Sur and later get Safari 16+ through Software Update, macOS 10.15 never does. Converting to Pages or Word would still work on 10.15, but a half-working app is worse than a clear « needs macOS 11 » from the Finder. On macOS 11 with an outdated Safari, the editor says to update instead of showing misplaced lines. Each slice's `minos` is checked so the Finder never promises more than the binary delivers. |
| **`mac-app` profile: fat LTO, stripped symbols** | 18 % smaller than `release` (arm64 slice 9.6 → 7.9 MB) at the same speed, measured on `qa/reports/fidelity` (convert + render). `opt-level = "s"` would save another 15 % but makes conversion and rendering about twice as slow: not worth it. Panics still log their message and source line in `~/Library/Logs/Modifier mes PDF`; backtraces lose function names. |
| **ULMO disk image (LZMA)** | A third smaller than UDZO (10.5 → 7.0 MB for 1.1.0). It needs macOS 10.15, below the app's own minimum. |
| **Universal binary** | One download for Intel Macs and for any Apple Silicon Mac. |
| **`LSHandlerRank` `Alternate`** for PDF | The app appears in « Ouvrir avec » for PDF files but never becomes their default: double-clicking a PDF keeps opening Preview, as people are used to. |
| **Hardened runtime, one entitlement** | Notarization requires the hardened runtime. `com.apple.security.automation.apple-events` lets the app (through `osascript`) drive Pages to create and open `.pages` files. No sandbox (the app saves where the person chose and runs `osascript`), no JIT entitlement (WKWebView's JIT runs in Apple's own WebContent process). |
| **French usage strings** | macOS shows them word for word in its permission dialogs: short, plain French that says what the app does with the folder, for a non-technical reader. |
| **Notarize the app, then the DMG** | The app inside the DMG carries its own stapled ticket, so it still opens offline once copied to Applications; the DMG's ticket lets Gatekeeper accept the disk image itself. |
| **Self-test after signing** | The hardened runtime can stop a binary that ran fine unsigned (library validation, entitlements). Testing the signed bundle, under Rosetta too, catches that before Apple or the user does. |
| **`CFBundleVersion` = commit count** | Always grows, needs no manual bump. |
