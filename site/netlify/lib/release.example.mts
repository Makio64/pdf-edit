// Fallback release info. scripts/build-web.sh writes netlify/lib/release.generated.mts with the
// same shape from the real DMG, and from the facts of this file when no DMG is given (a preview).
// `url` is the static /downloads/<file> while the repo is private, a GitHub Releases URL once public.
export const release = {
  version: '1.2.0',
  file: 'PDF-Edit-1.2.0.dmg',
  url: '/downloads/PDF-Edit-1.2.0.dmg',
  bytes: 7340032,
  sha256: '0000000000000000000000000000000000000000000000000000000000000000',
}
