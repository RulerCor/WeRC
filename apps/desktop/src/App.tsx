import {StartupGate} from './components/StartupGate';
import {DesktopNotification,type NotificationNavigation} from './components/DesktopNotification';
import {DesktopLockGate} from './components/DesktopLock';
import { useEffect, useState } from 'react';
import {QuickStart,guideShouldOpen,rememberGuide} from './components/QuickStart';
import { useListMotion } from './hooks/useListMotion';
import { useOnboarding } from './hooks/useOnboarding';
import { Onboarding } from './pages/Onboarding';
import { useMotionPolicy, revealTheme } from './lib/motion';
import { AudioLines, BookOpen, Cable, ChevronRight, Code2, Layers3, Moon, RefreshCw, Settings2, Sun, X, Wrench, CircleHelp } from 'lucide-react';
import { command,desktopAvailable } from './lib/desktop';
import { useWorkbench } from './hooks/useWorkbench';
import { useFeedbackPresence } from './hooks/useFeedbackMotion';
import { applyTheme, readTheme, followSystem } from './lib/theme';
import { Connections } from './pages/Connections';
import { Audio } from './pages/Audio';
import { Chat as History } from './pages/Chat';
import { ApiSettings } from './pages/ApiSettings';
import { Capabilities } from './pages/Capabilities';
import { Outbox } from './pages/Outbox';
import { Settings } from './pages/Settings';
import { Library } from './pages/Library';
import {ClosePrompt} from './components/ClosePrompt';
import appIcon from '../src-tauri/icons/128x128.png';
import { ConnectionEmpty } from './components/ConnectionEmpty';
import { ModelManager } from './pages/ModelManager';
import { FirstRun } from './pages/FirstRun';
import { readFirstRun, saveFirstRun } from './lib/first-run';
import { useConnectionResume, useNativeConnectionStatus } from './hooks/useConnectionResume';

const navigation = [
  { id: 'connect', label: '连接', english: 'CONNECTION', icon: Cable },
  { id: 'history', label: '会话', english: 'CONVERSATIONS', icon: BookOpen },
  { id: 'outbox', label: '发送队列', english: 'OUTBOX', icon: Layers3 },
  { id: 'audio', label: '音频', english: 'AUDIO', icon: AudioLines },
  { id: 'library', label: '资料库', english: 'LIBRARY', icon: BookOpen },
  { id: 'api', label: 'Bot 接入', english: 'HTTP / MCP', icon: Code2 },
  { id: 'capabilities', label: '能力', english: 'CAPABILITIES', icon: Layers3 },
  { id: 'settings', label: '设置', english: 'SETTINGS', icon: Settings2 },
] as const;
type Page = typeof navigation[number]['id'];

export default function App() {if(typeof window!=='undefined'&&new URLSearchParams(window.location.search).get('notification')==='1')return <DesktopNotification/>;return <StartupGate><DesktopLockGate><WorkbenchApp/></DesktopLockGate></StartupGate>;}
function WorkbenchApp() {
  // Primary: active workspace; secondary: connection/API state; tertiary: help/version.
  // One primary action per empty workspace: connect. Existing paper/ink/signal tokens,
  // 8/12/16/24/32 spacing and 180/280ms mechanical motion supply the shared system.
  const workbench = useWorkbench();
  const onboarding = useOnboarding(workbench);
  const motion = useMotionPolicy();
  useListMotion();
  const { data, busy, error, setError, act } = workbench;
  const notice = useFeedbackPresence(error || null);
  const [page, setPage] = useState<Page>('history');
  const [firstRun,setFirstRun]=useState(()=>desktopAvailable&&readFirstRun()!=='complete');
  const nativeStatus=useNativeConnectionStatus(data.current_connection.account_id,workbench.loaded&&!firstRun);
  const resumeNotice=useConnectionResume(workbench.loaded&&!firstRun&&readFirstRun()==='complete',data,nativeStatus,workbench.refresh);
  const [notificationTarget,setNotificationTarget]=useState<NotificationNavigation|null>(null);
  useEffect(()=>{if(!desktopAvailable)return;let alive=true,pending=false;const read=async()=>{if(pending)return;pending=true;try{const n=await command<NotificationNavigation|null>('take_notification_navigation');if(alive&&n){setNotificationTarget(n);setPage(n.page==='audio'?'audio':n.page==='library'?'library':'history');if(n.page!=='history')setToolsOpen(true);}}catch{/* Locked and closing windows leave navigation pending. */}finally{pending=false;}};void read();const timer=setInterval(()=>void read(),1000);return()=>{alive=false;clearInterval(timer);};},[]);
  const [toolsOpen,setToolsOpen]=useState(false),[guideOpen,setGuideOpen]=useState(guideShouldOpen);
  function closeGuide(){rememberGuide();setGuideOpen(false);}
  const [theme, setTheme] = useState(readTheme);
  useEffect(() => { applyTheme(theme); return followSystem(theme); }, [theme]);
  function go(next: Page) { if(onboarding.visible) void onboarding.close(); setPage(next); if(['library','audio','capabilities'].includes(next))setToolsOpen(true); setError(''); }
  const current = navigation.find(item => item.id === page)!;
  const account = data.accounts.find(item => item.id === data.current_connection.account_id);
  const needsConnection = data.accounts.length === 0 && ['history','outbox','audio','library'].includes(page);

  function startFirstRun(){saveFirstRun('wechat');setFirstRun(true);setGuideOpen(false);}
  if(firstRun)return <><ClosePrompt/>{workbench.loaded?<FirstRun workbench={workbench} finish={(target,complete)=>{rememberGuide();setGuideOpen(false);setFirstRun(false);setPage(target);if(complete){try{localStorage.setItem('werc.chat.mode','live_window');}catch{/* Optional preference. */}}}}/>:<div className="first-run"><main><p role="status">正在准备首次设置…</p>{error&&<p role="alert">{error}</p>}</main></div>}</>;

  return <div className="ef app-shell" data-workspace={page}>
    <ClosePrompt/><div className="ef-tri" aria-hidden="true"><i/><i/><i/></div>
    <aside className="sidebar">
      <a href="#" className="brand" onClick={e => { e.preventDefault(); go('connect'); }} aria-label="WeRC 首页">
        <img className="brand-mark" src={appIcon} alt="" width={44} height={44}/><span><strong>WeRC</strong><small>WORKSPACE</small></span>
      </a>
      <div className="nav-caption">WORKSPACE / 工作台</div>
      <nav aria-label="主导航">
        {(['history','api','outbox','connect'] as const).map(id=>{const item=navigation.find(n=>n.id===id)!;const Icon=item.icon;return <button key={id} className={`nav-item ${page===id?'active':''}`} onClick={()=>go(id)} aria-label={item.label} aria-current={page===id?'page':undefined}><Icon size={18}/><span>{item.label}<small>{item.english}</small></span>{page===id&&<ChevronRight size={15}/>}</button>;})}
        <details className="nav-tools" open={toolsOpen} onToggle={e=>setToolsOpen(e.currentTarget.open)}><summary aria-label="工具与诊断"><Wrench size={18}/><span>工具与诊断</span><ChevronRight size={14}/></summary><div className="nav-tool-items">
          {navigation.filter(item=>['library','audio','capabilities'].includes(item.id)).map(({id,label,english,icon:Icon})=><button key={id} className={`nav-item ${page===id?'active':''}`} onClick={()=>go(id)} aria-label={label} aria-current={page===id?'page':undefined}><Icon size={18}/><span>{label}<small>{english}</small></span></button>)}
        </div></details>
        <button className={`nav-item ${page==='settings'?'active':''}`} onClick={()=>go('settings')} aria-label="设置" aria-current={page==='settings'?'page':undefined}><Settings2 size={18}/><span>设置<small>SETTINGS</small></span></button>
      </nav>
      <div className="sidebar-foot">
        <span className="version">v{data.status.version}</span>
        <button className="icon-button" aria-label={theme === 'dark' ? '切换浅色主题' : '切换深色主题'} onClick={e => { const next=theme==='dark'?'light':'dark'; revealTheme(e.currentTarget,()=>{applyTheme(next);setTheme(next);}); }}>{theme === 'dark' ? <Sun size={17}/> : <Moon size={17}/>}</button>
      </div>
    </aside>
    <div className="main-shell">
      <header className="topbar"><span className="command-label">WeRC <span>/</span> {current.label}</span><div className="workspace-status"><span className="connection-indicator" data-ready={Boolean(account&&nativeStatus?.connected&&nativeStatus.account_id===account.id)} aria-hidden="true"/><span>{account?.label || '微信待连接'}</span><span className="api-indicator">API {data.status.api_enabled?'已启动':'未启动'}</span></div><button className="guide-entry" onClick={()=>setGuideOpen(v=>!v)} aria-expanded={guideOpen}><CircleHelp size={16}/><span>使用指南</span></button><span className="runtime-label">{desktopAvailable ? '本机' : '界面预览'}</span></header>
      <main id="workspace-main">
        <div className="page-heading">
          <div><p className="eyebrow">// {current.english}</p><h1 tabIndex={-1}>{current.label}</h1></div>
          <span className="page-index" aria-hidden="true">{String(navigation.findIndex(item=>item.id===page)+1).padStart(2,'0')}</span>
          <button className="button refresh-button" aria-label="刷新状态" disabled={busy || !desktopAvailable} aria-busy={busy} onClick={() => act()}><RefreshCw size={16}/><span>刷新</span></button>
        </div>
        {guideOpen&&<QuickStart navigate={go} close={closeGuide}/>}
        {resumeNotice&&<div className="error-notice" role="status"><span>{resumeNotice}</span><button className="button" onClick={()=>go('connect')}>查看连接</button></div>}
        {notice.value && <div className="error-notice feedback-surface" data-motion={notice.animated} data-exiting={notice.exiting} inert={notice.exiting} aria-hidden={notice.exiting || undefined} role={notice.exiting ? undefined : 'alert'} onTransitionEnd={event => { if (event.target === event.currentTarget && event.propertyName === 'opacity' && notice.exiting) notice.finish(); }}><span>{notice.value}</span><button className="icon-button" aria-label="关闭错误" onClick={() => setError('')}><X size={16}/></button></div>}
        {workbench.completed>0&&<div key={workbench.completed} className="action-complete" aria-hidden="true"/>}
        {onboarding.visible ? <Onboarding flow={onboarding} navigate={go}/> : <div className="page-content" key={page}>
          {page === 'connect' && <><button className="button" disabled={!desktopAvailable||busy} onClick={startFirstRun}>自动设置微信<ChevronRight size={16}/></button><details className="snapshot-entry"><summary>历史快照归档</summary><button className="button setup-entry" disabled={!desktopAvailable||busy||!onboarding.status} onClick={onboarding.open}>{onboarding.status?.run&&!['complete','partial'].includes(onboarding.status.run.phase)?'快照未完成 / 继续导入':'导入快照 / 查看进度'}<ChevronRight size={16}/></button></details><Connections {...workbench}/></>}
          {needsConnection && <ConnectionEmpty workspace={current.label} connect={()=>go('connect')}/>}
          {page === 'audio' && needsConnection && <ModelManager/>}
          {page === 'history' && !needsConnection && <History {...workbench} notificationTarget={notificationTarget} onConnect={()=>go('connect')}/>}
          {page === 'library' && !needsConnection && <Library {...workbench}/>}
          {page === 'outbox' && !needsConnection && <Outbox {...workbench}/>}
          {page === 'audio' && !needsConnection && <Audio {...workbench}/>}
          {page === 'api' && <ApiSettings {...workbench}/>}
          {page === 'capabilities' && <Capabilities {...workbench}/>}
          {page === 'settings' && <><button className="button" disabled={!desktopAvailable||busy} onClick={startFirstRun}>重新运行首次设置<ChevronRight size={16}/></button><Settings {...workbench} theme={theme} setTheme={setTheme} motion={motion.mode} setMotion={motion.setMode} startSetup={onboarding.open}/></>}
        </div>}
      </main>
    </div>
  </div>;
}
