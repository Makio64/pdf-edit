# PDF Edit — Website (v2, 2026-11-02)

The public website of **PDF Edit**, in French, a few simple pages under one menu: a home page that offers the Mac app with screenshots of it, the same three-step app running **inside the browser** on the page « Version en ligne » (`/en-ligne/`, the Rust engine compiled to WebAssembly), the questions, a privacy and legal page, and a private dashboard of first-party, cookieless counts. It is hosted on Netlify (project `pdf-edit-mac`, at **https://pdf-edit-mac.netlify.app**).

Everything static is assembled on a Mac by [`scripts/build-web.sh`](../scripts/build-web.sh), because it needs the Rust/WASM toolchain and the notarized disk image; Netlify only serves the files and runs three small functions. Documents never reach the site: the in-browser app works on the visitor's computer, and the counts carry nothing about the files.

---

## 1. What Is Where

| URL | What | Comes from |
|---|---|---|
| `/` | Home (« Accueil »): hero, screenshots, the three steps, correcting in the PDF, privacy, download | `site/src/index.html` |
| `/en-ligne/` | « Version en ligne »: the site's menu, then the in-browser app in a frame (`<iframe src="/app/">`) filling the window (« The Online Version » below) | `site/src/en-ligne/index.html` |
| `/questions/` | « Questions »: the questions and answers (FAQPage JSON-LD) | `site/src/questions/index.html` |
| `/mentions-legales/` | « Confidentialité »: legal notice, privacy, what is counted, the opt-out button | `site/src/mentions-legales/index.html` |
| `/img/*.webp` | The screenshots of the app (1600 × 1047), made by `site/shots/` (section 10) | `site/src/img/` |
| `/app/` | The in-browser app, screens 1 to 3, shown inside `/en-ligne/`. `noindex`, canonical `/en-ligne/`; opened on its own, it sends the visitor to `/en-ligne/` | `extension/ui/drop.html`, copied by the build as `app/index.html` |
| `/app/edit.html` | The in-browser editor (« Corriger quelques mots »), in the same frame | `extension/ui/edit.html` |
| `/app/drop.html` | Same page as `/app/`: the editor's « Retour » and « Modifier un autre PDF » link to it | `extension/ui/drop.html` |
| `/stats/` | Private dashboard (`noindex`, not in the sitemap) | `site/src/stats/index.html`, `site/src/assets/stats.js` |
| `/404.html` | Not-found page | `site/src/404.html` |
| `/telecharger/mac?from=<where>` | Counts one Mac download, then `302` to the disk image | function `site/netlify/functions/download.mts` |
| `/downloads/<file>.dmg` | The disk image itself, while the repository is private | copied by the build (`--dmg`) |
| `/api/e` | Receives the anonymous events (`POST`) | function `event.mts` |
| `/api/stats` | Aggregated counts for the dashboard (`GET`, key required) | function `stats.mts` |
| `/api/app-info` | **Never served, on purpose**: the app pages ask for it and, getting no answer, switch to the in-browser engine. No function may answer it (no catch-all `/api/*`). | — |

```
site/
  netlify.toml            base directory of the Netlify project: publish dist/, functions netlify/functions/
  package.json            @netlify/blobs for the functions; puppeteer-core (dev) for the screenshots (npm, package-lock.json)
  src/                    static sources, copied into dist/ with the placeholders filled in
    index.html, en-ligne/, questions/, mentions-legales/, 404.html, stats/   the pages
    img/                  the app's screenshots (section 10)
    _headers              response headers (section 7)
    assets/site.css       the one stylesheet of the pages
    assets/track.js       the counting beacon (section 5)
    assets/optout.js      the « Ne plus être compté » button of the privacy page (section 5)
    assets/tour.js        the home page's slideshow: the app's four screens under the title, one dot per step
  app-overlay/
    wasm-client.js        wraps the app's engine bridge to count conversions (section 5)
  netlify/functions/      event.mts, stats.mts, download.mts
  netlify/lib/            their shared modules; release.generated.mts is written by the build
  test/                   node --test unit tests of the counting logic (cd site && npm test)
  shots/                  remakes the screenshots from the real app (section 10)
  og/og.html              source of src/og.png (1200 × 630, the hero screenshot), not deployed; its first lines hold the Chrome command that renders it
  dist/                   the build output (ignored by git)
scripts/build-web.sh      the build
```

Every page has the same menu at the top: « PDF Edit » (home), « Accueil », « Version en ligne », « Questions », « Confidentialité » and the button « Télécharger pour Mac ». The pages are light only, with no external font or script; their only scripts are `/assets/track.js`, `/assets/tour.js` on the home page and `/assets/optout.js` on the privacy page.

`assets/site.css` (light only, `color-scheme: light`, no dark-mode rules) also styles `/stats/`: the dashboard's table cells use `.table .num`, so the pages' own classes keep other names (the home page's slideshow dots are `.tour-dot`). The French text uses typographic spaces: a no-break space (U+00A0) before « : » and inside « », a narrow no-break space (U+202F) before « ? », « ! » and « ; ». Type them when editing text by hand, and keep the FAQPage JSON-LD of `/questions/` word for word the same as the visible questions and answers.

The app pages are never edited for the website: the build copies them from `extension/ui` and changes the copies (section 2). Fix the app in `extension/ui`, commit, rebuild.

### The Online Version: One Page, One Frame

`/en-ligne/` is the page people see, share and find in search engines; the app itself stays the set of pages of `extension/ui`, unchanged, under `/app/`, and runs in an `<iframe src="/app/">` below the site's menu. Inside the frame the app works exactly as on its own: same origin, so the same storage, downloads and folder picker (the frame must have no `sandbox` attribute; the build refuses one). The editor opens inside the frame too, and its « Retour » comes back to `/app/drop.html` there.

The build adds to the three app pages, first thing in `<head>`, a small script:

- **Framed** (`window.top !== window.self`): `<html>` gets the class `in-frame`, and an injected style hides the app's own brand link (`.in-frame .brand{visibility:hidden}`), since the site's menu just above already shows it. On phones the app keeps its own header.
- **Opened on its own** at `/app/`, `/app/index.html`, `/app/drop.html` or `/app/drop` (Netlify also serves `drop.html` there; an old link or bookmark): `location.replace('/en-ligne/' + location.search + location.hash)`. Before leaving it marks the page `data-no-pageview` (so the beacon sends nothing from a page that is going away) and sets the referrer policy to `no-referrer`, so `/en-ligne/` arrives without a referrer and counts as an entry (`direct`) rather than as an internal page view coming from `/app/`. The original referrer of such a visit is lost: a script redirect always gives the page it leaves as referrer. The frame itself always opens `/app/`, so an old link to a later screen (`drop.html#choix`) starts again at screen 1.
- **The editor opened on its own** (`/app/edit.html`, unframed) stays where it is and is counted as `/app/edit.html`.

The app pages also say `noindex` (and `/app/*` sends `X-Robots-Tag: noindex`), their canonical address is `/en-ligne/`, and the sitemap does not list them. `robots.txt` does **not** disallow `/app/`, on purpose: a crawler that may not fetch a page never sees its `noindex`, its canonical or its redirect, and can keep the bare address in its results (« indexed, though blocked by robots.txt »). Left crawlable, `/app/` folds into `/en-ligne/`.

---

## 2. Building

### Prerequisites (once per Mac)

| What | Why | How |
|---|---|---|
| `python3`, `git`, `shasum` | page rewriting, placeholders, checks | Apple's command line tools: `xcode-select --install` |
| rustup with the `wasm32-unknown-unknown` target | the engine (not needed with `--pkg`) | `brew install rustup && rustup-init`; `rust-toolchain.toml` lists the target, so rustup installs it |
| `wasm-bindgen` CLI, **same version** as the `wasm-bindgen` crate in `Cargo.lock` (0.2.129 today) | the engine's JavaScript bindings | `cargo install wasm-bindgen-cli --version <version>` (or Homebrew's `wasm-bindgen` when versions match) |
| `wasm-opt` | shrinks and speeds up the engine | `brew install binaryen` |
| Netlify CLI, logged in, `site/` linked | deploying | `npm install -g netlify-cli`, `netlify login`, then `cd site && netlify link --name pdf-edit-mac` |
| `site/node_modules` | the CLI bundles the functions with `@netlify/blobs` at deploy time | `cd site && npm install` |

### Commands

| Command | Result |
|---|---|
| `pnpm web:build` | A **preview**: the app pages of the working tree (uncommitted changes included), the engine built from the working tree, and the example release facts of `site/netlify/lib/release.example.mts`. The download link leads nowhere. Never deploy it to production. |
| `scripts/build-web.sh --ui-rev HEAD --dmg dist/PDF-Edit-<version>.dmg` | **A release build**: the pages and the engine of the commit `HEAD` (the engine is built from a `git archive` of that commit, whatever the working tree holds), version, size and SHA-256 from the notarized disk image, which is copied into `dist/downloads/`. |
| `scripts/build-web.sh --ui-rev HEAD --pkg <dir> --dmg dist/PDF-Edit-<version>.dmg` | The same with an engine already built (the `wasm-bindgen --target web` output, `p2d_wasm.js` + `p2d_wasm_bg.wasm`). Only pass a package built **from the same commit**: the build stops when the package lacks a function the pages call, but cannot see subtler differences. |

The disk image is the one of `pnpm mac:release` ([RELEASE-macOS.md](RELEASE-macOS.md)); the older name `Modifier-mes-PDF-<version>.dmg` is accepted too. The build refuses an image named `…-NON-NOTARISE.dmg` and, on a Mac, one without a stapled notarization ticket (`xcrun stapler validate`).

| Option | Meaning |
|---|---|
| `--ui-rev <rev>` | Git revision of `extension/ui` and of the engine (default: the working tree) |
| `--pkg <dir>` | Prebuilt engine instead of `cargo build --profile release-wasm` + `wasm-bindgen` + `wasm-opt` (built in `target/build-web/`, never in `extension/pkg`) |
| `--dmg <file>` | The release disk image (without it: the example release facts) |
| `--site-url <url>` | Public address, no trailing slash (default `https://pdf-edit-mac.netlify.app`; change it with a custom domain) |
| `--github-public` or `GITHUB_PUBLIC=1` | Keep the `<!--IF_GITHUB-->` blocks (section 8) |
| `--dmg-hosting site\|github` | Where `/telecharger/mac` sends to: `dist/downloads/<file>` (default while private) or the GitHub release `v<version>` (default with `--github-public`) |
| `--out <dir>` | Output folder (default `site/dist`). It is emptied first; the build refuses `/`, the repository and folders that do not look like an earlier build |
| `--site-dir <dir>` | Another copy of `site/` (tests only) |

### What the Build Does

| Step | What happens |
|---|---|
| **1. Checks** | Options, the revision, the package, the disk image (name, version, size, SHA-256, notarization), the output folder: all before anything is built or deleted. |
| **2. Engine** | `--pkg`, or `cargo build --profile release-wasm --target wasm32-unknown-unknown -p p2d-wasm` (with `--locked` for a revision), `wasm-bindgen --target web`, `wasm-opt -O3 --enable-simd --enable-bulk-memory --enable-nontrapping-float-to-int`, like [`build-extension.sh`](../scripts/build-extension.sh). |
| **3. Static pages** | Empties the output folder and copies `site/src` into it. |
| **4. In-browser app** | Copies `drop.html drop.css drop.js edit.html edit.css edit.js wasm-client.js` into `app/`; `drop.html` also becomes `app/index.html`. `wasm-client.js` is renamed `wasm-client.core.js` and `site/app-overlay/wasm-client.js` takes its place. The engine goes to `app/pkg/` (no `.d.ts`). In the three pages: the frame script right after `<meta charset>` (section 1, « The Online Version »); `noindex` and canonical `/en-ligne/`; the header's `<span class="brand">…</span>` becomes `<a class="brand" href="/">PDF Edit</a>` with a small style (same look, focus ring, hidden in the frame, and on its own on a phone still shown, centred, as the way back to the site; in the editor it also keeps the phone save bar to the height of its content, `.save-bar .save-info{flex:none}`, until `extension/ui/edit.css` does it itself); new `<title>`; favicons and manifest; `<script type="module" src="/assets/track.js">`. The example-PDF buttons are removed from `#samplesBox` (the element stays: `drop.js` uses it) and `#samplesBox{display:none!important}` hides it. No sample PDF is copied. |
| **5. Release facts** | Writes `site/netlify/lib/release.generated.mts` (`version`, `file`, `url`, `bytes`, `sha256`), which `download.mts` imports; without `--dmg` it holds the facts of `release.example.mts` (with the GitHub release address when `--dmg-hosting` is `github`). With `--dmg-hosting site` the image is copied to `downloads/`. |
| **6. Placeholders** | In every `.html .xml .txt .json .webmanifest .js` file: the values below, then the `<!--IF_GITHUB-->…<!--/IF_GITHUB-->` blocks are kept without their markers (public) or removed whole (private). |
| **7. Checks** | The build **fails** when: a `{{NAME}}` placeholder or an `IF_GITHUB` marker is left anywhere; the repository's address appears while it is private; a `.pdf` file is in the output; `index.html`, `en-ligne/index.html`, `questions/index.html`, `sitemap.xml`, `_headers`, `assets/track.js`, `app/index.html`, `app/drop.html`, `app/edit.html`, the two `wasm-client` files or the engine are missing; an app page does not load `/assets/track.js`, or lacks the frame script or its canonical `/en-ligne/`; an `api/` folder exists; an app page links to a file that is not there; the copied image differs in size. And, in the site's own pages (everything outside `app/` and `downloads/`): a link, stylesheet, script, frame, `og:image` or image points at a file that is not in the output (`/telecharger/`, `/api/` and `/downloads/` excepted); an `<img>` comes from another site or lacks a numeric `width` and `height`; `/`, `/en-ligne/`, `/questions/`, `/mentions-legales/` or `/404.html` does not load `<script type="module" src="/assets/track.js">`; `/en-ligne/` has no `<iframe src="/app/">`, or its frame has no `title` or has a `sandbox`; `sitemap.xml` does not list `/`, `/en-ligne/`, `/questions/` and `/mentions-legales/`, or lists `/app/`, `/stats/` or `/downloads/`. Then a summary: pages, version, size, SHA-256, where the download goes, GitHub public or not, size of the site. |

| Placeholder | Value |
|---|---|
| `{{SITE_URL}}` | `--site-url`, no trailing slash |
| `{{VERSION}}` | from the disk image's name, e.g. `1.2.1` |
| `{{DMG_FILE}}` | e.g. `PDF-Edit-1.2.1.dmg` |
| `{{DMG_SIZE}}` | French, decimal megabytes like the Finder, with a no-break space: `6,9 Mo` |
| `{{DMG_BYTES}}` | size in bytes |
| `{{DMG_SHA256}}` | SHA-256 of the image |
| `{{RELEASE_DATE}}` | modification date of the image (UTC), `YYYY-MM-DD`; the build date without `--dmg` |
| `{{BUILD_DATE}}` | today (UTC), `YYYY-MM-DD` |
| `{{BUILD_DATE_FR}}` | the same day in French, for people: `1er novembre 2026` (no-break spaces) |
| `{{GITHUB_URL}}` | `https://github.com/Makio64/pdf-edit` (only inside `IF_GITHUB` blocks) |
| `{{DOWNLOAD_URL}}` | `/telecharger/mac`: every download link uses it, with `?from=hero\|header\|download\|footer\|faq\|app` |

### Trying It Locally

```sh
python3 -m http.server --directory site/dist 8890     # then http://localhost:8890/en-ligne/
```

The app runs in browser mode (there is no `/api/app-info`); the beacon's `POST /api/e` gets a 501 from Python's server, which changes nothing for the page. Python's server ignores `_headers` (no `X-Frame-Options`, no `X-Robots-Tag`): check those on a draft deploy. To run the functions too: `cd site && STATS_KEY=<any test key> netlify dev --offline --dir dist --functions netlify/functions --port 8890` (they then count into the `stats-dev` store, never into production; `--functions` is needed, see « Deploying »). Unit tests of the counting logic: `cd site && npm test`.

---

## 3. Deploying

```sh
scripts/build-web.sh --ui-rev HEAD --dmg dist/PDF-Edit-<version>.dmg
cd site && netlify deploy --dir dist --functions netlify/functions          # a draft first: look at the URL it prints
cd site && netlify deploy --prod --dir dist --functions netlify/functions   # or: pnpm web:deploy
```

`pnpm web:deploy` runs that last command, and refuses when `site/dist` is missing or is a preview build (no `--dmg`: the release facts still hold the example's all-zero SHA-256).

- **Always pass `--functions netlify/functions`.** Run from `site/` inside this git repository, the Netlify CLI takes the repository root as the project folder unless the project's base directory (`site/`) comes from the Netlify settings, and then looks for the functions in `<repo>/netlify/functions`, which does not exist: the deploy goes out without `/api/e`, `/api/stats` and `/telecharger/mac` (the download links break), with only a warning. `--dir` and `--functions` are resolved from the current folder, so the command works whatever the project settings say. (`base = "site"` in `netlify.toml` is not a fix: `netlify dev` then looks for `site/site`.) Check the deploy log lists the three functions.

- The CLI uploads `dist/` and bundles `netlify/functions/` (esbuild), including `release.generated.mts`: **build right before deploying**, so the page and the download function describe the same release.
- A draft deploy is not a production deploy: its functions count into another store (`stats-<context>`), never into the production one.
- There is no build command on purpose. Do not connect the Git repository to the project for continuous deployment: Netlify would publish an empty `dist/`.
- Before `--prod`, on the draft: `/` shows the right version, size and screenshots; every menu link works on a computer and on a phone; `/en-ligne/` converts a PDF with each choice and opens the editor inside its frame; `/app/` goes to `/en-ligne/`; `/telecharger/mac?from=hero` downloads the image, and its SHA-256 matches `shasum -a 256 dist/PDF-Edit-<version>.dmg`; `curl -sI <draft>/app/ | grep -i -e x-frame -e x-robots` shows `SAMEORIGIN` and `noindex`.

---

## 4. Environment Variables

Set on Netlify (Project configuration › Environment variables, or `netlify env:set`), with the **Functions** scope. Never in `netlify.toml`: functions do not see variables defined there.

| Variable | Used by | Effect |
|---|---|---|
| `STATS_KEY` | `/api/stats`, so `/stats/` | The dashboard's password: a long random string (`openssl rand -base64 32`), marked as secret. Without it, or with fewer than 24 characters, `/api/stats` answers `503 { "error": "not configured" }`; with a wrong key, `401`. Netlify also limits `/api/stats` to 30 requests a minute per IP address, against guessing. |
| `GITHUB_REPO` | `/api/stats` | `Makio64/pdf-edit` once the repository is public: the dashboard adds the download counts of the GitHub releases (`.dmg` assets, fetched without a token, cached one hour). Unset: no GitHub figures. |

Build-time only (on the Mac that builds): `GITHUB_PUBLIC=1` (same as `--github-public`), `CARGO_TARGET_DIR` (the engine is built in `<it>/build-web`).

---

## 5. Analytics: Design and Privacy

**Goal:** know whether people come, from where, whether they download the Mac app, and whether the in-browser app works for them; without cookies, without identifiers and without anything about their documents, so no consent banner is needed.

### What Is Sent

The beacon `/assets/track.js` (an ES module of a few kilobytes, no dependency) sends small JSON events to `/api/e` with `navigator.sendBeacon`. Each event is a name and a few fixed-vocabulary values:

| Event | When | Values |
|---|---|---|
| `pv` | A page is shown (not the app inside the frame of `/en-ligne/`: that page view is the one of `/en-ligne/`) | page (`/`, `/en-ligne/`, `/questions/`, `/mentions-legales/`, `/app/`, `/app/edit.html`, `/404`, or `other`), whether the visitor arrived from outside the site, and then from where (a named group such as `google` or `gmail`, or the bare referring host), the `utm_source`, `mobile`/`desktop`, operating system family |
| `conv` | The in-browser app made a Word file | `pages`/`docx` (the choice of screen 2), worked or not, duration bucket (`lt5`, `5-15`, `15-60`, `gt60` seconds) |
| `pw` | A password was asked for | while converting or opening in the editor |
| `eopen` | A PDF opened in the in-browser editor | worked or not |
| `esave` | A corrected PDF was saved in the browser | worked or not |
| `gh` | A link to GitHub was clicked (`data-track="gh"`) | where on the page |
| `dl` | A Mac download | **counted by the server** in `/telecharger/mac` (version, link position, OS family from the request's user agent); `/api/e` refuses it |

The beacon names a page from its path (`/en-ligne`, `/en-ligne/index.html` → `/en-ligne/`); the server accepts only the names of this list (`PAGES` in `site/netlify/lib/validate.mts`). A new page needs both, and a label in `site/src/assets/stats.js`. Inside a frame (`window.top !== window.self`) `track.js` sends no page view, but `track()` still sends the app's events: one visit of `/en-ligne/` is one page view, whatever the app does in its frame. `/app/` and `/app/edit.html` page views now only come from an editor opened on its own (an unframed `/app/` leaves for `/en-ligne/` without counting, section 1); before the online version moved to `/en-ligne/` (November 2026) they were every visit of the app.

The in-browser app's events come from `site/app-overlay/wasm-client.js`: it re-exports the original `wasm-client.js` (renamed `wasm-client.core.js`) and wraps `convertPdfWasm`, `openEditSessionWasm` and `saveEditSessionWasm`, returning exactly what they return and rethrowing what they throw. It loads the beacon with a dynamic `import()` whose failure is ignored, so a blocked or missing beacon never breaks the app.

**Never sent or kept:** cookies, `localStorage`/`sessionStorage` identifiers, IP addresses (a referrer that is an IP address, such as a home server, is sent as `other`, and `/api/e` refuses an `r` without a letter), user-agent strings, fingerprints, file names, sizes or contents, page text, the full referring URL.

**Not counted at all:**

- Browsers that send Global Privacy Control or Do Not Track: `track.js` sends nothing (`navigator.globalPrivacyControl`, `navigator.doNotTrack === '1'`), and the server ignores requests with `Sec-GPC: 1` or `DNT: 1`, so `/telecharger/mac` still redirects but does not count the download.
- Visitors who clicked « Ne plus être compté sur ce navigateur » on `/mentions-legales/` (`site/src/assets/optout.js`). Safari sends neither signal, so this is its only way to object. The button stores one flag, `localStorage['pdfedit.optout'] = '1'` (a setting, not an identifier); `track.js` then sends nothing and adds `nc=1` to the download links, which `/telecharger/mac` does not count.
- Automated browsers (`navigator.webdriver`) and known bots.

### How It Is Stored

- `/api/e` accepts at most 1 KB, from the site's own pages only (the `Origin` or `Referer` host must be the site's, or `localhost`), drops known bots by user agent, validates every value against the allowlist and drops anything else; it always answers `204` with an empty body, so nothing can be learned by probing it. Netlify limits it to 60 requests a minute per IP address; Netlify uses the address for that limit only, and the site never stores it.
- Netlify Blobs, store `stats` in production (`stats-<context>` for drafts, previews and `netlify dev`), kept in Europe: region `eu-central-1` (Frankfurt), named on every `getStore` call (`STATS_REGION` in `netlify/lib/store.mts`), and the three functions run in Frankfurt too (`config.region: 'fra'`). Counts made before 2026-11-02 stayed in the old default region (`us-east-2`) and no longer show. Each event is a new, empty-valued blob whose **key** holds everything: `e/<UTC day>/<event>/<sorted values>/<time>-<random>`. Nothing is ever read and rewritten, so concurrent visits cannot lose counts; counting means listing the keys of a day.
- A finished day (UTC) is summed once into `r/<day>` the first time the dashboard asks for it; today is always counted live. The dashboard only ever shows those totals.
- Values chosen by the browser are bounded so nobody can blow up the counts with made-up referrers or campaigns: per day, the 200 most frequent `r` and `u` values are kept and the others are counted as `other`; each breakdown of `/api/stats` has at most 50 rows (the rest is added to `other`); and at most 50,000 raw events are counted per day (a day above that is rolled up with `"truncated": true`), so a flooded day cannot make `/api/stats` time out. `/telecharger/mac` itself has no rate limit on purpose: a limit would answer `429` instead of the disk image to people who share one address (a club, a library); adding one is the spec owner's call.
- **Kept for now, to decide:** the raw event blobs stay in the store after their day is rolled up, and their key ends with the recording time in milliseconds (`<Date.now() base36>`) as the spec asks. There is no identifier, but on a quiet day those times could let someone with access to the store follow one visitor's few events. Deleting `e/<day>/` some days after its rollup, and dropping the time from the key, would remove that; both change the spec and are not done. Only the site's owner can read the store.

### Why No Consent Banner

The design aims to stay within what the CNIL accepts for audience measurement without consent: strictly first-party; the beacon only reads what any page can see (the referrer, the screen width, `navigator.userAgent`, `maxTouchPoints`, the privacy signals) and stores nothing on the device for counting (the opt-out flag is the visitor's own choice); anonymous and aggregated daily counts, no cross-site tracking, no data shared with anyone, and a way to object: honoured automatically (GPC, Do Not Track) or with one click on `/mentions-legales/`. Visitors are told what is counted on `/mentions-legales/`. Any new event or value must keep to these rules: never anything about a document, never anything that could single out a person.

---

## 6. Reading the Stats

Open **`/stats/`**, type `STATS_KEY` once (it is kept in that browser's `localStorage`; « Se déconnecter » forgets it), choose the range (7 j, 30 j, 90 j, 1 an, Tout).

| Figure | Meaning |
|---|---|
| **Visites** | Page views of people arriving from outside the site (`pv` with entry = 1). A visitor who comes back tomorrow counts again: there are no « unique visitors » by design. |
| **Pages vues** | Every page view. |
| **Téléchargements Mac** | `GET /telecharger/mac` from non-bot browsers, counted by the server: someone who clicks twice counts twice. |
| **PDF convertis** | Successful / all conversions in the browser. |
| **Corrections enregistrées** | Corrected PDFs saved in the browser. |
| **Clics GitHub** | Clicks on GitHub links (only once the repository is public). |
| **GitHub downloads** | All-time downloads of the `.dmg` assets of the GitHub releases (with `GITHUB_REPO`). Downloads through the site are counted here **and** in « Téléchargements Mac ». |

The « Pages » table names the pages as the menu does:

| Value | Shown as |
|---|---|
| `/` | Accueil |
| `/en-ligne/` | Version en ligne |
| `/questions/` | Questions |
| `/mentions-legales/` | Confidentialité et mentions légales |
| `/app/` | App en ligne (ancienne adresse /app/) — the visits of the app before `/en-ligne/` |
| `/app/edit.html` | Éditeur en ligne (hors du cadre) |
| `/404` | Page introuvable |
| `other` | Autre page |

The tables break the counts down by page, origin, `utm_source`, OS, device, format, duration, error, version and link position. All counts are lower bounds: visitors with GPC or Do Not Track, those who opted out, and some content blockers, send nothing. A table shows at most 50 rows; smaller values are grouped in « Autre ». Days are UTC.

The same data as JSON, for scripts:

```sh
curl -s -H "Authorization: Bearer $STATS_KEY" "https://pdf-edit-mac.netlify.app/api/stats?days=30"
# days = 7 | 30 | 90 | 365 | all
```

It returns `range`, `series` (one entry per day, zeros included), `totals`, `allTime`, `breakdowns` and `github` (`null` without `GITHUB_REPO`). The raw keys can be listed with the CLI, from `site/`: `netlify blobs:list stats --prefix e/2026-11-01/`.

To recognise a shared link in the `utm_source` table, add `?utm_source=<name>` (lowercase letters, digits, `.`, `_`, `-`), for example `https://pdf-edit-mac.netlify.app/?utm_source=newsletter`.

---

## 7. Security Headers

`site/src/_headers` (copied to `dist/_headers`) sets, on every static file: `X-Content-Type-Options: nosniff`, `Referrer-Policy: strict-origin-when-cross-origin`, `X-Frame-Options: SAMEORIGIN`, `Permissions-Policy: camera=(), microphone=(), geolocation=()`. `SAMEORIGIN` is what lets `/en-ligne/` frame `/app/` while no other site can frame any page: never set `DENY` or `frame-ancestors 'none'` on `/app/*`. Also `Content-Type: application/wasm` on the engine, `X-Robots-Tag` on `/app/` (`noindex`), `/stats/` and `/downloads/`, the disk image as an attachment (`application/x-apple-diskimage`), one hour of browser cache for `/assets/*`, `/img/*`, the icons, `og.png` and the manifest (their names do not change between releases, so not longer), and a strict `Content-Security-Policy` on `/stats/` (below). Netlify joins the values of every rule matching a path, so each header is set by one rule per path only. The functions set their own headers (`Cache-Control: no-store` on `/api/stats` and `/telecharger/mac`).

**`/stats/`** holds the dashboard key in `localStorage` and loads only `/assets/stats.js` and `/assets/site.css` (its styles are set through CSSOM), so it gets `default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`. `form-action 'none'` also keeps the key out of the address bar if the script fails and the form falls back to a plain submit. Check `/stats/` in Safari, Firefox and Chrome after changing it.

**No `Content-Security-Policy` on the other pages yet.** The in-browser app needs a policy written for it and tested in Safari, Firefox and Chrome: WebAssembly compilation (`'wasm-unsafe-eval'`, which older Safari versions do not know), `blob:` URLs for the rendered pages and the fonts (`FontFace`), font downloads from `cdn.jsdelivr.net`, the inline frame script and styles the build injects into the app pages, and the pages' inline JSON-LD. A wrong policy breaks the app silently for exactly the people it is for. The plan: ship it first as `Content-Security-Policy-Report-Only`, check every flow in the three browsers, then enforce it.

---

## 8. Moving the Disk Image to GitHub Releases

While the repository is private, the image is a static file of the site (`/downloads/<file>`) and every GitHub mention sits in an `<!--IF_GITHUB-->` block that the build removes. Once the repository is public:

1. Publish the release with the image under its exact name, tag `v<version>`:
   ```sh
   gh release create v1.2.1 dist/PDF-Edit-1.2.1.dmg --title "PDF Edit 1.2.1" --notes "…"
   ```
2. Check that it downloads without being logged in: `curl -sIL https://github.com/Makio64/pdf-edit/releases/download/v1.2.1/PDF-Edit-1.2.1.dmg | grep -i '^HTTP'` ends with `200`.
3. Build with the GitHub blocks and GitHub hosting (the default with `--github-public`):
   ```sh
   scripts/build-web.sh --ui-rev HEAD --github-public --dmg dist/PDF-Edit-1.2.1.dmg
   ```
   `release.url` becomes the GitHub URL and `dist/downloads/` is no longer built (`--dmg-hosting site` keeps the image on the site instead).
4. Set `GITHUB_REPO=Makio64/pdf-edit` on Netlify to see the GitHub download counts on the dashboard.
5. Deploy. Links on the site always go through `/telecharger/mac`, so the old `/downloads/` address disappearing breaks nothing on the site.

---

## 9. When the App Pages Change

The build rewrites the copies of `extension/ui`, and stops rather than publish a page it does not understand:

- the header has no `<span class="brand">…</span>`, or `#samplesBox` cannot be emptied, or an example-PDF button (`data-sample`) remains;
- a page has no `<meta charset>` or `<head>` to put the frame script after, or already has its own `robots`, `referrer` or `canonical` tag (the build adds them);
- the engine lacks a `wasm_*` function that `wasm-client.js` calls;
- an app page links to a file the build does not copy: add the new file to `UI_FILES` in `scripts/build-web.sh`.

New engine functions only need the three wrappers in `site/app-overlay/wasm-client.js` if they should be counted; every other export of `wasm-client.js` passes through unchanged.

After a change to the app's pages, remake the screenshots too (section 10) if the screens they show changed.

---

## 10. Screenshots of the App

The six images of `site/src/img/` (shown on the home page in a light Mac-window frame drawn in CSS) are real screenshots of the app, light theme, 1600 × 1047 WebP, plus one close-up for phones, of a fictional letter (`site/shots/demo-letter.html`: « Les Amis du Jardin », « Madame Marie Dupont »). Never show a real document.

| File | Shows |
|---|---|
| `app-1-choisir.webp` | Step 1, « Quel PDF voulez-vous modifier ? » |
| `app-2-que-faire.webp` | Step 2, « Que voulez-vous faire ? » and its three choices |
| `app-3-ou-enregistrer.webp` | Step 3, « Où enregistrer le nouveau fichier ? » |
| `app-4-pret.webp` | « C'est prêt ! », the file name and where it is |
| `app-corriger-dans-le-pdf.webp` | The editor: the date retyped in the letter, « 1 ligne modifiée » |
| `app-comparer.webp` | « Comparer »: before and after, side by side |
| `app-corriger-dans-le-pdf-crop.webp` | 1306 × 825, the letter's heading, address and retyped date, cut from the 2× capture: shown instead of the editor screenshot below 561 px wide (a `<picture>` `<source>`), where the whole window is too small to read |

To make them again after the app changes (details in [`site/shots/README.md`](../site/shots/README.md)):

```sh
cargo build --release -p p2d-cli && ./target/release/p2d-cli app 8893 --no-open     # from the repository root
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --no-pdf-header-footer \
  --print-to-pdf="site/shots/Lettre de l’association.pdf" "file://$PWD/site/shots/demo-letter.html"
cd site && npm install && npm run shots       # capture.js (puppeteer-core + the installed Chrome), then convert.py (Pillow)
```

`capture.js` answers every call that would write, open or show a file on the Mac itself, so nothing is written. `convert.py` resizes the raw 2× PNGs of `site/shots/raw/` to 1600 px wide, WebP quality 82, under the names above, and cuts the phone close-up from the raw capture (`CROPS`: the box and the page's `width`/`height` change together); it warns when an image is not 1600 × 1047, the size the pages give in `width`/`height` (the build refuses an `<img>` without them, but cannot know the right values). `puppeteer-core` is a devDependency: the functions never import it, so it is not in their bundles. Look at every image before committing them.
