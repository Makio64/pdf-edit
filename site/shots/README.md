# Screenshots of the app for the website

The website shows six screenshots of the real app (`site/src/img/*.webp`, 1600 × 1047, light theme). This folder makes them again after the app's pages change.

| File | What it is |
|---|---|
| `demo-letter.html` | A fictional one-page letter (« Les Amis du Jardin », « Madame Marie Dupont, 8 rue des Lilas »), printed to PDF and opened in the app. Never use a real document. |
| `capture.js` | Drives Google Chrome (`puppeteer-core`, no bundled browser) through the app served by `p2d-cli app`: the three steps, « C'est prêt ! », the editor (the date retyped as « 10 novembre ») and « Comparer ». It answers every call that would write, open or show a file on the Mac (`/api/convert`, `/api/action`, `/api/choose-folder`, `/api/edit/save`) itself: nothing is written, no app opens. Raw 2× PNGs go to `raw/`. |
| `convert.py` | Turns the six shots the site uses into `site/src/img/*.webp` (Pillow): 1600 px wide, WebP quality 82. It also cuts a close-up of the corrected letter for phones from the raw capture (`CROPS`). |

`raw/` and the printed PDF are ignored by git.

## Steps

From the repository root, on a Mac with Google Chrome in `/Applications`:

1. **Start the developer server** on port 8893. It serves the pages of `extension/ui` from the working tree, so run it from the repository root:

   ```sh
   cargo build --release -p p2d-cli
   ./target/release/p2d-cli app 8893 --no-open
   ```

2. **Print the demo letter to PDF** (once; the name is the one the screenshots show):

   ```sh
   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" --headless=new --no-pdf-header-footer \
     --print-to-pdf="site/shots/Lettre de l’association.pdf" "file://$PWD/site/shots/demo-letter.html"
   ```

3. **Capture and convert**, in another terminal:

   ```sh
   cd site
   npm install            # once: puppeteer-core (a devDependency, never in the functions)
   npm run shots          # capture.js, then convert.py
   ```

   `npm run shots:convert` only redoes the conversion from `raw/`. `convert.py` needs Pillow (`python3 -m pip install --user Pillow`).

4. **Look at every image** in `site/src/img/` before committing them, then stop the server (Ctrl-C).

| Raw shot | Published as |
|---|---|
| `raw/1-choose.png` | `app-1-choisir.webp` |
| `raw/2-action.png` | `app-2-que-faire.webp` |
| `raw/3-place.png` | `app-3-ou-enregistrer.webp` |
| `raw/4-done.png` | `app-4-pret.webp` |
| `raw/7-corrected.png` | `app-corriger-dans-le-pdf.webp`, and the phone close-up `app-corriger-dans-le-pdf-crop.webp` (1306 × 825, raw pixels x 360–1666, y 240–1065) |
| `raw/8-compare.png` | `app-comparer.webp` |

`5-editor` and `6-typing` are made too, but not published.

## Options

| Variable | Default | Meaning |
|---|---|---|
| `PORT` | `8893` | Port of `p2d-cli app` |
| `BASE_URL` | `http://127.0.0.1:$PORT` | Another server |
| `CHROME` | `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` | The browser |

`python3 shots/convert.py --raw <dir> --out <dir>` converts from and to other folders.

## When It Breaks

- **« date line not found »**: the editor's lines changed; check that `demo-letter.html` still says « Paris, le 3 novembre 2026 » and that the PDF was printed again.
- **A selector times out** (`#stepChoice`, `#doneView`, `.edit-line`, `#compareBtn`): the app's pages changed; update `capture.js` to match `extension/ui`.
- **The phone close-up cuts the letter or the date**: the editor or `demo-letter.html` moved; change the box in `CROPS` (`convert.py`), and the `width`/`height` of the `<source>` in `site/src/index.html` if its size changes.
- **convert.py warns about the shape**: the pages give every screenshot `width="1600" height="1047"`; a different window size in `capture.js` needs the pages changed too (`scripts/build-web.sh` refuses an image without `width`/`height`, but cannot know the right ones).
