// Money and the decimal separator. Run with: node --test tests/amount-separator.test.js
//
// Why this file exists: parse_amount_briv used to do replace(',', '.') so an ES user could type "12,5".
// That silently turned "1,000" into 1 BRVA. In English "1,000" is one thousand — someone sending 1,000
// would have sent 1 and lost 999. The backend cannot know which convention the typist meant, so it refuses
// commas outright; the frontend, which knows the active language, normalises to the canonical form and
// REFUSES the separator that does not belong to that language instead of reinterpreting it.
//
// The rule under test:
//   ES  -> comma is the decimal separator, a dot is an error
//   EN  -> dot is the decimal separator, a comma is an error
//   no thousands separators in either language
//   mixtures ("1,000.50" / "1.000,50") always refused
//   the backend always receives a canonical dot

const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

// Pull the real function out of app.js instead of copying it: a copy would drift and keep passing while
// the shipped code broke.
const src = fs.readFileSync(path.join(__dirname, '..', 'src', 'renderer', 'app.js'), 'utf8');
const fn = src.match(/function toCanonicalAmount\(raw, lang\) \{[\s\S]*?\n\}/);
assert.ok(fn, 'toCanonicalAmount not found in app.js — did it get renamed?');
// eslint-disable-next-line no-new-func
const toCanonicalAmount = new Function(`${fn[0]}; return toCanonicalAmount;`)();

test('THE BUG: "1,000" in English must never become 1 BRVA', () => {
  // One thousand, written the English way. Ambiguous as an amount -> refuse, never reinterpret.
  assert.strictEqual(toCanonicalAmount('1,000', 'en'), null);
});

test('"1.000" in Spanish is refused too (it reads as a thousands separator)', () => {
  assert.strictEqual(toCanonicalAmount('1.000', 'es'), null);
});

test('the four cases asked for, in both languages', () => {
  // EN: dot decimal, comma refused
  assert.strictEqual(toCanonicalAmount('1.00000000', 'en'), '1.00000000');
  assert.strictEqual(toCanonicalAmount('1,00000000', 'en'), null);
  assert.strictEqual(toCanonicalAmount('1.000', 'en'), '1.000');   // one, three decimals — unambiguous in EN
  assert.strictEqual(toCanonicalAmount('1,000', 'en'), null);      // one thousand — refused

  // ES: comma decimal, dot refused
  assert.strictEqual(toCanonicalAmount('1,00000000', 'es'), '1.00000000');
  assert.strictEqual(toCanonicalAmount('1.00000000', 'es'), null);
  assert.strictEqual(toCanonicalAmount('1,000', 'es'), '1.000');   // one, three decimals — unambiguous in ES
  assert.strictEqual(toCanonicalAmount('1.000', 'es'), null);      // one thousand — refused
});

test('mixtures are always refused', () => {
  for (const lang of ['es', 'en']) {
    assert.strictEqual(toCanonicalAmount('1,000.50', lang), null, `1,000.50 accepted in ${lang}`);
    assert.strictEqual(toCanonicalAmount('1.000,50', lang), null, `1.000,50 accepted in ${lang}`);
    assert.strictEqual(toCanonicalAmount('1,2,3', lang), null);
    assert.strictEqual(toCanonicalAmount('1.2.3', lang), null);
  }
});

test('the backend always receives a dot', () => {
  assert.strictEqual(toCanonicalAmount('12,5', 'es'), '12.5');
  assert.strictEqual(toCanonicalAmount('12.5', 'en'), '12.5');
  assert.strictEqual(toCanonicalAmount('0,00000001', 'es'), '0.00000001');
  assert.strictEqual(toCanonicalAmount('0.00000001', 'en'), '0.00000001');
});

test('the same typing means the same money in both languages', () => {
  assert.strictEqual(toCanonicalAmount('12,5', 'es'), toCanonicalAmount('12.5', 'en'));
  assert.strictEqual(toCanonicalAmount('0,1', 'es'), toCanonicalAmount('0.1', 'en'));
});

test('junk and empties are refused', () => {
  for (const lang of ['es', 'en']) {
    for (const bad of ['', '   ', 'abc', '$5', '1.5x', ',', '.', '1-2', '1e8']) {
      assert.strictEqual(toCanonicalAmount(bad, lang), null, `${JSON.stringify(bad)} accepted in ${lang}`);
    }
    assert.strictEqual(toCanonicalAmount(null, lang), null);
    assert.strictEqual(toCanonicalAmount(undefined, lang), null);
  }
});

test('an unknown language falls back to the dot rule, never to guessing', () => {
  assert.strictEqual(toCanonicalAmount('1,000', undefined), null);
  assert.strictEqual(toCanonicalAmount('1.5', undefined), '1.5');
});
