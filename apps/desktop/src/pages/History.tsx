import { useState, type FormEvent } from 'react';
import { Database, Download, Search, Trash2 } from 'lucide-react';
import { command, desktopAvailable, type HistoryContact, type HistoryMessage, type HistorySession, type MessagePage } from '../lib/desktop';
import type { Workbench } from '../hooks/useWorkbench';
import { DataSources } from './DataSources';
import { emptyHistoryFilters, historyQuery, type HistoryFilters } from '../lib/history';

export function History(workbench: Workbench) {
  const [accountId, setAccountId] = useState(workbench.data.current_connection.account_id ?? '');
  return <>
    <div className="section-title"><h2>数据来源</h2><span className="state-tag">离线快照 / 非实时收信</span></div>
    <label className="history-account">归档资料范围<select aria-label="记录账号" value={accountId} disabled={workbench.busy || !desktopAvailable} onChange={e => setAccountId(e.target.value)}><option value="">选择归档范围</option>{workbench.data.accounts.map(a => <option key={a.id} value={a.id}>{a.label}</option>)}</select></label>
    {accountId ? <AccountHistory key={accountId} accountId={accountId} {...workbench}/> : <p className="empty-inline">选择归档范围后导入和查询记录。</p>}
  </>;
}
function AccountHistory({ accountId, data, busy, act }: Workbench & { accountId: string }) {
  const [path, setPath] = useState('');
  const [key, setKey] = useState('');
  const [plain, setPlain] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [filters, setFilters] = useState<HistoryFilters>(emptyHistoryFilters);
  const [cursors, setCursors] = useState<(string | null)[]>([null]);
  const [pageIndex, setPageIndex] = useState(0);
  const [page, setPage] = useState<MessagePage | null>(null);
  const [contacts, setContacts] = useState<HistoryContact[]>([]);
  const [sessions, setSessions] = useState<HistorySession[]>([]);
  const [tab, setTab] = useState<'messages' | 'contacts' | 'sessions'>('messages');
  const [format, setFormat] = useState('json');
  const [destination, setDestination] = useState('');
  const [exported, setExported] = useState<number | null>(null);
  const [removing, setRemoving] = useState('');
  const canImport=accountId===data.current_connection.account_id;
  const sources = data.history_sources.filter(s => s.account_id === accountId);
  const query = (cursor: string | null = null) => historyQuery(filters, cursor);
  function changeFilters(value: Partial<HistoryFilters>) { setFilters(previous => ({ ...previous, ...value })); setPage(null); setCursors([null]); setPageIndex(0); setExported(null); }
  async function load(cursor: string | null = null, index = 0) {
    await act(async () => { setPage(await command<MessagePage>('history_messages', { accountId, query: query(cursor) })); setPageIndex(index); setCursors(previous => [...previous.slice(0, index), cursor]); });
  }
  async function importFile(event: FormEvent) {
    event.preventDefault();
    const secret = key || null;
    setKey('');
    await act(async () => {
      await command('import_history', { accountId, path, keyHex: secret, allowPlaintext: plain, confirmed });
      setConfirmed(false);
      setPage(await command<MessagePage>('history_messages', { accountId, query: query() }));
    });
  }
  async function chooseTab(next: typeof tab) {
    setTab(next);
    if (next === 'contacts') await act(async () => { setContacts(await command<HistoryContact[]>('history_contacts', { accountId })); });
    if (next === 'sessions') await act(async () => { setSessions(await command<HistorySession[]>('history_sessions', { accountId })); });
  }
  return <>
    {canImport&&<>
    <DataSources busy={busy} act={act} onChoose={value => { setPath(value); setKey(''); setConfirmed(false); }}/>
    <form className="history-import" onSubmit={importFile}>
      <label>离线数据库<input aria-label="离线数据库路径" value={path} disabled={busy} onChange={e => setPath(e.target.value)} placeholder="数据库完整路径；存在 WAL 时须保留同目录同名文件"/></label>
      <label>本次密钥<input aria-label="本次数据库密钥" type="password" value={key} autoComplete="off" maxLength={64} disabled={busy || plain} onChange={e => setKey(e.target.value)} placeholder="64位十六进制；留空使用已保护密钥"/></label>
      <label className="permission-choice"><input type="checkbox" checked={plain} disabled={busy} onChange={e => { setPlain(e.target.checked); setKey(''); }}/>此来源是明文数据库</label>
      <label className="permission-choice"><input aria-label="确认读取离线来源" type="checkbox" checked={confirmed} disabled={busy} onChange={e => setConfirmed(e.target.checked)}/>我有权读取此离线来源</label>
      <button className="button primary" disabled={busy || !confirmed || !path.trim() || (key.length > 0 && !/^[a-f\d]{64}$/i.test(key))}><Database size={16}/>导入记录</button>
    </form>
    </>}
    {!canImport&&<p className="field">旧资料仅供浏览和导出，不能继续读取来源。</p>}
    <details className="permission-details"><summary>读取范围</summary><p>仅显式读取所选 DB 及相邻 WAL。写入者未退出、页面认证失败或结构未知时拒绝；不结束微信、不修改来源。导入记录加密保存，但尚无真实微信版本验收。</p></details>
    {sources.length > 0 && <ul className="history-sources">{sources.map(s => <li key={s.id}><div><strong>{s.message_count} 条消息 / {s.contact_count} 位联系人 / {s.session_count} 个会话</strong><code>{s.source_sha256.slice(0, 16)} / {s.encryption}</code><span className="field">{new Date(s.imported_at).toLocaleString('zh-CN')}</span></div>{removing === s.id ? <div className="history-actions"><span>仅清除导入副本</span><button className="button danger" disabled={busy} onClick={() => act(async () => { await command('remove_history', { accountId, id: s.id }); setRemoving(''); setPage(null); setContacts([]); setSessions([]); })}>确认移除</button><button className="button" disabled={busy} onClick={() => setRemoving('')}>取消</button></div> : <button className="button" disabled={busy} onClick={() => setRemoving(s.id)}><Trash2 size={15}/>移除来源</button>}</li>)}</ul>}
    <div className="section-title"><h2>已导入记录</h2><span className="field">{sources.length} 份来源</span></div>
    <div className="history-tabs" role="group" aria-label="记录类别">{(['messages', 'contacts', 'sessions'] as const).map(id => <button key={id} className={`button ${tab === id ? 'primary' : ''}`} aria-pressed={tab === id} disabled={busy || !sources.length} onClick={() => chooseTab(id)}>{{ messages: '消息', contacts: '联系人', sessions: '会话' }[id]}</button>)}</div>
    {tab === 'messages' && <>
      <form className="history-query" onSubmit={e => { e.preventDefault(); load(); }}>
        <label>会话<input aria-label="记录会话筛选" value={filters.conversation} disabled={busy} onChange={e => changeFilters({ conversation: e.target.value })}/></label>
        <label>关键词<input aria-label="记录关键词" value={filters.search} maxLength={200} disabled={busy} onChange={e => changeFilters({ search: e.target.value })}/></label>
        <label>发送者<input aria-label="记录发送者" value={filters.sender} maxLength={512} disabled={busy} onChange={e => changeFilters({ sender: e.target.value })}/></label>
        <label>消息类型<input aria-label="记录消息类型" inputMode="numeric" value={filters.type} disabled={busy} onChange={e => changeFilters({ type: e.target.value })} placeholder="类型数字；留空不限"/></label>
        <label>开始时间（本地）<input aria-label="记录开始时间" type="datetime-local" value={filters.from} disabled={busy} onChange={e => changeFilters({ from: e.target.value })}/></label>
        <label>结束时间（不含）<input aria-label="记录结束时间" type="datetime-local" value={filters.until} disabled={busy} onChange={e => changeFilters({ until: e.target.value })}/></label>
        <button className="button" disabled={busy || !sources.length}><Search size={16}/>查询记录</button>
        <button type="button" className="button" disabled={busy} onClick={() => changeFilters(emptyHistoryFilters)}>清除筛选</button>
      </form>
      {page?.data.length ? <ol className="history-messages">{page.data.map((message: HistoryMessage) => <li key={message.id}><div className="field">{new Date(message.timestamp * 1000).toLocaleString('zh-CN')} / 类型 {message.message_type}</div><strong>{message.conversation_id}</strong><code>{message.sender_id ?? '身份未解析'} / {message.server_id}</code><pre>{message.text ?? '[内容未解码或缺失]'}</pre></li>)}</ol> : <p className="empty-inline">{page ? '当前条件下没有记录。' : '导入来源后查询消息，不会自动扫描微信。'}</p>}
      <div className="history-actions">{pageIndex > 0 && <button className="button" disabled={busy} onClick={() => load(cursors[pageIndex - 1], pageIndex - 1)}>上一页</button>}{page && <span className="field">第 {pageIndex + 1} 页</span>}{page?.next_cursor && <button className="button" disabled={busy} onClick={() => load(page.next_cursor, pageIndex + 1)}>下一页</button>}</div>
      <div className="section-title"><h2>导出</h2><span className="field">按当前筛选 / 不覆盖已有文件</span></div>
      <form className="history-export" onSubmit={e => { e.preventDefault(); setExported(null); act(async () => { setExported(await command<number>('save_history_export', { accountId, query: query(), format, path: destination })); }); }}><label>格式<select value={format} disabled={busy} onChange={e => { setFormat(e.target.value); setExported(null); }}><option value="json">JSON</option><option value="csv">CSV</option><option value="txt">TXT</option><option value="html">HTML</option><option value="md">Markdown</option><option value="jsonl">JSONL</option><option value="sql">SQL / 独立离线表</option></select></label><label>目标文件（相对系统下载目录）<input aria-label="记录导出路径" value={destination} disabled={busy} onChange={e => { setDestination(e.target.value); setExported(null); }} placeholder={`下载目录内文件名，扩展名 .${format}`}/></label><button className="button" disabled={busy || !sources.length || !destination.trim()}><Download size={16}/>导出记录</button></form>
      {exported !== null && <p role="status" className="field">已写入 {exported.toLocaleString()} 字节</p>}
    </>}
    {tab === 'contacts' && (contacts.length ? <ul className="history-messages">{contacts.map(c => <li key={c.username}><strong>{c.remark || c.nickname || c.username}</strong><code>{c.username}</code>{c.alias && <span>{c.alias}</span>}</li>)}</ul> : <p className="empty-inline">导入来源中没有联系人资料。</p>)}
    {tab === 'sessions' && (sessions.length ? <ul className="history-messages">{sessions.map(s => <li key={s.username}><strong>{s.username}</strong><p>{s.summary}</p><span className="field">{new Date(s.timestamp * 1000).toLocaleString('zh-CN')} / 未读 {s.unread_count ?? '未知'}</span></li>)}</ul> : <p className="empty-inline">导入来源中没有会话资料。</p>)}
  </>;
}
