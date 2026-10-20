// Runs before first paint so the page never flashes the wrong theme.
// Keep THEME_KEY in sync with src/lib/theme.ts.
;(function () {
  var THEME_KEY = 'linkscope-theme'
  var saved = null
  try {
    saved = localStorage.getItem(THEME_KEY)
  } catch (e) {}
  var dark = saved === 'dark' || (saved !== 'light' && window.matchMedia('(prefers-color-scheme: dark)').matches)
  document.documentElement.classList.toggle('dark', dark)
})()
