import { expect, test } from 'bun:test';
import { CommandRestoreVersion, CommandPhotoPage } from './libraryCommands';

test('restore rejects changed current revision and foreign historical versions', async () => {
    const calls: string[] = [];
    const ctx = { args: ['/my-files/file','old','current'], paths: {getPath:()=>({getNode:async()=>({type:'file',uid:'file',activeRevision:{uid:'changed'}})})},
        sdk: {async getNode(){return {activeRevision:{uid:'current'}};},async *iterateRevisions(){yield {uid:'foreign',state:'superseded'};},async restoreRevision(uid:string){calls.push(uid);}} } as any;
    await expect(new CommandRestoreVersion().action(ctx)).rejects.toThrow('changed');
    ctx.paths.getPath=()=>({getNode:async()=>({type:'file',uid:'file',activeRevision:{uid:'current'}})});
    await expect(new CommandRestoreVersion().action(ctx)).rejects.toThrow('no longer');
    expect(calls).toEqual([]);
    ctx.sdk.iterateRevisions=async function*(){yield {uid:'old',state:'superseded'};};
    await new CommandRestoreVersion().action(ctx);
    expect(calls).toEqual(['old']);
    ctx.sdk.getNode=async()=>({activeRevision:{uid:'new-edit'}});
    await expect(new CommandRestoreVersion().action(ctx)).rejects.toThrow('changed while');
    expect(calls).toEqual(['old']);
});
test('photo page rejects invalid offsets before calling SDK', async () => {
    for(const offset of ['-1','x','9007199254740992']) await expect(new CommandPhotoPage().action({args:[offset]} as any)).rejects.toThrow();
});
test('photo page bounds metadata and ignores thumbnail failures', async () => {
    let requested: string[]=[];
    const ctx={args:['0'],photosSdk:{async *iterateTimeline(){for(let i=0;i<100;i++)yield {nodeUid:String(i)};},
        async *iterateThumbnails(){throw new Error('offline');},
        async *iterateNodes(ids:string[]){requested=ids;for(const uid of ids)yield {uid,name:{ok:true,value:uid}};}}} as any;
    await new CommandPhotoPage().action(ctx);
    expect(requested.length).toBe(48);
});
