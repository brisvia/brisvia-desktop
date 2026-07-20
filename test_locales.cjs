const fs = require('fs');
global.window = {};
eval(fs.readFileSync(__dirname + '/src/renderer/locales.js', 'utf8'));
const L = window.LOCALES;
const LANGS = ['es', 'en', 'pt', 'zh', 'ru'];
function keys(obj, p = '') {
  let k = [];
  for (const key in obj) {
    const full = p ? `${p}.${key}` : key;
    const v = obj[key];
    if (v && typeof v === 'object' && !('one' in v) && !('other' in v)) k = k.concat(keys(v, full));
    else k.push(full);
  }
  return k;
}
// Every UI variable {x} in the English source must survive in each translation (plural forms scanned too).
function phs(v) {
  const set = new Set();
  const scan = (s) => { if (typeof s === 'string') (s.match(/\{(\w+)\}/g) || []).forEach((x) => set.add(x)); };
  if (v && typeof v === 'object') Object.values(v).forEach(scan); else scan(v);
  return set;
}
function resolve(obj, key) { return key.split('.').reduce((o, k) => (o && o[k] != null ? o[k] : null), obj); }

const enk = keys(L.en).sort();
let fail = 0;
for (const lang of LANGS) {
  if (!L[lang]) { console.log('FALLO: falta el idioma', lang); fail++; continue; }
  const kk = keys(L[lang]).sort();
  const missing = enk.filter((k) => !kk.includes(k));
  const extra = kk.filter((k) => !enk.includes(k));
  let ph = [];
  for (const k of enk) {
    if (!kk.includes(k)) continue;
    const need = phs(resolve(L.en, k)), got = phs(resolve(L[lang], k));
    for (const p of need) if (!got.has(p)) ph.push(`${k}(${p})`);
  }
  if (missing.length || extra.length || ph.length) {
    fail++;
    console.log(`FALLO ${lang}:`);
    if (missing.length) console.log('  faltan:', missing.slice(0, 15).join(', '));
    if (extra.length) console.log('  sobran:', extra.slice(0, 15).join(', '));
    if (ph.length) console.log('  placeholders perdidos:', ph.slice(0, 15).join(', '));
  } else {
    console.log(`OK ${lang}: ${kk.length} claves, placeholders intactos`);
  }
}
if (fail) { console.log(`\nFALLO: ${fail} idioma(s) con problemas`); process.exit(1); }
console.log(`\nOK: los ${LANGS.length} idiomas coinciden exactamente (${enk.length} claves) con placeholders intactos`);
