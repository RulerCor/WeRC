import {beforeEach,describe,expect,it,vi} from 'vitest';
import {readResume,rememberApi,rememberConnection} from './connection-resume';
describe('恢复用户确认的连接',()=>{
  let items:Map<string,string>;
  beforeEach(()=>{items=new Map();vi.stubGlobal('localStorage',{getItem:(k:string)=>items.get(k)??null,setItem:(k:string,v:string)=>items.set(k,v)});});
  it('默认不自动连接或开启API；单独启用API不推断微信授权',()=>{expect(readResume()).toBeNull();rememberApi('one',5031);expect(readResume()).toEqual({accountId:'one',connect:false,apiPort:5031});});
  it('保留同一账号选择，手动断开/停止覆盖恢复设置',()=>{rememberConnection('one',true);rememberApi('one',5031);rememberConnection('one',false);expect(readResume()).toEqual({accountId:'one',connect:false,apiPort:5031});rememberApi('one',null);expect(readResume()).toEqual({accountId:'one',connect:false,apiPort:null});});
  it('换账号不沿用原API端口和连接选择',()=>{rememberConnection('one',true);rememberApi('one',5031);rememberConnection('two',false);expect(readResume()).toEqual({accountId:'two',connect:false,apiPort:null});});
  it('拒绝损坏和非法端口偏好',()=>{for(const value of ['bad',JSON.stringify({accountId:'one',connect:true,apiPort:80}),JSON.stringify({accountId:'one',connect:true,apiPort:65536})]){items.set('werc.connection-resume.v1',value);expect(readResume()).toBeNull();}});
});
