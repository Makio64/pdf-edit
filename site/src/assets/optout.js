// « Ne plus être compté sur ce navigateur » (mentions légales, section « Si vous ne voulez pas
// être compté »). Stores one flag, `pdfedit.optout` = '1', never an identifier: track.js reads it
// and then sends nothing, and marks the download links so the server does not count them either.
const KEY = 'pdfedit.optout'
const box = document.querySelector('[data-optout-box]')
const button = box && box.querySelector('[data-optout]')
const status = box && box.querySelector('[data-optout-status]')
const nav = globalThis.navigator || {}
const signal = nav.globalPrivacyControl === true || nav.doNotTrack === '1'

function read() {
  try {
    return localStorage.getItem(KEY) === '1'
  } catch {
    return null
  }
}

function write(on) {
  try {
    if (on) localStorage.setItem(KEY, '1')
    else localStorage.removeItem(KEY)
    return read() === on
  } catch {
    return false
  }
}

// The links of this page, without reloading it (track.js does it on the next pages, and leaves
// them absolute: hence *= rather than ^=).
function markLinks(on) {
  for (const a of document.querySelectorAll('a[href*="/telecharger/"]')) {
    const url = new URL(a.href)
    if (url.origin !== location.origin) continue
    if (on) url.searchParams.set('nc', '1')
    else url.searchParams.delete('nc')
    a.href = url.pathname + url.search + url.hash
  }
}

function render(message) {
  const on = read()
  button.textContent = on ? 'Être compté de nouveau' : 'Ne plus être compté sur ce navigateur'
  status.textContent =
    message ??
    (on
      ? 'Ce navigateur n’est plus compté.'
      : signal
        ? 'Votre navigateur demande déjà à ne pas être suivi : rien n’est compté.'
        : '')
}

if (box && button && status) {
  if (read() === null) {
    // Storage blocked (some private windows): nothing can be remembered, say so.
    button.hidden = true
    status.textContent = 'Ce navigateur ne permet pas d’enregistrer ce choix. Le signal « Do Not Track » ou Global Privacy Control de ses réglages fonctionne toujours.'
  } else {
    button.addEventListener('click', () => {
      const on = !read()
      if (!write(on)) return render('Le choix n’a pas pu être enregistré dans ce navigateur.')
      markLinks(on)
      render(on ? 'C’est fait : ce navigateur n’est plus compté.' : 'C’est noté : ce navigateur est de nouveau compté, de façon anonyme.')
    })
    render()
  }
  box.hidden = false
}
