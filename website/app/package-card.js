'use client';
import { useState } from 'react';
import { packageAssets } from '../lib/downloads.mjs';
export default function PackageCard({format,description,note,release,failed,labels}) {
  const [selection,setSelection] = useState('');
  const assets = packageAssets(release,format);
  const asset = assets.find(a=>a.name===selection) || assets[0];
  return <article className="package">
    <div><h3>{format}</h3><span aria-hidden="true">↓</span></div><p>{description}</p><small>{note}</small>
    {assets.length>1 && <label className="package-choice">{labels.variant}<select aria-label={`${format}: ${labels.variant}`} value={asset.name} onChange={e=>setSelection(e.target.value)}>{assets.map(a=><option key={a.name} value={a.name}>{a.name}</option>)}</select></label>}
    {asset ? <><span className="asset-name">{asset.name}</span><a className="package-download" href={asset.browser_download_url} download={asset.name} aria-label={`${labels.download}: ${asset.name}`}>{labels.download} <span aria-hidden="true">↓</span></a></> : <p className="package-unavailable" role="status">{release || failed ? labels.unavailable : labels.loading}</p>}
  </article>;
}
