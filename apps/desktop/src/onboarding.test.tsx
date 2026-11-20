import {describe,it,expect,vi} from 'vitest';
import {renderToStaticMarkup} from 'react-dom/server';
import {Onboarding} from './pages/Onboarding';
import {setupProgress} from './lib/setup-progress';
import {shouldOpenSetup,type OnboardingController,type SetupItem,type SetupRun,type SetupStatus} from './hooks/useOnboarding';
vi.mock('./lib/desktop',()=>({desktopAvailable:false,command:vi.fn()}));
const flow:OnboardingController={status:{run:null,active:false,working:false,dismissed:false},visible:true,busy:false,error:'',restored:false,action:async()=>{},pause:async()=>{},close:async()=>{},open:()=>{}};
const run:SetupRun={id:'synthetic',revision:1,binding:{account_id:'test',installation:{id:'fixture',executable_path:'C:/fixture/Weixin.exe',version:'4.1.11.65535',architecture:'x86_64',product_name:'fixture',sha256:'1'.repeat(64),process_ids:[],sources:[],compatibility:'unsupported',reason:'synthetic'},bound_at:'test',checked_at:'test',validation:'metadata_only'},process_id:42,key_revision:1,phase:'waiting',items:[{path:'C:/private-owner/session.db',key_ready:true,skipped:false,error:null,source:null}]};
function render(items=run.items,phase=run.phase,working=false,busy=false) {
  return renderToStaticMarkup(<Onboarding flow={{...flow,busy,status:{run:{...run,items,phase},active:false,working,dismissed:false}}} navigate={()=>{}}/>);
}
const importedSource={id:'history',account_id:'test',source_sha256:'1'.repeat(64),schema:'synthetic',encryption:'fixture',message_count:2,contact_count:1,session_count:1,imported_at:'test',real_wechat_verified:false};
describe('启动引导显示策略',()=>{
  const fresh:SetupStatus={run:null,active:false,working:false,dismissed:false};
  it('首次连接也不自动打开可选快照导入',()=>{
    expect(shouldOpenSetup(fresh,0)).toBe(false);
    expect(shouldOpenSetup(fresh,1)).toBe(false);
    expect(shouldOpenSetup({...fresh,dismissed:true},0)).toBe(false);
  });
  it('所有已保存阶段均需手动打开，不因账号缺失或dismissed状态强制展开',()=>{
    for(const phase of ['keys','waiting','importing','failed','complete','partial'])for(const dismissed of [false,true])for(const accounts of [0,1]){
      expect(shouldOpenSetup({...fresh,dismissed,run:{...run,phase}},accounts)).toBe(false);
    }
  });
  it('恢复的旧失败结果明确未重新执行，不删除原错误',()=>{
    const html=renderToStaticMarkup(<Onboarding flow={{...flow,restored:true,status:{...fresh,run:{...run,phase:'failed',items:[{...run.items[0],error:'WAL_SALT_MISMATCH',outcome:'failed'}]}}}} navigate={()=>{}}/>);
    expect(html).toContain('上次保存的设置结果，尚未重新执行');
    expect(html).toContain('WAL_SALT_MISMATCH');
    expect(html).toContain('继续 / 重试');
    expect(render()).not.toContain('上次保存的设置结果');
  });
});
describe('真实库级进度',()=>{
  it('密钥首验不算导入处理完成',()=>{
    expect(setupProgress(run,true).processed).toBe(1);
    expect(setupProgress(run,false).processed).toBe(0);
    expect(setupProgress(run,false).fraction).toBe(0);
  });
  it('已处理包含独立结算的未适配、缺密钥和失败，当前库不提前结算',()=>{
    const items:SetupItem[]=[
      {...run.items[0],source:importedSource,outcome:'imported'},
      {...run.items[0],path:'fts.db',skipped:true,outcome:'unsupported'},
      {...run.items[0],path:'missing.db',key_ready:false,skipped:true,outcome:'key_deferred'},
      {...run.items[0],path:'failed.db',error:'PAGE_AUTH_FAILED',outcome:'failed'},
      {...run.items[0],path:'current.db'},
    ];
    const progress=setupProgress({...run,phase:'importing',items},false);
    expect(progress.processed).toBe(4);expect(progress.total).toBe(5);expect(progress.fraction).toBe(0.8);expect(progress.currentIndex).toBe(4);expect(progress.current?.path).toBe('current.db');
    expect(setupProgress({...run,phase:'failed',items},false).currentIndex).toBe(3);
  });
  it('空计划不除零，全部暂缓也不代表导入成功',()=>{
    expect(setupProgress({...run,items:[]},false).fraction).toBe(0);
    const items=[{...run.items[0],key_ready:false,skipped:true,outcome:'key_deferred' as const,error:'KEY_NOT_FOUND'}];
    expect(setupProgress({...run,items},false).fraction).toBe(1);
    const html=render(items,'partial');
    expect(html).toContain('已处理 1/1 个库');expect(html).toContain('不等于全部导入');expect(html).toContain('尚无记录导入');expect(html).not.toContain('查看记录');
  });
  it('处理中显示真实比值、当前库、可见暂停入口，不虚构当前库百分比',()=>{
    const html=renderToStaticMarkup(<Onboarding flow={{...flow,busy:true,status:{run:{...run,phase:'importing',items:[{...run.items[0],source:importedSource},{...run.items[0],path:'C:/private-owner/message/message_2.db'}]},active:true,working:true,dismissed:false}}} navigate={()=>{}}/>);
    expect(html).toContain('已处理 1/2 个库');expect(html).toContain('role="progressbar"');expect(html).toContain('aria-valuenow="1"');expect(html).toContain('scaleX(0.5)');expect(html).toContain('当前处理 / 2');expect(html).toContain('message/message_2.db');expect(html).toContain('完成后更新进度');expect(html).toContain('正在处理…');expect(html).toContain('暂停');expect(html).not.toContain('预计');expect(html).not.toContain('C:/private-owner');
  });
});
describe('首次引导默认点击流程与真实阶段',()=>{
  it('首屏不自动检测，按钮旁明确本机元数据范围，无checkbox',()=>{
    const html=renderToStaticMarkup(<Onboarding flow={flow} navigate={()=>{}}/>);
    expect(html).toContain('点击即允许检测本机安装、进程和数据目录元数据');expect(html).toContain('开始检测');expect(html).toContain('disabled=""');
    expect(html).not.toContain('type="checkbox"');expect(html).toContain('稍后设置');
    expect(html).toMatch(/<details><summary>高级设置（可选）<\/summary>[\s\S]*引导数据根目录/);
  });
  it('首验完成不是导入完成，恢复后按钮本身重新确认',()=>{
    const html=render();
    expect(html).toContain('引导已暂停');expect(html).toContain('首验通过 / 待导入');expect(html).toContain('首验 1/1');expect(html).toContain('已导入 0');
    expect(html).toContain('点击继续即重新确认');expect(html).toContain('继续 / 重试');expect(html).not.toContain('type="checkbox"');expect(html).not.toContain('离线记录已导入');expect(html).not.toContain('C:/private-owner');
  });
  it('旧run的跳过项不假报部分导入成功，无source不出现查看记录',()=>{
    const html=render([{...run.items[0],skipped:true,error:'unknown_schema'}],'partial');
    expect(html).toContain('配置结束，尚无记录导入');expect(html).toContain('未导入 / 已跳过 / unknown_schema');expect(html).toContain('不代表实时收发可用');
    expect(html).not.toContain('部分记录已导入');expect(html).not.toContain('查看记录');expect(html).not.toContain('离线记录已导入');
  });
  it('outcome准确区分已导入、未适配、超预算与失败，钥首验不计入导入',()=>{
    const items:SetupItem[]=[
      {...run.items[0],outcome:'imported',source:importedSource},
      {...run.items[0],path:'already.db',outcome:'already_imported',source:importedSource},
      {...run.items[0],path:'fts.db',outcome:'unsupported',skipped:true,error:'unknown_schema'},
      {...run.items[0],path:'large.db',outcome:'resource_deferred',skipped:true},
      {...run.items[0],path:'failed.db',outcome:'failed',error:'read_failed'},
      {...run.items[0],path:'pending.db',outcome:'pending'},
    ];
    const html=render(items,'partial');
    expect(html).toContain('部分记录已导入');expect(html).toContain('已导入 2 │ 未适配 1 │ 超预算 1 │ 失败 1');expect(html).toContain('已导入（已有记录）');
    expect(html).toContain('未适配 / unknown_schema');expect(html).toContain('超预算 / 暂缓处理');expect(html).toContain('失败 / read_failed');expect(html).toContain('首验通过 / 待导入');
  });
  it('缺密钥明确暂缓且不计入导入成功',()=>{
    const html=render([{...run.items[0],key_ready:false,outcome:'key_deferred',skipped:true,error:'KEY_NOT_FOUND'}],'partial');
    expect(html).toContain('缺密钥 1');expect(html).toContain('未取得此库密钥 / 暂缓处理 / KEY_NOT_FOUND');expect(html).toContain('尚无记录导入');expect(html).not.toContain('查看记录');
  });
  it('完整导入仍兼容旧run，明确不会自动启用API',()=>{
    const html=render([{...run.items[0],source:importedSource}],'complete');
    expect(html).toContain('离线记录已导入');expect(html).toContain('2 条消息');expect(html).toContain('查看记录');expect(html).toContain('API 不会自动启动');
  });
  it('原生working即使本地busy为false也呈现工作中且禁止reset',()=>{
    const html=render(run.items,'waiting',true);
    expect(html).toContain('正在结束当前步骤');expect(html).toContain('aria-busy="true"');expect(html).toContain('正在停止');expect(html).not.toContain('重置引导');expect(html).not.toContain('引导已暂停');
    expect(html).toMatch(/disabled=""[^>]*><svg[\s\S]*继续 \/ 重试/);
    expect(render(run.items,'waiting',false,true)).not.toContain('重置引导');
  });
  it('outcome声称导入但缺source时不报成功',()=>{
    const html=render([{...run.items[0],outcome:'imported'}],'complete');
    expect(html).toContain('尚无记录导入');expect(html).toContain('未确认导入结果');expect(html).not.toContain('查看记录');
  });
});
