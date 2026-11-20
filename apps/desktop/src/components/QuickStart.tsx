import {useState} from 'react';
import {ArrowLeft,ArrowRight,X} from 'lucide-react';
import {HelpTip} from './HelpTip';
export const GUIDE_KEY='werc.quickstart.v1';
export function guideShouldOpen(){try{return typeof window!=='undefined'&&window.localStorage.getItem(GUIDE_KEY)!=='seen';}catch{return true;}}
export function rememberGuide(){try{window.localStorage.setItem(GUIDE_KEY,'seen');}catch{/* Guide remains usable without preference storage. */}}
const steps=[
  {title:'连接你的微信',text:'首次设置会自动准备连接；需要重新设置时，在连接页点击“自动设置微信”。',detail:'先打开并登录电脑微信。WeRC自动登记、绑定、核验身份并准备资料；多个安装或进程时选择一次。无需导入全部聊天历史，也无需设置密码。遇到未支持版本会保留原因。',action:'前往连接',page:'connect'},
  {title:'按需要查看消息',text:'“微信会话”查看已有聊天和资料；“实时新消息”查看连接期间收到的消息。',detail:'电脑微信需要保持登录。点击“打开在线会话”即可自动准备资料。打开聊天后可按日期、文字、发送者和类型查找，选中消息查看上下文。“离线归档”用于已导入快照。问号可通过悬停、键盘或点击打开。',action:'打开会话',page:'history'},
  {title:'连接你的 Bot',text:'打开“Bot 接入”，启动服务并授权收发，再把连接配置粘贴到客户端。',detail:'只需授权一次，无需逐个会话设置转发规则，也无需逐条回桌面批准。Bot提供精确账号，负责自动回复、名单和免打扰。HTTP与MCP使用同一个发送队列，结果未知时自动回查，不自动重发。',action:'连接 Bot 客户端',page:'api'},
  {title:'独立工具按需使用',text:'手动发送在聊天或发送队列操作；浏览聊天时展开“导出”即可保存资料。',detail:'桌面手动发送保留预览与批准，联系人或群从聊天中选择。导出、搜索、日历和资料浏览独立保留，不依赖Bot。次要选项默认收起，问号中可以查看详细说明。',action:'查看发送队列',page:'outbox'},
] as const;
export function QuickStart({navigate,close}:{navigate:(page:'connect'|'history'|'outbox'|'api')=>void;close:()=>void}) {
  const [step,setStep]=useState(0);const current=steps[step];
  return <section className="quick-start" aria-label="首次使用指南"><header><span className="eyebrow">开始使用 · {step+1} / {steps.length}</span><button className="icon-button" aria-label="收起使用指南" onClick={close}><X size={16}/></button></header>
    <h2>{current.title} <HelpTip label={`${current.title}的详细说明`}>{current.detail}</HelpTip></h2><p>{current.text}</p><div className="quick-start-actions"><button className="button primary" onClick={()=>navigate(current.page)}>{current.action}<ArrowRight size={16}/></button>
    <div className="guide-pagination"><button className="icon-button" aria-label="上一条引导" disabled={step===0} onClick={()=>setStep(step-1)}><ArrowLeft size={16}/></button>{step<steps.length-1?<button className="button" onClick={()=>setStep(step+1)}>下一步<ArrowRight size={16}/></button>:<button className="button" onClick={close}>开始使用</button>}</div></div>
    <button className="guide-later" onClick={close}>稍后再看 · 可从右上角重新打开</button></section>;
}
