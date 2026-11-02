// PDF Edit: private statistics dashboard (/stats/).
// Every value that comes from the API is put in the page with textContent / setAttribute only:
// referrer hosts and campaign names are typed by strangers.

const KEY_STORE = 'pdfedit.stats.key'
const PREFS_STORE = 'pdfedit.stats.prefs'
const SVG_NS = 'http://www.w3.org/2000/svg'
const RANGES = ['7', '30', '90', '365', 'all']
const METRICS = {
  visits: { label: 'Visites', one: 'visite', many: 'visites' },
  dl: { label: 'Téléchargements Mac', one: 'téléchargement', many: 'téléchargements' },
  conv: { label: 'Conversions', one: 'conversion', many: 'conversions' },
  esave: { label: 'Corrections', one: 'correction enregistrée', many: 'corrections enregistrées' },
}

const UNKNOWN = 'Non précisé'
const LABELS = {
  pages: {
    '/': 'Accueil',
    '/en-ligne/': 'Version en ligne',
    '/questions/': 'Questions',
    '/mentions-legales/': 'Confidentialité et mentions légales',
    // Before /en-ligne/ (November 2026) every visit of the app; since then the framed app sends no
    // page view and an unframed /app/ goes to /en-ligne/, so only an editor opened on its own adds here.
    '/app/': 'App en ligne (ancienne adresse /app/)',
    '/app/edit.html': 'Éditeur en ligne (hors du cadre)',
    '/404': 'Page introuvable',
    other: 'Autre page',
  },
  referrers: {
    direct: 'Accès direct, favori ou lien privé',
    google: 'Google',
    bing: 'Bing',
    duckduckgo: 'DuckDuckGo',
    qwant: 'Qwant',
    ecosia: 'Ecosia',
    yahoo: 'Yahoo',
    brave: 'Brave Search',
    github: 'GitHub',
    x: 'X (Twitter)',
    facebook: 'Facebook',
    instagram: 'Instagram',
    linkedin: 'LinkedIn',
    reddit: 'Reddit',
    hackernews: 'Hacker News',
    youtube: 'YouTube',
    gmail: 'Gmail',
    outlook: 'Outlook',
    other: 'Autre',
  },
  os: {
    mac: 'macOS',
    ios: 'iPhone ou iPad',
    windows: 'Windows',
    android: 'Android',
    linux: 'Linux',
    chromeos: 'ChromeOS',
    other: 'Autre',
  },
  device: { desktop: 'Ordinateur', mobile: 'Téléphone' },
  conv_format: { pages: 'Modifier dans Pages', docx: 'Modifier dans Word' },
  conv_time: { lt5: 'Moins de 5 secondes', '5-15': '5 à 15 secondes', '15-60': '15 à 60 secondes', gt60: 'Plus d’une minute' },
  conv_errors: { other: 'Erreur pendant la conversion' },
  dl_src: {
    hero: 'Haut de la page d’accueil',
    header: 'En-tête',
    download: 'Section «\u00a0Télécharger\u00a0»',
    footer: 'Pied de page',
    faq: 'Questions',
    app: 'App en ligne',
    other: 'Lien direct ou autre',
  },
  gh_src: {
    header: 'En-tête',
    footer: 'Pied de page',
    opensource: 'Section open source',
    download: 'Section «\u00a0Télécharger\u00a0»',
    faq: 'Questions',
    other: 'Autre',
  },
}
LABELS.dl_os = LABELS.os

const GROUPS = [
  {
    title: 'Audience',
    tables: [
      { name: 'pages', title: 'Pages vues', col: 'Page' },
      { name: 'referrers', title: 'D’où viennent les visites', col: 'Source', raw: true },
      { name: 'utm', title: 'Campagnes (utm_source)', col: 'Campagne', raw: true },
      { name: 'os', title: 'Systèmes des visiteurs', col: 'Système' },
      { name: 'device', title: 'Appareils', col: 'Appareil' },
    ],
  },
  {
    title: 'Téléchargements Mac',
    tables: [
      { name: 'dl_src', title: 'Bouton utilisé', col: 'Endroit' },
      { name: 'dl_version', title: 'Versions', col: 'Version', raw: true },
      { name: 'dl_os', title: 'Système au moment du téléchargement', col: 'Système' },
    ],
  },
  {
    title: 'App en ligne',
    tables: [
      { name: 'conv_format', title: 'Conversions par choix', col: 'Choix' },
      { name: 'conv_time', title: 'Durée des conversions réussies', col: 'Durée', order: ['lt5', '5-15', '15-60', 'gt60'] },
      { name: 'conv_errors', title: 'Conversions qui ont échoué', col: 'Cause' },
    ],
  },
  {
    title: 'GitHub',
    tables: [{ name: 'gh_src', title: 'Clics vers GitHub', col: 'Endroit' }],
  },
]

const nf = new Intl.NumberFormat('fr-FR')
const pct = new Intl.NumberFormat('fr-FR', { style: 'percent', maximumFractionDigits: 1 })
const dayShort = new Intl.DateTimeFormat('fr-FR', { day: 'numeric', month: 'short', timeZone: 'UTC' })
const dayLong = new Intl.DateTimeFormat('fr-FR', { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric', timeZone: 'UTC' })
const dayFull = new Intl.DateTimeFormat('fr-FR', { day: 'numeric', month: 'short', year: 'numeric', timeZone: 'UTC' })
const monthShort = new Intl.DateTimeFormat('fr-FR', { month: 'short', timeZone: 'UTC' })
const monthYear = new Intl.DateTimeFormat('fr-FR', { month: 'short', year: 'numeric', timeZone: 'UTC' })
const timeFmt = new Intl.DateTimeFormat('fr-FR', { hour: '2-digit', minute: '2-digit' })

const $ = (id) => document.getElementById(id)
const els = {
  login: $('login'),
  loginForm: $('login-form'),
  key: $('key'),
  loginError: $('login-error'),
  loginSubmit: $('login-submit'),
  logout: $('logout'),
  dashboard: $('dashboard'),
  rangeLabel: $('range-label'),
  ranges: $('ranges'),
  status: $('status'),
  statusText: $('status-text'),
  retry: $('retry'),
  content: $('content'),
  kpis: $('kpis'),
  metrics: $('metrics'),
  chartSub: $('chart-sub'),
  chart: $('chart'),
  dayTable: $('day-table'),
  tables: $('tables'),
}

const state = { key: null, days: '30', metric: 'visits', data: null, request: 0, controller: null }

// ---- storage (per-viewer convenience only; every access may throw) ----
function storageGet(name) {
  try {
    return localStorage.getItem(name)
  } catch {
    return null
  }
}
function storageSet(name, value) {
  try {
    if (value === null) localStorage.removeItem(name)
    else localStorage.setItem(name, value)
  } catch {}
}
function loadPrefs() {
  try {
    const prefs = JSON.parse(storageGet(PREFS_STORE) || '{}')
    if (RANGES.includes(prefs.days)) state.days = prefs.days
    if (Object.hasOwn(METRICS, prefs.metric)) state.metric = prefs.metric
  } catch {}
}
function savePrefs() {
  storageSet(PREFS_STORE, JSON.stringify({ days: state.days, metric: state.metric }))
}

// ---- DOM helpers (text only, never HTML strings) ----
function h(tag, attrs = {}, ...children) {
  const node = document.createElement(tag)
  for (const [name, value] of Object.entries(attrs)) {
    if (value === undefined || value === null || value === false) continue
    if (name === 'class') node.className = value
    else node.setAttribute(name, value === true ? '' : String(value))
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue
    node.append(child instanceof Node ? child : document.createTextNode(String(child)))
  }
  return node
}
function svg(tag, attrs = {}) {
  const node = document.createElementNS(SVG_NS, tag)
  for (const [name, value] of Object.entries(attrs)) node.setAttribute(name, String(value))
  return node
}
const num = (value) => (Number.isFinite(value) ? value : 0)
const fmt = (value) => nf.format(num(value))
const plural = (n, one, many) => `${fmt(n)} ${Math.abs(num(n)) >= 2 ? many : one}`
const dateOf = (day) => new Date(`${String(day)}T00:00:00Z`)
/** Formats a YYYY-MM-DD day; anything unexpected is shown as is (Intl throws on invalid dates). */
const fday = (formatter, day) => {
  const date = dateOf(day)
  return Number.isNaN(date.getTime()) ? String(day) : formatter.format(date)
}

// ---- views ----
function showLogin(message = '') {
  state.data = null
  els.dashboard.hidden = true
  els.logout.hidden = true
  els.login.hidden = false
  els.loginError.hidden = !message
  els.loginError.textContent = message
  els.loginSubmit.disabled = false
  els.key.value = ''
  els.key.focus()
}

function showDashboard() {
  els.login.hidden = true
  els.dashboard.hidden = false
  els.logout.hidden = false
  els.content.hidden = !state.data
  syncPressed()
}

function syncPressed() {
  for (const b of els.ranges.querySelectorAll('button')) b.setAttribute('aria-pressed', String(b.dataset.days === state.days))
  for (const b of els.metrics.querySelectorAll('button')) b.setAttribute('aria-pressed', String(b.dataset.metric === state.metric))
}

function setStatus(text, { retry = false, error = false } = {}) {
  els.status.hidden = !text
  els.status.classList.toggle('is-error', error)
  els.statusText.textContent = text
  els.retry.hidden = !retry
}

// ---- data ----
async function fetchStats(key, days, signal) {
  const res = await fetch(`/api/stats?days=${encodeURIComponent(days)}`, {
    headers: { authorization: `Bearer ${key}`, accept: 'application/json' },
    cache: 'no-store',
    credentials: 'omit',
    signal,
  })
  let body = null
  try {
    body = await res.json()
  } catch {}
  return { status: res.status, ok: res.ok, body }
}

function validPayload(body) {
  return (
    body &&
    typeof body === 'object' &&
    Array.isArray(body.series) &&
    body.totals &&
    typeof body.totals === 'object' &&
    body.range &&
    typeof body.range === 'object'
  )
}

async function load() {
  if (!state.key) return showLogin()
  const id = ++state.request
  state.controller?.abort()
  state.controller = new AbortController()
  els.content.classList.add('is-loading')
  els.dashboard.setAttribute('aria-busy', 'true')
  if (!state.data) setStatus('Chargement des statistiques…')
  try {
    const res = await fetchStats(state.key, state.days, state.controller.signal)
    if (id !== state.request) return
    if (res.status === 401) {
      storageSet(KEY_STORE, null)
      state.key = null
      return showLogin('Cette clé n’est pas (ou plus) la bonne. Entrez-la de nouveau.')
    }
    if (res.status === 503) {
      return setStatus('Le tableau de bord n’est pas encore configuré : ajoutez dans Netlify la variable STATS_KEY (au moins 24 caractères), puis redéployez le site.', { error: true, retry: true })
    }
    if (res.status === 429) {
      return setStatus('Trop de demandes en peu de temps : patientez une minute, puis réessayez.', { error: true, retry: true })
    }
    if (!res.ok || !validPayload(res.body)) {
      return setStatus('Impossible de charger les statistiques pour le moment.', { error: true, retry: true })
    }
    state.data = res.body
    setStatus('')
    render()
  } catch (err) {
    if (err?.name === 'AbortError' || id !== state.request) return
    setStatus('Impossible de joindre le serveur. Vérifiez la connexion, puis réessayez.', { error: true, retry: true })
  } finally {
    if (id === state.request) {
      els.content.classList.remove('is-loading')
      els.dashboard.removeAttribute('aria-busy')
    }
  }
}

async function login(event) {
  event.preventDefault()
  const key = els.key.value.trim()
  if (!key) {
    els.loginError.hidden = false
    els.loginError.textContent = 'Entrez la clé d’accès.'
    els.key.focus()
    return
  }
  els.loginSubmit.disabled = true
  els.loginError.hidden = true
  try {
    const res = await fetchStats(key, state.days)
    if (res.status === 401) return showLogin('Cette clé n’est pas la bonne.')
    if (res.status === 503) return showLogin('Le tableau de bord n’est pas encore configuré : la variable STATS_KEY manque dans Netlify, ou elle a moins de 24 caractères.')
    if (res.status === 429) return showLogin('Trop d’essais en peu de temps : patientez une minute, puis réessayez.')
    if (!res.ok || !validPayload(res.body)) return showLogin('Impossible de charger les statistiques pour le moment. Réessayez dans un instant.')
    state.key = key
    storageSet(KEY_STORE, key)
    state.data = res.body
    els.key.value = ''
    showDashboard()
    setStatus('')
    render()
    $('dash-title').focus({ preventScroll: true })
  } catch {
    showLogin('Impossible de joindre le serveur. Vérifiez la connexion, puis réessayez.')
  } finally {
    els.loginSubmit.disabled = false
  }
}

function logout() {
  state.controller?.abort()
  state.request++
  state.key = null
  storageSet(KEY_STORE, null)
  els.kpis.replaceChildren()
  els.chart.replaceChildren()
  els.dayTable.replaceChildren()
  els.tables.replaceChildren()
  setStatus('')
  showLogin()
}

// ---- rendering ----
function render() {
  const data = state.data
  if (!data) return
  els.content.hidden = false
  syncPressed()
  renderRange(data)
  renderKpis(data)
  renderChart(data)
  renderDayTable(data)
  renderTables(data)
}

function renderRange({ range, generatedAt }) {
  const from = String(range.from ?? '')
  const to = String(range.to ?? '')
  const days = num(range.days)
  let text = from && to ? `Du ${fday(dayFull, from)} au ${fday(dayFull, to)} (${plural(days, 'jour', 'jours')})` : ''
  const at = Date.parse(generatedAt)
  if (Number.isFinite(at)) text += ` · mis à jour à ${timeFmt.format(new Date(at))}`
  els.rangeLabel.textContent = text
}

function kpi(label, value, sub, total) {
  return h(
    'div',
    { class: 'stats-kpi' },
    h('p', { class: 'stats-kpi-label' }, label),
    h('p', { class: 'stats-kpi-value' }, value),
    sub ? h('p', { class: 'stats-kpi-sub' }, sub) : null,
    total !== undefined ? h('p', { class: 'stats-kpi-all' }, `Depuis le début : ${total}`) : null,
  )
}

function renderKpis({ totals: t, allTime: a = {}, github }) {
  const tiles = [
    kpi('Visites', fmt(t.visits), 'arrivées sur le site', fmt(a.visits)),
    kpi('Pages vues', fmt(t.pv), null, fmt(a.pv)),
    kpi('Téléchargements Mac', fmt(t.dl), 'clics sur «\u00a0Télécharger\u00a0»', fmt(a.dl)),
    kpi(
      'PDF convertis',
      `${fmt(t.conv_ok)} / ${fmt(t.conv)}`,
      `réussis / total${num(t.pw) ? ` · ${plural(t.pw, 'mot de passe demandé', 'mots de passe demandés')}` : ''}`,
      `${fmt(a.conv_ok)} / ${fmt(a.conv)}`,
    ),
    kpi('Corrections enregistrées', fmt(t.esave_ok), `${plural(t.eopen, 'PDF ouvert', 'PDF ouverts')} dans l’éditeur`, fmt(a.esave_ok)),
    kpi('Clics GitHub', fmt(t.gh), null, fmt(a.gh)),
  ]
  if (github && typeof github === 'object') {
    const at = Date.parse(github.fetchedAt)
    tiles.push(
      kpi(
        'Téléchargements GitHub',
        fmt(github.total),
        `toutes versions${Number.isFinite(at) ? `, relevé à ${timeFmt.format(new Date(at))}` : ''}`,
      ),
    )
  }
  els.kpis.replaceChildren(...tiles)
}

function niceStep(max, ticks = 4) {
  if (max <= ticks) return 1
  const raw = max / ticks
  const pow = 10 ** Math.floor(Math.log10(raw))
  for (const m of [1, 2, 2.5, 5, 10]) if (m * pow >= raw && Number.isInteger(m * pow)) return m * pow
  return 10 * pow
}

// At most `max` date labels (fewer on a phone, where « 12 oct. » labels would overlap).
function xLabels(series, max = 6) {
  const n = series.length
  const out = []
  if (n <= (max < 6 ? max : 10)) {
    series.forEach((d, i) => out.push([i, fday(dayShort, d.date)]))
    return out
  }
  if (n <= 120) {
    const step = Math.ceil(n / max)
    for (let i = n - 1; i >= 0; i -= step) out.unshift([i, fday(dayShort, series[i].date)])
    return out
  }
  const months = []
  series.forEach((d, i) => {
    if (i === 0 || String(d.date).endsWith('-01')) months.push(i)
  })
  const every = Math.ceil(months.length / (max < 6 ? max : 8))
  months.forEach((i, k) => {
    if (k % every === 0) {
      const date = dateOf(series[i].date)
      if (!Number.isNaN(date.getTime())) out.push([i, date.getUTCMonth() === 0 || k === 0 ? monthYear.format(date) : monthShort.format(date)])
    }
  })
  return out
}

function barPath(x, y, w, hgt) {
  const r = Math.min(4, w / 2, hgt)
  if (r < 1) return `M${x},${y + hgt}V${y}H${x + w}V${y + hgt}Z`
  return `M${x},${y + hgt}V${y + r}Q${x},${y} ${x + r},${y}H${x + w - r}Q${x + w},${y} ${x + w},${y + r}V${y + hgt}Z`
}

let tooltip = null
let activeIndex = -1

function renderChart({ series }) {
  const metric = METRICS[state.metric]
  const values = series.map((d) => num(d[state.metric]))
  const total = values.reduce((s, v) => s + v, 0)
  const max = Math.max(0, ...values)
  els.chartSub.textContent = `${metric.label} par jour · ${plural(total, metric.one, metric.many)} sur la période`

  const width = Math.max(280, Math.floor(els.chart.clientWidth || 640))
  const height = width < 520 ? 220 : 260
  const m = { top: 14, right: 6, bottom: 30, left: 40 }
  const plotW = width - m.left - m.right
  const plotH = height - m.top - m.bottom
  const step = niceStep(max)
  const yMax = Math.max(step, Math.ceil(max / step) * step)
  const y = (v) => m.top + plotH - (v / yMax) * plotH
  const band = plotW / Math.max(1, series.length)
  const gap = band >= 6 ? 2 : band >= 3 ? 1 : 0
  const barW = Math.max(0.5, Math.min(24, band - gap))

  const root = svg('svg', {
    viewBox: `0 0 ${width} ${height}`,
    width,
    height,
    class: 'stats-svg',
    role: 'img',
    tabindex: 0,
    'aria-label': `${metric.label} par jour, ${plural(total, metric.one, metric.many)} au total. Flèches gauche et droite pour lire chaque jour ; le tableau jour par jour est juste en dessous.`,
  })

  const grid = svg('g', { class: 'stats-grid', 'aria-hidden': 'true' })
  for (let v = 0; v <= yMax; v += step) {
    const yy = Math.round(y(v)) + 0.5
    grid.append(svg('line', { x1: m.left, x2: width - m.right, y1: yy, y2: yy }))
    const label = svg('text', { x: m.left - 8, y: yy + 4, 'text-anchor': 'end' })
    label.textContent = v >= 10000 ? `${nf.format(Math.round(v / 1000))} k` : nf.format(v)
    grid.append(label)
  }
  root.append(grid)

  const axis = svg('g', { class: 'stats-axis', 'aria-hidden': 'true' })
  for (const [i, text] of xLabels(series, width < 520 ? 4 : 6)) {
    const cx = m.left + band * i + band / 2
    const anchor = cx < m.left + 30 ? 'start' : cx > width - m.right - 30 ? 'end' : 'middle'
    const label = svg('text', { x: Math.round(anchor === 'start' ? m.left + band * i : anchor === 'end' ? m.left + band * (i + 1) : cx), y: height - 8, 'text-anchor': anchor })
    label.textContent = text
    axis.append(label)
  }
  root.append(axis)

  const bars = svg('g', { class: 'stats-bars' })
  const hits = svg('g', { class: 'stats-hits' })
  series.forEach((d, i) => {
    const v = values[i]
    const x = m.left + band * i + (band - barW) / 2
    if (v > 0) {
      const top = y(v)
      bars.append(svg('path', { d: barPath(x, top, barW, m.top + plotH - top), class: 'stats-bar', 'data-i': i }))
    }
    const hit = svg('rect', { x: m.left + band * i, y: m.top, width: band, height: plotH, 'data-i': i })
    hits.append(hit)
  })
  root.append(bars, hits)

  tooltip = h('div', { class: 'stats-tip', hidden: true, 'aria-hidden': 'true' })
  const children = [root, tooltip]
  if (total === 0) children.push(h('p', { class: 'stats-empty-chart' }, 'Aucune donnée pour cette période.'))
  els.chart.replaceChildren(...children)
  activeIndex = -1

  const show = (i) => {
    if (i < 0 || i >= series.length) return
    activeIndex = i
    for (const bar of bars.querySelectorAll('.is-active')) bar.classList.remove('is-active')
    bars.querySelector(`[data-i="${i}"]`)?.classList.add('is-active')
    const v = values[i]
    tooltip.replaceChildren(
      h('strong', {}, plural(v, metric.one, metric.many)),
      h('span', {}, fday(dayLong, series[i].date)),
    )
    tooltip.hidden = false
    const cx = m.left + band * i + band / 2
    const tipW = tooltip.offsetWidth
    const left = Math.min(Math.max(cx - tipW / 2, 0), width - tipW)
    tooltip.style.left = `${left}px`
    tooltip.style.top = `${Math.max(0, y(v) - tooltip.offsetHeight - 10)}px`
  }
  const hide = () => {
    activeIndex = -1
    tooltip.hidden = true
    for (const bar of bars.querySelectorAll('.is-active')) bar.classList.remove('is-active')
  }
  const indexFromEvent = (event) => {
    const box = root.getBoundingClientRect()
    const x = ((event.clientX - box.left) / box.width) * width
    return Math.floor((x - m.left) / band)
  }
  root.addEventListener('pointermove', (event) => {
    const i = indexFromEvent(event)
    if (i >= 0 && i < series.length && i !== activeIndex) show(i)
  })
  root.addEventListener('pointerleave', () => {
    if (document.activeElement !== root) hide()
  })
  root.addEventListener('focus', () => show(activeIndex >= 0 ? activeIndex : series.length - 1))
  root.addEventListener('blur', hide)
  root.addEventListener('keydown', (event) => {
    const moves = { ArrowLeft: -1, ArrowRight: 1, Home: -Infinity, End: Infinity }
    if (!(event.key in moves)) return
    event.preventDefault()
    const delta = moves[event.key]
    const next = delta === -Infinity ? 0 : delta === Infinity ? series.length - 1 : Math.min(series.length - 1, Math.max(0, (activeIndex < 0 ? series.length - 1 : activeIndex) + delta))
    show(next)
  })
}

function renderDayTable({ series }) {
  const rows = [...series].reverse().map((d) =>
    h(
      'tr',
      {},
      h('th', { scope: 'row' }, fday(dayFull, d.date)),
      h('td', { class: 'num' }, fmt(d.visits)),
      h('td', { class: 'num' }, fmt(d.pv)),
      h('td', { class: 'num' }, fmt(d.dl)),
      h('td', { class: 'num' }, `${fmt(d.conv_ok)} / ${fmt(d.conv)}`),
      h('td', { class: 'num' }, fmt(d.eopen)),
      h('td', { class: 'num' }, fmt(d.esave)),
      h('td', { class: 'num' }, fmt(d.gh)),
    ),
  )
  els.dayTable.replaceChildren(
    h(
      'table',
      { class: 'table stats-table' },
      h('caption', { class: 'stats-sr' }, 'Chiffres jour par jour, du plus récent au plus ancien'),
      h(
        'thead',
        {},
        h(
          'tr',
          {},
          h('th', { scope: 'col' }, 'Jour'),
          ['Visites', 'Pages vues', 'Téléch. Mac', 'Conversions (réussies / total)', 'PDF ouverts', 'Corrections', 'Clics GitHub'].map((t) =>
            h('th', { scope: 'col', class: 'num' }, t),
          ),
        ),
      ),
      h('tbody', {}, rows),
    ),
  )
}

function labelFor(name, value) {
  if (value === 'unknown') return UNKNOWN
  const table = LABELS[name]
  return table && Object.hasOwn(table, value) ? table[value] : value
}

function meter(share) {
  const fill = h('span')
  fill.style.width = `${Math.max(2, Math.round(share * 100))}%`
  return h('span', { class: 'stats-meter', 'aria-hidden': 'true' }, fill)
}

function breakdownCard(spec, table) {
  const entries = Object.entries(table && typeof table === 'object' ? table : {})
    .map(([value, count]) => [String(value), num(count)])
    .filter(([, count]) => count > 0)
  if (spec.order) entries.sort((a, b) => spec.order.indexOf(a[0]) - spec.order.indexOf(b[0]))
  else entries.sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
  const sum = entries.reduce((s, [, c]) => s + c, 0)
  const card = h('section', { class: 'card stats-card' }, h('h3', {}, spec.title))
  if (!entries.length) {
    card.append(h('p', { class: 'muted stats-empty' }, 'Rien pour cette période.'))
    return card
  }
  const LIMIT = 8
  const rows = entries.map(([value, count], i) => {
    const share = sum ? count / sum : 0
    const label = labelFor(spec.name, value)
    const cell = h('th', { scope: 'row' }, h('span', { class: 'stats-label' }, label), meter(share))
    if (label !== value) cell.title = value
    return h(
      'tr',
      { hidden: i >= LIMIT },
      cell,
      h('td', { class: 'num' }, fmt(count)),
      h('td', { class: 'num' }, pct.format(share)),
    )
  })
  const tableEl = h(
    'table',
    { class: 'table stats-table' },
    h('thead', {}, h('tr', {}, h('th', { scope: 'col' }, spec.col), h('th', { scope: 'col', class: 'num' }, 'Nombre'), h('th', { scope: 'col', class: 'num' }, 'Part'))),
    h('tbody', {}, rows),
  )
  card.append(h('div', { class: 'stats-scroll' }, tableEl))
  if (entries.length > LIMIT) {
    const more = h('button', { type: 'button', class: 'stats-more', 'aria-expanded': 'false' }, `Afficher les ${fmt(entries.length)} lignes`)
    more.addEventListener('click', () => {
      const open = more.getAttribute('aria-expanded') !== 'true'
      rows.forEach((row, i) => (row.hidden = !open && i >= LIMIT))
      more.setAttribute('aria-expanded', String(open))
      more.textContent = open ? 'Afficher moins' : `Afficher les ${fmt(entries.length)} lignes`
    })
    card.append(more)
  }
  return card
}

function githubCard(github) {
  const card = h('section', { class: 'card stats-card' }, h('h3', {}, 'GitHub Releases (.dmg)'))
  const assets = Array.isArray(github.assets) ? github.assets : []
  if (!assets.length) {
    card.append(h('p', { class: 'muted stats-empty' }, 'Aucun fichier .dmg publié pour l’instant.'))
    return card
  }
  const rows = assets.map((a) => h('tr', {}, h('th', { scope: 'row' }, String(a?.name ?? '')), h('td', { class: 'num' }, fmt(a?.downloads))))
  card.append(
    h(
      'div',
      { class: 'stats-scroll' },
      h(
        'table',
        { class: 'table stats-table' },
        h('thead', {}, h('tr', {}, h('th', { scope: 'col' }, 'Fichier'), h('th', { scope: 'col', class: 'num' }, 'Téléchargements'))),
        h('tbody', {}, rows),
      ),
    ),
  )
  return card
}

function renderTables({ breakdowns = {}, github }) {
  const sections = GROUPS.map((group) => {
    const cards = group.tables.map((spec) => breakdownCard(spec, breakdowns[spec.name]))
    if (group.title === 'GitHub' && github && typeof github === 'object') cards.push(githubCard(github))
    return h('section', { class: 'stats-group' }, h('h2', { class: 'stats-h2' }, group.title), h('div', { class: 'stats-cards' }, cards))
  })
  els.tables.replaceChildren(...sections)
}

// ---- wiring ----
els.loginForm.addEventListener('submit', login)
els.logout.addEventListener('click', logout)
els.retry.addEventListener('click', () => load())
els.ranges.addEventListener('click', (event) => {
  const button = event.target.closest('button[data-days]')
  if (!button || button.dataset.days === state.days) return
  state.days = button.dataset.days
  savePrefs()
  syncPressed()
  load()
})
els.metrics.addEventListener('click', (event) => {
  const button = event.target.closest('button[data-metric]')
  if (!button || !Object.hasOwn(METRICS, button.dataset.metric)) return
  state.metric = button.dataset.metric
  savePrefs()
  syncPressed()
  if (state.data) renderChart(state.data)
})

let resizeTimer = 0
let lastWidth = 0
new ResizeObserver(() => {
  const w = els.chart.clientWidth
  if (!state.data || Math.abs(w - lastWidth) < 4) return
  lastWidth = w
  clearTimeout(resizeTimer)
  resizeTimer = setTimeout(() => renderChart(state.data), 120)
}).observe(els.chart)

loadPrefs()
state.key = storageGet(KEY_STORE)
if (state.key) {
  showDashboard()
  load()
} else {
  showLogin()
}
