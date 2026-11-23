import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import type {Bootstrap} from '../lib/desktop';
import {useConnectionResume} from './useConnectionResume';

const mock=vi.hoisted(()=>({
  command:vi.fn(),
  notice:null as {message:string}|null,
  preference:{accountId:'approved',connect:true,apiPort:5031 as number|null},
}));
vi.mock('react',()=>({
  useState:()=>[mock.notice,(set:(previous:typeof mock.notice)=>typeof mock.notice)=>{mock.notice=set(mock.notice);}],
  useEffect:(run:()=>void)=>run(),
}));
vi.mock('../lib/desktop',()=>({desktopAvailable:true,command:mock.command}));
vi.mock('../lib/connection-resume',async importOriginal=>({
  ...await importOriginal<typeof import('../lib/connection-resume')>(),readResume:()=>mock.preference,
}));

const wait=()=>vi.advanceTimersByTimeAsync(0);
const reconnect=()=>mock.command.mock.calls.filter(([name])=>name==='native_connect');
const api=()=>mock.command.mock.calls.filter(([name])=>name==='set_api_enabled');
let data:Bootstrap,ids:number[],component:boolean,failure:string|null,connected:string|null;

beforeEach(()=>{
  vi.useFakeTimers();
  vi.stubGlobal('window',{setInterval,clearInterval});
  mock.notice=null;
  mock.preference={accountId:'approved',connect:true,apiPort:5031};
  data={current_connection:{account_id:'approved',revision:'rev1',selection_required:false,identity_verified:true},
    status:{product:'WeRC',version:'test',stage:'test',api_enabled:false,real_wechat_verified:false},
    accounts:[],tokens:[],capabilities:[],api_address:null,bindings:[],key_statuses:[],audio_assets:[],image_assets:[],file_assets:[],operations:[],history_sources:[]};
  ids=[101];component=true;failure='NATIVE_CORE_UNAVAILABLE';connected=null;
  const status=()=>({component_available:component,connected:connected!==null,account_id:connected,error_code:null,scope:''});
  mock.command.mockReset().mockImplementation(async(name:string)=>{
    if(name==='native_status')return status();
    if(name==='bootstrap')return data;
    if(name==='native_processes')return ids;
    if(name==='native_connect'){
      if(failure)throw Object.assign(new Error(failure),{code:failure});
      connected='approved';return status();
    }
    if(name==='set_api_enabled')return;
    throw new Error(`Unexpected command: ${name}`);
  });
});
afterEach(()=>{vi.clearAllTimers();vi.useRealTimers();vi.unstubAllGlobals();});

function start(){useConnectionResume(true,data,null,async()=>{});return wait();}

describe('自动恢复原生连接',()=>{
  it('在同一微信进程加载核心后自动重试，认证成功才恢复 API',async()=>{
    await start();
    expect(reconnect()).toEqual([['native_connect',{accountId:'approved',revision:'rev1',processId:101}]]);
    expect(mock.notice?.message).toContain('等待微信核心模块就绪');
    expect(api()).toHaveLength(0);
    failure=null;
    await vi.advanceTimersByTimeAsync(25000);
    expect(reconnect()).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(5000);
    expect(reconnect()).toHaveLength(2);
    expect(api()).toEqual([['set_api_enabled',{enabled:true,port:5031}]]);
    expect(mock.notice).toBeNull();
  });
  it('重试时不绕过当前账号选择和唯一进程检查',async()=>{
    await start();
    failure=null;
    data.current_connection.selection_required=true;
    await vi.advanceTimersByTimeAsync(30000);
    expect(reconnect()).toHaveLength(1);
    data.current_connection.selection_required=false;
    ids=[101,102];
    await vi.advanceTimersByTimeAsync(5000);
    expect(reconnect()).toHaveLength(1);
    expect(api()).toHaveLength(0);
    expect(mock.notice?.message).toContain('多个微信进程');
  });
  it('组件损坏给出修复提示，不反复连接同一进程',async()=>{
    failure='NATIVE_COMPONENT_INVALID';
    await start();
    expect(mock.notice?.message).toContain('完整解压便携包');
    expect(mock.notice?.message).toContain('NATIVE_COMPONENT_INVALID');
    await vi.advanceTimersByTimeAsync(60000);
    expect(reconnect()).toHaveLength(1);
    expect(api()).toHaveLength(0);
  });
  it('构建没有组件时直接说明原因，不调用连接命令',async()=>{
    component=false;
    await start();
    expect(mock.notice?.message).toContain('原生组件不可用');
    expect(reconnect()).toHaveLength(0);
  });
});
