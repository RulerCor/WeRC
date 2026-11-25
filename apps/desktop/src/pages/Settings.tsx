import {StorageManager} from './StorageManager';
import {Diagnostics} from './Diagnostics';
import {AppLockSettings} from '../components/DesktopLock';
import { useEffect, useState } from 'react';
import type { Workbench } from '../hooks/useWorkbench';
import { command, desktopAvailable } from '../lib/desktop';
import type { MotionMode } from '../lib/motion';
import {DesktopPreferences} from './DesktopPreferences';
import type {Theme} from '../lib/theme';
import {HelpTip} from '../components/HelpTip';
import {UpdateSettings} from './UpdateSettings';

export function Settings({ data, motion='full',setMotion,startSetup,theme='light',setTheme }: Workbench & {theme?:Theme;setTheme?:(value:Theme)=>void;motion?:MotionMode;setMotion?:(value:MotionMode)=>void;startSetup?:()=>void}) {
  const [location,setLocation]=useState('');
  const [section,setSection]=useState<'appearance'|'desktop'|'storage'|'privacy'|'diagnostics'>('appearance');
  useEffect(()=>{if(desktopAvailable) void command<{executable:string}>('runtime_location').then(value=>setLocation(value.executable)).catch(()=>setLocation('无法读取程序位置'));},[]);
  return <div className="settings-workspace"><nav className="settings-workspace-nav" aria-label="设置分类">{([['appearance','外观'],['desktop','本机运行'],['storage','存储'],['privacy','隐私与锁'],['diagnostics','诊断']] as const).map(([id,label])=><button key={id} className={`button ${section===id?'primary':''}`} aria-pressed={section===id} onClick={()=>setSection(id)}>{label}</button>)}</nav>
    {section==='desktop'&&<DesktopPreferences/>}{section==='privacy'&&<section className="panel"><div className="section-title"><h2>本机隐私与应用锁</h2></div><AppLockSettings/></section>}{section==='storage'&&<StorageManager/>}{section==='diagnostics'&&<Diagnostics/>}
    {section==='appearance'&&<dl className="settings-list">
    <div><dt>应用</dt><dd>WeRC</dd></div>
    <div><dt>版本</dt><dd className="field">v{data.status.version}</dd></div>
    <div><dt>平台</dt><dd>Windows</dd></div>
    <div><dt>主题</dt><dd><select aria-label="主题" value={theme} onChange={e=>setTheme?.(e.target.value as Theme)}><option value="light">浅色</option><option value="dark">深色</option><option value="system">跟随系统</option></select></dd></div>
    <div><dt>动画效果 <HelpTip label="动画设置说明">系统减少动态效果设置优先，键盘操作即时响应。</HelpTip></dt><dd><select aria-label="动画效果" value={motion} onChange={e=>setMotion?.(e.target.value as MotionMode)}><option value="full">完整</option><option value="calm">简洁</option><option value="off">关闭</option></select></dd></div>
    <div><dt>历史归档</dt><dd><button className="button" disabled={!desktopAvailable} onClick={startSetup}>导入快照 / 查看进度</button></dd></div>
    <div><dt>微信接入</dt><dd>连接与支持状态见“连接”和“能力”页面</dd></div>
    <div><dt>程序信息</dt><dd><details><summary>查看程序位置与使用说明</summary><dl><div><dt>当前程序</dt><dd><code>{location||'仅桌面应用可查看'}</code></dd></div>
    <div><dt>使用说明</dt><dd><code>usage.md</code>（程序所在目录）</dd></div></dl></details></dd></div>
  </dl>}{section==='appearance'&&<UpdateSettings/>}</div>;
}
