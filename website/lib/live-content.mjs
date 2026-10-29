export const reviewedRelease = 'v0.1.0-beta1';
export const liveLabels = {
 pt: {notes:'Notas da versão', original:'Notas originais da versão (inglês)', capture:'Capturas de ecrã da versão', pending:'As novas capturas de ecrã ainda não estão disponíveis.', overview:'Funcionalidades verificadas para', status:'Estado da versão', preview:'Versão de teste', stable:'Versão estável', safety:'Consulte as notas da versão atual para conhecer as limitações e as assinaturas dos pacotes. Mantenha uma cópia de segurança independente.', limits:'Onde posso consultar as limitações atuais?', answer:'As notas da versão abaixo descrevem alterações, requisitos, limitações e instruções de atualização.'},
 en: {notes:'Release notes', original:'Original release notes (English)', capture:'Screenshots from', pending:'New screenshots are not yet available.', overview:'Feature overview reviewed for', status:'Release status', preview:'Preview release', stable:'Stable release', safety:'Check the current release notes for limitations and package signatures. Keep an independent backup.', limits:'Where can I find current limitations?', answer:'The current release notes below describe changes, requirements, limitations and upgrade instructions.'},
 de: {notes:'Release-Hinweise', original:'Originale Release-Hinweise (Englisch)', capture:'Screenshots aus', pending:'Neue Screenshots sind noch nicht verfügbar.', overview:'Funktionsübersicht geprüft für', status:'Release-Status', preview:'Vorabversion', stable:'Stabile Version', safety:'Prüfe die aktuellen Release-Hinweise zu Einschränkungen und Paketsignaturen. Behalte ein unabhängiges Backup.', limits:'Wo finde ich die aktuellen Einschränkungen?', answer:'Die aktuellen Release-Hinweise unten beschreiben Änderungen, Voraussetzungen, Einschränkungen und Upgrade-Schritte.'},
 fr: {notes:'Notes de version', original:'Notes de version originales (anglais)', capture:'Captures de', pending:'Les nouvelles captures ne sont pas encore disponibles.', overview:'Fonctionnalités vérifiées pour', status:'Statut de la version', preview:'Préversion', stable:'Version stable', safety:'Consultez les notes de version actuelles pour les limites et les signatures des paquets. Gardez une sauvegarde indépendante.', limits:'Où trouver les limites actuelles ?', answer:'Les notes de version ci-dessous décrivent les changements, prérequis, limites et instructions de mise à niveau.'},
 es: {notes:'Notas de la versión', original:'Notas originales de la versión (inglés)', capture:'Capturas de', pending:'Las nuevas capturas aún no están disponibles.', overview:'Funciones verificadas para', status:'Estado de la versión', preview:'Versión preliminar', stable:'Versión estable', safety:'Consulta las notas actuales sobre limitaciones y firmas de paquetes. Mantén una copia independiente.', limits:'¿Dónde están las limitaciones actuales?', answer:'Las notas de la versión describen los cambios, requisitos, limitaciones e instrucciones de actualización.'},
 it: {notes:'Note di rilascio', original:'Note di rilascio originali (inglese)', capture:'Schermate di', pending:'Le nuove schermate non sono ancora disponibili.', overview:'Funzioni verificate per', status:'Stato della versione', preview:'Versione preliminare', stable:'Versione stabile', safety:'Consulta le note attuali per limiti e firme dei pacchetti. Mantieni un backup indipendente.', limits:'Dove trovo i limiti attuali?', answer:'Le note di rilascio descrivono modifiche, requisiti, limiti e istruzioni di aggiornamento.'}
};
export function contentCopy(base, content, language) {
 const label=liveLabels[language] || liveLabels.en;
 const t={...base, trust:base.trust.map((row,i)=>i===1?[label.status,label.safety]:row), faq:[...base.faq.slice(0,4), base.faq.at(-1)]};
 t.faq.splice(4,0,[label.limits,label.answer]);
 const localized={...content?.locales?.en,...content?.locales?.[language]};
 if (localized) for (const field of ['description','features','faq','trust','views']) if (localized[field]) t[field]=localized[field];
 return t;
}
export function screenshotUrl(content, name) {
 const base=content?.screenshot_base;
 return `${typeof base==='string' && /^\/live\/snapshots\/[a-f0-9]{64}\/screenshots$/.test(base) ? base : '/screenshots'}/${name}.png`;
}
export function readableNotes(notes) {
 return (notes || '').replace(/!\[[^\]]*\]\([^)]*\)/g,'').replace(/\[([^\]]+)\]\([^)]*\)/g,'$1').replace(/\*\*([^*]+)\*\*/g,'$1').replace(/`([^`]+)`/g,'$1').replace(/^#{1,6}\s+/gm,'').replace(/\n{3,}/g,'\n\n').trim();
}
