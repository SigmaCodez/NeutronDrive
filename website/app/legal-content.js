'use client';
import { useEffect } from 'react';
import { useLanguage, LanguageSelector } from './language';
const repo = 'https://github.com/SigmaCodez/NeutronDrive';
const email = <a href="mailto:sigmacodez@silvamail.org">sigmacodez@silvamail.org</a>;
export default function LegalContent({ type }) {
 const {t} = useLanguage();
 const l = t.legal;
 const title = type==='imprint' ? t.imprint : t.privacy;
 useEffect(() => { document.title = `${title} — NeutronDrive`; }, [title]);
 return <main className="wrap legal"><div className="legal-nav"><a href="/">← {t.back}</a><LanguageSelector/></div><h1>{title}</h1>{type==='imprint' ? <>
 <h2>{l.operator}</h2><p>SigmaCodez<br/>[location removed]<br/>{l.address}</p><p>{l.email}: {email}</p><p>{l.privateText}</p>
 <h2>{l.independentTitle}</h2><p>{l.independentText}</p><h2>{l.sourceTitle}</h2><p>{l.sourceBefore} <a href={`${repo}/blob/main/LICENSE`}>{t.license}</a>. {l.sourceAfter}</p><h2>{l.contactTitle}</h2><p>{l.contactText} <a href="/datenschutz/">{t.privacy}</a>.</p>
 </> : <>
 <p>{l.privacyIntro}</p><h2>{l.technicalTitle}</h2><p>{l.technicalText}</p><h2>{l.trackingTitle}</h2><p>{l.trackingText}</p><h2>{l.releaseTitle}</h2><p>{l.releaseText}</p><h2>{l.linksTitle}</h2><p>{l.linksText}</p><h2>{l.appTitle}</h2><p>{l.appText} <a href={`${repo}/blob/main/SECURITY.md`}>{l.securityLink}</a>.</p><h2>{l.responsibleTitle}</h2><p>{l.responsibleText} {email}. {l.rights}</p><h2>{l.projectTitle}</h2><p>{l.projectBefore} <a href="https://github.com/SigmaCodez">SigmaCodez / GitHub</a>. {l.projectAfter}</p>
 </>}</main>;
}
