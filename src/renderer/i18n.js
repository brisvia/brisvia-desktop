// Brisvia i18n module. Spanish (source) + English. Resolves dotted keys (wallet.balance_available),
// replaces variables ({count}), handles plurals (Intl.PluralRules), formats numbers/dates (Intl) and persists the
// preference. Fallback: chosen language -> Spanish -> [key] (visible in development). Dictionaries live in
// window.LOCALES (locales.js), inlined to avoid load/CSP issues inside Tauri.
window.I18N = (function () {
  let lang = 'es';
  let dict = {};
  const fallback = () => (window.LOCALES && window.LOCALES.es) || {};

  function resolve(obj, key) {
    return key.split('.').reduce((o, k) => (o && o[k] != null ? o[k] : null), obj);
  }

  // t('clave', {count: 3, amount: '5'}) -> texto traducido con variables y plural.
  function t(key, vars) {
    let val = resolve(dict, key);
    if (val == null) val = resolve(fallback(), key);
    if (val == null) return `[${key}]`;
    if (val && typeof val === 'object') {
      // plural: { one, other } (y opcionalmente many/few/two/zero)
      if (vars && vars.count != null) {
        const cat = new Intl.PluralRules(lang).select(vars.count);
        val = val[cat] != null ? val[cat] : (val.other != null ? val.other : val.one);
      } else {
        val = val.other != null ? val.other : String(val);
      }
    }
    if (vars && typeof val === 'string') {
      val = val.replace(/\{(\w+)\}/g, (_, k) => (vars[k] != null ? vars[k] : `{${k}}`));
    }
    return val;
  }

  // Aplica traducciones a todo el DOM: [data-i18n]=textContent, [data-i18n-attr]="attr:clave;attr2:clave".
  function applyDom(root) {
    const r = root || document;
    r.querySelectorAll('[data-i18n]').forEach((el) => { el.textContent = t(el.getAttribute('data-i18n')); });
    r.querySelectorAll('[data-i18n-attr]').forEach((el) => {
      el.getAttribute('data-i18n-attr').split(';').forEach((pair) => {
        const idx = pair.indexOf(':');
        if (idx > 0) el.setAttribute(pair.slice(0, idx).trim(), t(pair.slice(idx + 1).trim()));
      });
    });
    document.documentElement.lang = lang;
  }

  function setLang(l, persist) {
    lang = (window.LOCALES && window.LOCALES[l]) ? l : 'es';
    dict = window.LOCALES[lang] || {};
    if (persist !== false) { try { localStorage.setItem('brv_lang', lang); } catch {} }
    applyDom();
    document.dispatchEvent(new CustomEvent('langchange', { detail: { lang } }));
  }

  // First run: OS in Spanish -> es; anything else -> en. Afterwards it respects the saved manual choice.
  function detect() {
    let saved = null;
    try { saved = localStorage.getItem('brv_lang'); } catch {}
    if (saved && window.LOCALES && window.LOCALES[saved]) return saved;
    const sys = (navigator.language || navigator.userLanguage || 'en').toLowerCase();
    return sys.startsWith('es') ? 'es' : 'en';
  }

  function fmtNum(n, opts) { return new Intl.NumberFormat(lang, opts).format(Number(n) || 0); }
  function fmtDate(epoch) {
    if (!epoch) return '';
    // Includes seconds: show hour, minute and second for each movement.
    return new Intl.DateTimeFormat(lang, {
      day: '2-digit', month: '2-digit', year: 'numeric',
      hour: '2-digit', minute: '2-digit', second: '2-digit',
    }).format(new Date(epoch * 1000));
  }

  return { t, applyDom, setLang, detect, fmtNum, fmtDate, get lang() { return lang; } };
})();
