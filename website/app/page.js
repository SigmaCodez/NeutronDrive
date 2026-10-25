'use client';
import { useEffect, useState } from 'react';
import PackageCard from './package-card';
import { contentCopy, liveLabels, reviewedRelease, screenshotUrl, readableNotes } from '../lib/live-content.mjs';
import { useLanguage, LanguageSelector } from './language';
import { newestRelease, releaseFallback } from '../lib/preferences.mjs';
const repo = 'https://github.com/SigmaCodez/NeutronDrive';
const files = ['files-grid-dark', 'files-grid-light', 'photos-light', 'computer-backup-light', 'sharing-light'];
const formats = ['DEB','RPM','AppImage','Flatpak','Snap','Arch'];
const icons = ['↔','↓','▧','⌘'];
function Arrow(){return <span aria-hidden="true">↗</span>}
function Lines({text}) {return <>{text[0]}<br/>{text[1]}</>}
export default function Home(){
  const { t: baseCopy, language } = useLanguage();
  const [content,setContent]=useState(null);
  const t=contentCopy(baseCopy, content, language);
  const live=liveLabels[language] || liveLabels.en;
  const shot=name=>screenshotUrl(content,name);
  useEffect(() => {
    let disposed=false;
    let controller;
    async function refreshContent() {
      controller?.abort();
      const request=new AbortController(); controller=request;
      const timeout=setTimeout(()=>request.abort(),10000);
      try {
        const response=await fetch('/live/current/content.json',{cache:'no-store',signal:request.signal});
        if (!response.ok) return;
        const data=await response.json();
        if (!disposed && controller===request && data.schema===1 && typeof data.release==='string') setContent(data);
      } catch { /* Keep the last verified snapshot with its actual version. */ }
      finally {clearTimeout(timeout);}
    }
    refreshContent();
    const timer=setInterval(refreshContent,5*60*1000);
    function visible(){if(document.visibilityState==='visible') refreshContent();}
    document.addEventListener('visibilitychange',visible);
    return ()=>{disposed=true;controller?.abort();clearInterval(timer);document.removeEventListener('visibilitychange',visible);};
  },[]);
  const [view,setView]=useState(0);
  const [latest,setLatest]=useState(null);
  const [releaseFailed,setReleaseFailed]=useState(false);
  useEffect(() => { document.title = `NeutronDrive — ${t.title}`; }, [t]);
  useEffect(() => {
    let disposed = false;
    let controller;
    async function refresh() {
      controller?.abort();
      const request = new AbortController();
      controller = request;
      const timeout = setTimeout(() => request.abort(), 10000);
      try {
        const response = await fetch('/api/releases', { signal: request.signal, cache: 'no-store' });
        if (!response.ok) throw new Error('Release request failed');
        const result = newestRelease(await response.json());
        if (!result) throw new Error('No published release');
        if (!disposed && controller === request) { setLatest(result); setReleaseFailed(false); }
      } catch {
        // Never leave an old version promoted as current after a failed refresh.
        if (!disposed && controller === request) { setLatest(null); setReleaseFailed(true); }
      } finally { clearTimeout(timeout); }
    }
    refresh();
    const timer = setInterval(refresh, 5 * 60 * 1000);
    function onVisible() { if (document.visibilityState === 'visible') refresh(); }
    document.addEventListener('visibilitychange', onVisible);
    return () => { disposed = true; controller?.abort(); clearInterval(timer); document.removeEventListener('visibilitychange', onVisible); };
  }, []);
  const release = latest?.html_url || releaseFallback;
  const version = latest?.tag_name || t.latest;
  return <>
    <a className="skip" href="#main">{t.skip}</a>
    <header className="header wrap"><a className="brand" href="#" aria-label={t.home}><img src="/icon.svg" width="35" height="35" alt=""/>Neutron<span>Drive</span></a><nav aria-label={t.home}><a href="#features">{t.nav[0]}</a><a href="#einblicke">{t.nav[1]}</a><a href="#faq">{t.nav[2]}</a></nav><div className="header-actions"><LanguageSelector/><a className="nav-download" href="#download">{t.download} <span aria-hidden="true">↓</span></a></div></header>
    <main id="main">
      <section className="hero wrap">
        <a className="release" href={release}><span className="status-dot"/>{version}<span className="release-sep">/</span>{t.open}<Arrow/></a>
        <div className="hero-heading"><h1>{t.hero[0]}<br/><span>{t.hero[1]}</span></h1><div className="hero-side"><p>{t.intro}<br/>{t.description}</p><div className="actions"><a className="button primary" href="#download">{t.download}<span aria-hidden="true">↓</span></a><a className="text-link" href={repo}>GitHub <Arrow/></a></div><p className="small">{t.free}</p></div></div>
        <div className="hero-visual"><div className="visual-caption"><span><i/> CLOUD, MEET DESKTOP.</span><span>LINUX NATIVE / QT QUICK</span></div><img className="hero-shot" src={shot("files-grid-dark")} alt={t.heroAlt} width="1220" height="820" fetchPriority="high"/><div className="floating-note"><span className="note-icon">↔</span><div>{t.flow}<small>{t.flowSmall}</small></div></div><div className="visual-bottom"><span>{t.everyday}</span><span>01 — DESKTOP PREVIEW</span></div></div>
        <div className="proof-strip"><span>{t.builtFor} <strong>Linux.</strong></span><span>Rust <i>+</i> Qt Quick</span><span>{t.languages}</span><span>{t.license} <Arrow/></span></div>
      </section>
      <section id="features" className="features wrap section"><div className="section-top"><p className="eyebrow">{t.featureTop}</p><span>{t.featureAside}</span></div><h2><Lines text={t.featureHeading}/></h2>{!content?.locales?.[language]?.features && !content?.locales?.en?.features && <p className="small">{live.overview} {reviewedRelease}</p>}<div className="feature-grid">{t.features.map(([title,text],i)=><article className="feature" key={i}><div className="feature-head"><span className="feature-icon" aria-hidden="true">{icons[i]}</span><span>0{i+1}</span></div><h3>{title}</h3><p>{text}</p></article>)}</div></section>
      <section id="einblicke" className="showcase section"><div className="wrap"><div className="section-top"><p className="eyebrow">{t.previewTop}</p><span>{t.previewAside}</span></div><div className="showcase-head"><h2><Lines text={t.previewHeading}/></h2><p><Lines text={t.previewIntro}/></p></div><div className="tabs" role="group" aria-label={t.selectScreenshot}>{t.views.map((v,i)=><button key={i} onClick={()=>setView(i)} aria-pressed={view===i}>{v[0]}</button>)}</div><figure className="gallery"><a href={shot(files[view])} target="_blank" rel="noreferrer" aria-label={`${t.views[view][0]}: ${t.fullScreenshot}`}><img src={shot(files[view])} alt={`NeutronDrive: ${t.views[view][0]} — ${t.sampleScreenshot}`} width={view===4?600:1220} height={view===4?430:820} loading="lazy"/></a><figcaption aria-live="polite"><div><strong>{t.views[view][1]}</strong><p>{t.views[view][2]}</p></div><span>0{view+1} / 05</span></figcaption></figure><p className="preview-note">{live.capture} {content?.screenshots_release || reviewedRelease} · {t.sampleScreenshot}. {content?.screenshots_pending ? live.pending : ""}</p></div></section>
      <section className="trust wrap section"><div><p className="eyebrow">{t.trustTop}</p><h2>{t.trustHeading[0]}<br/><span>{t.trustHeading[1]}</span></h2><p>{t.trustIntro}</p><a className="text-link" href={`${repo}/blob/main/SECURITY.md`}>{t.security}<Arrow/></a></div><div className="trust-notes">{t.trust.map(([title,text],i)=><article key={i}><span>0{i+1}</span><div><h3>{title}</h3><p>{text}</p></div></article>)}</div></section>
      <section id="download" className="download section"><div className="wrap"><div className="download-heading"><div><p className="eyebrow">{t.downloadTop}</p><h2><Lines text={t.downloadHeading}/></h2></div><div><p><Lines text={t.downloadIntro}/></p><a className="version-pill" href={release}>{version}</a></div></div><p className="download-note">{latest ? (latest.prerelease ? live.preview : live.stable) : ""}</p><p className="download-note" role="status">{releaseFailed ? t.releaseError : t.releaseNote}</p><div className="packages">{t.packages.map(([os,note],i)=><PackageCard key={formats[i]} format={formats[i]} description={os} note={note} release={latest} failed={releaseFailed} labels={t.downloadUi}/>)}</div><p className="download-note">{t.downloadNote} <a href={`${repo}#install`}>{t.install}</a>.</p><div className="steps">{t.steps.map(([title,text],i)=><div key={i}><span>{i+1}</span><p><strong>{title}</strong>{text}</p></div>)}</div></div></section>
      {content && <section id="release-notes" className="wrap section release-notes"><p className="eyebrow">{live.notes} · {content.release}</p><details><summary>{live.notes} <span>{content.release}</span></summary><p className="small">{content.locales?.[language]?.releaseNotes ? live.notes : live.original}</p><div className="release-notes-body" lang={content.locales?.[language]?.releaseNotes ? language : 'en'}>{readableNotes(content.locales?.[language]?.releaseNotes || content.locales?.en?.releaseNotes || content.release_notes)}</div><a className="text-link" href={content.release_url}>GitHub <Arrow/></a></details></section>}
      <section id="faq" className="faq wrap section"><div><p className="eyebrow">{t.faqTop}</p><h2>{t.faqHeading}</h2><p>{t.faqIntro}<br/><a className="text-link" href={`${repo}#readme`}>{t.docs}<Arrow/></a></p></div><div>{t.faq.map(([q,a],i)=><details key={i}><summary>{q}<span aria-hidden="true">+</span></summary><p>{a}</p></details>)}</div></section>
      <section className="closing wrap"><img src="/icon.svg" width="60" height="60" alt=""/><h2><Lines text={t.closing}/></h2><a className="button primary" href={repo}>{t.github}<Arrow/></a></section>
    </main><footer className="wrap"><div className="footer-top"><a className="brand" href="#"><img src="/icon.svg" width="28" height="28" alt=""/>Neutron<span>Drive</span></a><div><a href={repo}>{t.source}<Arrow/></a><a href={`${repo}/issues`}>{t.feedback}<Arrow/></a><a href={`${repo}/blob/main/LICENSE`}>{t.license}<Arrow/></a><a href="/datenschutz/">{t.privacy}</a><a href="/impressum/">{t.imprint}</a></div></div><div className="footer-bottom"><p>{t.independent}</p><p>Made for Linux. Open by nature.</p></div></footer>
  </>;
}
