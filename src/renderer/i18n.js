// Brisvia i18n module. Spanish (source) + English. Resolves dotted keys (wallet.balance_available),
// replaces variables ({count}), handles plurals (Intl.PluralRules), formats numbers/dates (Intl) and persists the
// preference. Fallback: chosen language -> Spanish -> [key] (visible in development). Dictionaries live in
// window.LOCALES (locales.js), inlined to avoid load/CSP issues inside Tauri.
window.I18N = (function () {
  let lang = 'es';
  let dict = {};
  // Universal fallback for missing keys: English (a full translation), then Spanish (source).
  const fallback = () => (window.LOCALES && (window.LOCALES.en || window.LOCALES.es)) || {};

  function resolve(obj, key) {
    return key.split('.').reduce((o, k) => (o && o[k] != null ? o[k] : null), obj);
  }

  // t('key', {count: 3, amount: '5'}) -> translated text with variables and plural.
  function t(key, vars) {
    let val = resolve(dict, key);
    if (val == null) val = resolve(fallback(), key);
    if (val == null) return `[${key}]`;
    if (val && typeof val === 'object') {
      // plural: { one, other } (and optionally many/few/two/zero)
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

  // Applies translations across the DOM: [data-i18n]=textContent, [data-i18n-attr]="attr:key;attr2:key".
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
    lang = (window.LOCALES && window.LOCALES[l]) ? l : 'en';
    dict = window.LOCALES[lang] || {};
    if (persist !== false) { try { localStorage.setItem('brv_lang', lang); } catch {} }
    applyDom();
    document.dispatchEvent(new CustomEvent('langchange', { detail: { lang } }));
  }

  // Maps an OS/browser language tag to a supported app language, or null if unsupported.
  // Policy mirrors the website: es/en/pt/ru by prefix; Simplified Chinese -> zh; Traditional
  // Chinese (zh-Hant / zh-TW / zh-HK / zh-MO) is NOT supported yet -> null (falls back to English).
  function mapTag(tag) {
    tag = String(tag || '').toLowerCase(); if (!tag) return null;
    if (tag === 'es' || tag.indexOf('es-') === 0) return 'es';
    if (tag === 'en' || tag.indexOf('en-') === 0) return 'en';
    if (tag === 'pt' || tag.indexOf('pt-') === 0) return 'pt';
    if (tag === 'ru' || tag.indexOf('ru-') === 0) return 'ru';
    if (tag === 'zh-tw' || tag === 'zh-hk' || tag === 'zh-mo' || tag.indexOf('hant') !== -1) return null;
    if (tag === 'zh' || tag.indexOf('zh-') === 0 || tag.indexOf('hans') !== -1) return 'zh';
    return null;
  }
  // First run: detect the OS language and open in the matching supported language; anything
  // unsupported -> English (universal fallback). Afterwards it respects the saved manual choice.
  function detect() {
    let saved = null;
    try { saved = localStorage.getItem('brv_lang'); } catch {}
    if (saved && window.LOCALES && window.LOCALES[saved]) return saved;
    const list = (navigator.languages && navigator.languages.length)
      ? navigator.languages : [navigator.language || navigator.userLanguage || ''];
    for (let i = 0; i < list.length; i++) {
      const m = mapTag(list[i]);
      if (m && window.LOCALES && window.LOCALES[m]) return m;
    }
    return 'en';
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
