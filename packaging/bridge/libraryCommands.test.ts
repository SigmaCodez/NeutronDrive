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

import { spyOn } from 'bun:test';
import { CommandPhotoLibrary } from './libraryCommands';
test('library inventory counts unique timeline entries without fetching originals or previews', async () => {
    const log=spyOn(console,'log').mockImplementation(()=>{});
    try {
        await new CommandPhotoLibrary().action({args:[JSON.stringify({action:'index'})],photosSdk:{async *iterateTimeline(){yield {nodeUid:'a'};yield {nodeUid:'a'};yield {nodeUid:'b'};}}} as any);
        expect(JSON.parse(log.mock.calls[0][0])).toEqual({items:[{uid:'a'},{uid:'b'}],total:2});
    } finally {log.mockRestore();}
});
test('library validates batch sizes and duplicates before SDK calls', async () => {
    for(const ids of [[],['a','a'],Array.from({length:49},(_,i)=>String(i))]) {
        await expect(new CommandPhotoLibrary().action({args:[JSON.stringify({action:'details',ids})]} as any)).rejects.toThrow();
    }
});
test('map metadata accepts zero coordinates and rejects invalid coordinates without original downloads', async () => {
    const log=spyOn(console,'log').mockImplementation(()=>{});
    try {
        await new CommandPhotoLibrary().action({args:[JSON.stringify({action:'details',ids:['a','b'],thumbnails:false})],photosSdk:{async *iterateNodes(){
            yield {uid:'a',name:{ok:true,value:'A'},activeRevision:{uid:'revision-a',claimedAdditionalMetadata:{Location:{Latitude:0,Longitude:0}}}};
            yield {uid:'b',name:{ok:true,value:'B'},activeRevision:{claimedAdditionalMetadata:{Location:{Latitude:91,Longitude:0}}}};
        }}} as any);
        const response=JSON.parse(log.mock.calls[0][0]);
        expect(response.photos[0].revision).toBe('revision-a');
        expect(response.photos[0].location).toEqual({latitude:0,longitude:0});expect(response.photos[1].location).toBeNull();
    } finally {log.mockRestore();}
});
test('album deletion preserves album-only photos and never requests force deletion', async () => {
    let options:any;
    const log=spyOn(console,'log').mockImplementation(()=>{});
    try {
        await new CommandPhotoLibrary().action({args:[JSON.stringify({action:'delete',album:'a',force:true})],photosSdk:{async deleteAlbum(_:string,value:any){options=value;}}} as any);
        expect(options).toEqual({saveToTimeline:true});
    } finally {log.mockRestore();}
});
test('partial album changes do not report success', async () => {
    await expect(new CommandPhotoLibrary().action({args:[JSON.stringify({action:'add',album:'a',ids:['b']})],photosSdk:{async *addPhotosToAlbum(){yield {uid:'b',ok:false,error:new Error('denied')};}}} as any)).rejects.toThrow('Some album changes failed');
});

import { CommandAccountStorage } from './libraryCommands';
test('account exposes only quota and sanitized display identity, never keys', async () => {
    const log=spyOn(console,'log').mockImplementation(()=>{});
    try {
        let user:any={MaxSpace:10,UsedSpace:12,Name:'alex',DisplayName:'Alex Morgan',Email:'alex@example.invalid',Keys:['private'],AccessToken:'secret'};
        const context={apiClient:{baseUrlWithProtocol:'https://example.invalid',authenticatedRequest:{get:(url:string)=>{
            expect(url).toBe('https://example.invalid/core/v4/users');return {json:async()=>({User:user})};
        }}}} as any;
        await new CommandAccountStorage().action(context);
        expect(JSON.parse(log.mock.calls[0][0])).toEqual({total:10,used:12,free:0,account:{name:'Alex Morgan',email:'alex@example.invalid'}});
        user={MaxSpace:-1,UsedSpace:0};
        await expect(new CommandAccountStorage().action(context)).rejects.toThrow('Invalid account');
        expect(log.mock.calls.length).toBe(1);
    } finally {log.mockRestore();}
});
