import {ArrowUpRight, Cable, MessageSquare, Database, AudioLines, Send} from 'lucide-react';
import {HelpTip} from './HelpTip';

export function ConnectionEmpty({workspace,connect}:{workspace:string;connect:()=>void}) {
  const Icon=workspace==='会话'?MessageSquare:workspace==='资料库'?Database:workspace==='音频'?AudioLines:Send;
  return <section className="connection-empty" aria-label={`${workspace}准备连接`}>
    <div className="empty-visual" aria-hidden="true"><Icon size={40} strokeWidth={1.25}/><Cable size={20}/></div>
    <span className="eyebrow">// GET CONNECTED</span>
    <h2>连接微信，开始使用</h2>
    <p>配置本机微信后，即可使用{workspace}。</p>
    <div className="empty-actions"><button className="button primary" onClick={connect}>连接当前微信<ArrowUpRight size={17}/></button><HelpTip label="连接前需要准备什么">保持电脑微信已登录，在连接页登记本机标签并绑定客户端。首次连接无需导入全部历史记录；详细兼容结果会在连接页显示。</HelpTip></div>
    <div className="empty-footer"><span>本机资料</span><span>按需读取</span><span>加密保存</span></div>
  </section>;
}
