import {describe,it,expect} from 'vitest';
import type {ClientInstallation,DataSourceCandidate} from './desktop';
import {MAX_SETUP_DATABASES,processCandidates,recommendSetup,selectAllDatabases,type SetupDetection} from './setup-plan';
const client=(id:string,pids:number[]=[]):ClientInstallation=>({id,executable_path:`C:/${id}/Weixin.exe`,version:'4.1.11.65535',architecture:'x86_64',product_name:'Weixin',sha256:'1'.repeat(64),process_ids:pids,sources:[],compatibility:'supported',reason:''});
const source=(id:string,count=1):DataSourceCandidate=>({id,account_directory:`C:/${id}`,directory_name:id,layout:'synthetic',identity_verified:false,has_media_directory:false,databases:Array.from({length:count},(_,i)=>({relative_path:`db_storage/${i%2?'message':'session'}/${i}.db`,category:i%2?'messages':'sessions',size:4096,wal_present:false}))});
const detection=(clients:ClientInstallation[],sources:DataSourceCandidate[],main_process_ids:number[]=[]):SetupDetection=>({clients:{installations:clients,warnings:[]},catalog:{sources,warnings:[],roots_checked:1},main_process_ids});
describe('纯默认推荐',()=>{
  it('唯一候选无需输入，精确版本不被简化',()=>{
    const c=client('single',[33]),s=source('single',24);
    expect(recommendSetup(detection([c],[s]))).toEqual({installation:'single',source:'single',pid:'33'});
    expect(c.version).toBe('4.1.11.65535');expect(selectAllDatabases(s)).toHaveLength(24);
  });
  it('主窗口优先于运行候选，运行优先于离线安装；PID自动推荐',()=>{
    const report=detection([client('a'),client('b',[44]),client('z',[90,50,80])],[source('z'),source('a')],[80,50]);
    expect(recommendSetup(report)).toEqual({installation:'z',source:'a',pid:'50'});
    expect(processCandidates(report.clients.installations[2],report.main_process_ids)).toEqual([50,80,90]);
    expect(recommendSetup(detection(report.clients.installations,report.catalog.sources))).toEqual({installation:'b',source:'a',pid:'44'});
  });
  it('多候选排序不依赖发现顺序，不把目录推荐当身份认证',()=>{
    const clients=[client('z',[3]),client('a',[5,2,2])],sources=[source('z'),source('a')];
    const before=JSON.stringify({clients,sources});
    expect(recommendSetup(detection(clients,sources))).toEqual(recommendSetup(detection([...clients].reverse(),[...sources].reverse())));
    expect(recommendSetup(detection(clients,sources))).toEqual({installation:'a',source:'a',pid:'2'});
    expect(JSON.stringify({clients,sources})).toBe(before);expect(sources.every(s=>!s.identity_verified)).toBe(true);
  });
  it('无PID不伪造进程；非法PID不进入候选',()=>{
    expect(recommendSetup(detection([client('a')],[source('a')]))).toEqual({installation:'a',source:'a',pid:''});
    expect(processCandidates(client('a',[0,-1,NaN,1.5,0x100000000,7]),[])).toEqual([7]);
    expect(recommendSetup(detection([],[]))).toEqual({installation:'',source:'',pid:''});
  });
});
describe('选择全部发现库',()=>{
  it.each([24,33,512,513])('%i 个库不受旧16库上限或静默截断影响',count=>{
    const s=source('all',count);expect(selectAllDatabases(s)).toHaveLength(count);
    expect(selectAllDatabases(s).length<=MAX_SETUP_DATABASES).toBe(count<=512);
  });
  it('包含大于64MiB、FTS、其他分类和小库，无checkbox范围过滤',()=>{
    const s=source('all',33);s.databases[0]={...s.databases[0],size:65*1024*1024,category:'fts'};s.databases[1]={...s.databases[1],size:0,category:'other'};
    const paths=selectAllDatabases(s);expect(paths).toHaveLength(33);expect(paths).toContain(s.databases[0].relative_path);expect(paths).toContain(s.databases[1].relative_path);
    expect(paths).toEqual(selectAllDatabases({...s,databases:[...s.databases].reverse()}));
  });
  it('空来源为空，重复发现不重复提交',()=>{
    expect(selectAllDatabases(undefined)).toEqual([]);const s=source('a');s.databases.push({...s.databases[0]});expect(selectAllDatabases(s)).toHaveLength(1);
  });
});
