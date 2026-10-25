import test from 'node:test';
import assert from 'node:assert/strict';
import { copy } from '../lib/translations.js';
import { contentCopy, screenshotUrl } from '../lib/live-content.mjs';
test('future release without translations does not retain beta-specific FAQs',()=>{
 for(const language of Object.keys(copy)){
  const t=contentCopy(copy[language],{release:'v9.0',locales:{}},language);
  assert.doesNotMatch(JSON.stringify([t.faq,t.trust]),/beta|bêta|0\.1/i);
 }
});
test('release translations and English fallback update content without altering legal copy',()=>{
 const content={locales:{en:{description:'New features'},de:{description:'Neue Funktionen'}}};
 assert.equal(contentCopy(copy.de,content,'de').description,'Neue Funktionen');
 assert.equal(contentCopy(copy.fr,content,'fr').description,'New features');
 assert.deepEqual(contentCopy(copy.de,content,'de').legal,copy.de.legal);
});
test('screenshots only use local validated snapshots',()=>{
 assert.equal(screenshotUrl({screenshot_base:'https://foreign.invalid'},'photos-light'),'/screenshots/photos-light.png');
 const base='/live/snapshots/'+'a'.repeat(64)+'/screenshots';
 assert.equal(screenshotUrl({screenshot_base:base},'photos-light'),base+'/photos-light.png');
});
