const fs = require('fs');
global.window = {};
eval(fs.readFileSync(__dirname + '/src/renderer/locales.js', 'utf8'));
const { es, en } = window.LOCALES;
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
const esk = keys(es).sort(), enk = keys(en).sort();
const missEn = esk.filter(k => !enk.includes(k));
const missEs = enk.filter(k => !esk.includes(k));
if (missEn.length || missEs.length) {
  console.log('FALLO test de claves:');
  if (missEn.length) console.log('  faltan en EN:', missEn.join(', '));
  if (missEs.length) console.log('  faltan en ES:', missEs.join(', '));
  process.exit(1);
}
console.log(`OK: ${esk.length} claves, ES y EN coinciden exactamente`);
