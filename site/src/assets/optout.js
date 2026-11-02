// « Ne plus être compté sur ce navigateur » (mentions légales, section « Si vous ne voulez pas
// être compté », and its translations). Stores one flag, `pdfedit.optout` = '1', never an identifier:
// track.js reads it and then sends nothing, and marks the download links so the server does not count
// them either. The words come from the page, in its language (site/i18n): the button's own text, and
// the box's data-again (the button once opted out) and data-off, data-signal, data-blocked,
// data-failed, data-done-off, data-done-on (the messages).
const KEY = 'pdfedit.optout'
const box = document.querySelector('[data-optout-box]')
const button = box && box.querySelector('[data-optout]')
const status = box && box.querySelector('[data-optout-status]')
const words = (box && box.dataset) || {}
const label = button ? button.textContent : ''
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
  button.textContent = (on && words.again) || label
  status.textContent = message ?? ((on ? words.off : signal ? words.signal : '') || '')
}

if (box && button && status) {
  if (read() === null) {
    // Storage blocked (some private windows): nothing can be remembered, say so.
    button.hidden = true
    status.textContent = words.blocked || ''
  } else {
    button.addEventListener('click', () => {
      const on = !read()
      if (!write(on)) return render(words.failed || '')
      markLinks(on)
      render((on ? words.doneOff : words.doneOn) || '')
    })
    render()
  }
  box.hidden = false
}
