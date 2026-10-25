export const languages = ['en', 'de', 'fr', 'es', 'it'];
export const storageKey = 'neutrondrive-language';
export function resolveLanguage(preference, browserLanguage = 'en') {
  if (languages.includes(preference)) return preference;
  const base = String(browserLanguage).toLowerCase().split(/[-_]/)[0];
  return languages.includes(base) ? base : 'en';
}
export const releaseFallback = 'https://github.com/SigmaCodez/NeutronDrive/releases';
export function newestRelease(data) {
  if (!Array.isArray(data)) return null;
  return data.filter(r => r && !r.draft && typeof r.tag_name === 'string' &&
    Number.isFinite(Date.parse(r.published_at)) && typeof r.html_url === 'string' &&
    /^https:\/\/github\.com\/SigmaCodez\/NeutronDrive\/releases\/tag\/[^\s?#]+$/.test(r.html_url))
    .sort((a, b) => Date.parse(b.published_at) - Date.parse(a.published_at))[0] ?? null;
}
