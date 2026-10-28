import { expect, test, spyOn } from 'bun:test';
import { configureSharing, linkSnapshot } from './sharingCommands';

function fixture() {
    let calls: any[] = [];
    const current = {urlAccess:{uid:'link',role:'viewer',customPassword:'old secret',expirationTime:new Date('2099-01-01T00:00:00.000Z')},editorsCanShare:false};
    const ctx = {args:['/my-files/a'],options:{json:true},paths:{getPath:()=>({getNode:async()=>({uid:'a'}),sdk:{getSharingInfo:async()=>current,shareNode:async(...args:any[])=>{calls.push(args);return {};}}})}} as any;
    const settings = {nodeUid:'a',role:'viewer',password:'old secret',expiration:'2099-01-01T00:00:00.000Z',editorsCanShare:false,expected:linkSnapshot(current)};
    return {calls,ctx,settings};
}
test('saving preserves secrets and expiry unless explicitly changed; supports editor and removal', async()=>{
    const log=spyOn(console,'log').mockImplementation(()=>{});
    try {
        const {calls,ctx,settings}=fixture();
        await configureSharing(ctx,JSON.stringify(settings));
        expect(calls[0][1].urlAccess.customPassword).toBe('old secret');
        expect(calls[0][1].urlAccess.expiration.toISOString()).toBe(settings.expiration);
        await configureSharing(ctx,JSON.stringify({...settings,role:'editor',password:'',expiration:'',editorsCanShare:true}));
        expect(calls[1][1]).toEqual({urlAccess:{role:'editor',customPassword:undefined,expiration:undefined},editorsCanShare:true});
    } finally {log.mockRestore();}
});
test('stale settings and invalid dates never modify the share',async()=>{
    const {calls,ctx,settings}=fixture();
    for(const change of [{nodeUid:"other"},{expected:{}},{role:'admin'},{password:42},{expiration:'2099-02-31T00:00:00.000Z'},{expiration:'2020-01-01T00:00:00.000Z'},{editorsCanShare:'true'}]) {
        await expect(configureSharing(ctx,JSON.stringify({...settings,...change}))).rejects.toThrow();
    }
    expect(calls).toHaveLength(0);
});
