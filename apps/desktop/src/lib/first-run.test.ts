import {beforeEach,describe,expect,it,vi} from 'vitest';
import {FIRST_RUN_KEY,readFirstRun,saveFirstRun} from './first-run';
describe('首次设置进度',()=>{
  let items:Map<string,string>;
  beforeEach(()=>{items=new Map();vi.stubGlobal('localStorage',{getItem:(k:string)=>items.get(k)??null,setItem:(k:string,v:string)=>items.set(k,v)});});
  it('新用户从微信检测开始，未知旧值不跳过设置',()=>{expect(readFirstRun()).toBe('wechat');items.set(FIRST_RUN_KEY,'unexpected');expect(readFirstRun()).toBe('wechat');});
  it('保留阶段和完成状态，不保存令牌等秘密',()=>{for(const step of ['wechat','prepare','bot','done','complete'] as const){saveFirstRun(step);expect(readFirstRun()).toBe(step);}expect([...items.keys()]).toEqual([FIRST_RUN_KEY]);});
  it('偏好存储不可用时仍允许当前会话操作',()=>{vi.stubGlobal('localStorage',{getItem:()=>{throw Error('denied');},setItem:()=>{throw Error('denied');}});expect(readFirstRun()).toBe('wechat');expect(()=>saveFirstRun('complete')).not.toThrow();});
});
