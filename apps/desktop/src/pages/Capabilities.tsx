import { useState } from 'react';
import { Layers3 } from 'lucide-react';
import type { Workbench } from '../hooks/useWorkbench';

const evidenceLabels: Record<string, string> = {
  not_implemented: '未实现', implemented: '已实现', fixture_verified: '合成测试通过', real_verified: '真机验证通过',
};
const capabilityLabels: Record<string, string> = {
  'wechat.discover':'微信检测','wechat.data_sources':'数据来源发现',
  'keys.acquire':'自动取钥','keys.import':'导入密钥','keys.protect':'密钥保护',
  'messages.read':'读取微信消息','messages.observe':'实时接收',
  'messages.send_text':'发送文字','messages.send_image':'发送图片',
  'messages.send_video':'发送视频','messages.send_emoticon':'发送表情',
  'messages.send_file':'发送文件','messages.send_media':'发送媒体','messages.send_voice':'发送语音',
  'media.read':'读取媒体','media.audio_preview':'语音播放','moments.read':'朋友圈浏览',
  'moments.publish':'朋友圈发布与互动','exports.create':'资料导出',
  'history.import':'导入快照','history.read':'浏览快照','history.export':'导出快照',
};
const availabilityLabels: Record<string, string> = {
  ready: '就绪', needs_setup: '待配置', unsupported: '不支持', degraded: '受限', blocked: '受阻',
};

export function Capabilities({ data }: Workbench) {
  const [search, setSearch] = useState('');
  const entries = data.capabilities.filter(c => `${c.id} ${capabilityLabels[c.id] ?? ''} ${c.reason}`.toLowerCase().includes(search.toLowerCase()));
  return <>
    <div className="capability-toolbar">
      <span className="field">{data.capabilities.length} 项能力</span>
      <input type="search" aria-label="搜索能力" placeholder="搜索能力" value={search} onChange={e => setSearch(e.target.value)}/>
    </div>
    {data.capabilities.length ? <div className="capability-table" role="table" aria-label="能力状态">
      <div className="capability-row table-heading" role="row"><span role="columnheader">能力</span><span role="columnheader">当前状态</span><span role="columnheader">详情</span></div>
      {entries.map(c => <div key={c.id} className="capability-row" role="row">
        <span role="cell">{capabilityLabels[c.id] ?? c.id}</span>
        <div className="capability-status" role="cell"><span className={`state-tag ${c.availability === 'ready' ? 'ready' : ''}`}>{availabilityLabels[c.availability] ?? c.availability}</span></div>
        <div role="cell" className="capability-detail"><details><summary>原因</summary><p>{c.reason}</p><small>{evidenceLabels[c.evidence] ?? c.evidence} · <code>{c.id}</code></small></details></div>
      </div>)}
      {entries.length === 0 && <p className="empty-inline" role="status">无匹配结果</p>}
    </div> : <div className="empty-state"><Layers3 size={32} strokeWidth={1.3}/><h3>暂无能力数据</h3><p>请在桌面应用中查看。</p></div>}
  </>;
}
