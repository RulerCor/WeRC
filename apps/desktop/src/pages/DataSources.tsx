import { useState } from 'react';
import { FolderSearch } from 'lucide-react';
import { command, type DataSourceCatalog } from '../lib/desktop';
import type { Workbench } from '../hooks/useWorkbench';

export function DataSources({ busy, act, onChoose }: Pick<Workbench, 'busy' | 'act'> & { onChoose: (path: string) => void }) {
  const [root, setRoot] = useState('');
  const [consent, setConsent] = useState(false);
  const [catalog, setCatalog] = useState<DataSourceCatalog | null>(null);
  return <details className="permission-details">
    <summary>查找数据库位置</summary>
    <form className="data-source-detect" onSubmit={e => { e.preventDefault(); act(async () => { setCatalog(await command<DataSourceCatalog>('discover_data_sources', { root: root.trim() || null, confirmed: consent })); }); }}>
      <label>来源目录<input aria-label="检测来源目录" value={root} disabled={busy} onChange={e => { setRoot(e.target.value); setCatalog(null); setConsent(false); }} placeholder="留空检测系统文档目录中的微信数据位置"/></label>
      <label className="permission-choice"><input aria-label="确认检测数据位置" type="checkbox" checked={consent} disabled={busy} onChange={e => setConsent(e.target.checked)}/>仅检测目录和文件元数据</label>
      <button className="button" disabled={busy || !consent}><FolderSearch size={16}/>检测位置</button>
    </form>
    {catalog && <>
      <p className="field">{catalog.sources.length} 个来源 / {catalog.roots_checked} 个根目录 / 身份未验证</p>
      {catalog.sources.length === 0 && <p className="empty-inline">没有检测到支持的数据库布局，可手动指定其他目录。</p>}
      {catalog.sources.map(source => <section key={source.id} className="history-source-candidate">
        <h3>{source.directory_name}</h3><code>{source.account_directory}</code>
        <ul className="history-sources">{source.databases.map(db => <li key={db.relative_path}><div><code>{db.relative_path}</code><span className="field">{db.size.toLocaleString()} 字节{db.wal_present ? ' / 含 WAL' : ''}</span></div><button type="button" className="button" disabled={busy} onClick={() => onChoose(`${source.account_directory.replace(/[\\/]$/, '')}/${db.relative_path}`)}>选择数据库</button></li>)}</ul>
      </section>)}
      {catalog.warnings.length > 0 && <p role="status">部分目录未完成检测：{[...new Set(catalog.warnings)].join(' / ')}</p>}
    </>}
  </details>;
}
