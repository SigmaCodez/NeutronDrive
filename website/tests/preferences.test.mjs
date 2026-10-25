import test from 'node:test';
import assert from 'node:assert/strict';
import { languages, resolveLanguage, newestRelease, releaseFallback } from '../lib/preferences.mjs';
import { copy } from '../lib/translations.js';
test('primary browser language, region variants and English fallback', () => {
 for (const [browser, expected] of [['de-CH','de'],['fr-CA','fr'],['es-MX','es'],['it-IT','it'],['en-GB','en'],['DE_ch','de'],['ja-JP','en'],['','en']]) {
  assert.equal(resolveLanguage('auto',browser),expected);
 }
 assert.equal(resolveLanguage('fr','de-CH'),'fr');
 assert.equal(resolveLanguage('invalid','it-CH'),'it');
});
function shape(value) {
 if (Array.isArray(value)) return value.map(shape);
 if (typeof value==='object') return Object.fromEntries(Object.entries(value).map(([k,v])=>[k,shape(v)]));
 assert.equal(typeof value,'string'); assert.ok(value.trim()); return 'string';
}
test('all five languages have complete home and legal translations', () => {
 assert.deepEqual(Object.keys(copy).sort(), [...languages].sort());
 for (const lang of languages) {
  assert.deepEqual(shape(copy[lang]),shape(copy.en),lang);
  assert.equal(JSON.stringify(copy[lang]).toLowerCase().includes('codezwerk'),false);
 }
});
const release = (tag,date,extra={}) => ({tag_name:tag,published_at:date,html_url:`${releaseFallback}/tag/${tag}`,draft:false,...extra});
test('most recently published release includes previews and ignores drafts', () => {
 const old = release('v1','2026-01-01');
 const newer = release('v2-alpha','2026-02-01',{prerelease:true});
 const draft = release('v3','2026-03-01',{draft:true});
 assert.equal(newestRelease([draft,old,newer]).tag_name,'v2-alpha');
 assert.equal(newestRelease([newer,old]).html_url,newer.html_url);
});
test('invalid or unavailable release data falls back safely', () => {
 for (const input of [null,{},[],[{draft:false}], [release('v1','invalid')], [release('v1','2026-01-01',{html_url:'https://example.com/release'})]]) assert.equal(newestRelease(input),null);
 assert.equal(releaseFallback,'https://github.com/SigmaCodez/NeutronDrive/releases');
});
