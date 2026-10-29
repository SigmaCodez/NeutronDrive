'use client';
import { createContext, useContext, useEffect, useState } from 'react';
import { resolveLanguage, languages, storageKey } from '../lib/preferences.mjs';
import { copy } from '../lib/translations';
const LanguageContext = createContext(null);
export function LanguageProvider({ children }) {
  const [preference, setPreference] = useState('auto');
  const [browserLanguage, setBrowserLanguage] = useState('en');
  useEffect(() => {
    function detect() { setBrowserLanguage(navigator.languages?.[0] || navigator.language || 'en'); }
    detect();
    try { const saved = localStorage.getItem(storageKey); if (languages.includes(saved)) setPreference(saved); } catch {}
    window.addEventListener('languagechange', detect);
    return () => window.removeEventListener('languagechange', detect);
  }, []);
  const language = resolveLanguage(preference, browserLanguage);
  useEffect(() => { document.documentElement.lang = language; }, [language]);
  function choose(value) {
    const next = languages.includes(value) ? value : 'auto';
    setPreference(next);
    try { if (next === 'auto') localStorage.removeItem(storageKey); else localStorage.setItem(storageKey, next); } catch {}
  }
  return <LanguageContext.Provider value={{ language, preference, choose, t: copy[language] }}>{children}</LanguageContext.Provider>;
}
export const useLanguage = () => useContext(LanguageContext);
export function LanguageSelector() {
  const { preference, choose, t } = useLanguage();
  return <label className="language-picker"><span aria-hidden="true">◎</span><span className="sr-only">{t.language}</span><select aria-label={t.language} value={preference} onChange={e => choose(e.target.value)}><option value="auto">{t.automatic}</option><option value="en">English</option><option value="de">Deutsch</option><option value="fr">Français</option><option value="es">Español</option><option value="it">Italiano</option><option value="pt">Português</option></select></label>;
}
