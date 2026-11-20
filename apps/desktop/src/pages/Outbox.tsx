import {momentActionLabels} from '../lib/outbox';
import {localMinute,scheduledTimestamp} from '../lib/send-schedule';
import {MomentVideoPreview} from './MomentVideoPreview';
import {MomentComposer} from './MomentComposer';
import {MomentImagePreview} from './MomentImagePreview';
import {validSendText,textByteLength,MAX_TEXT_BYTES} from '../lib/send-text';
import { useCallback, useEffect, useRef, useState, type FormEvent } from 'react';
import { command, desktopAvailable } from '../lib/desktop';
import type { Workbench } from '../hooks/useWorkbench';
import { AudioPreview } from './AudioPreview';
import { ImagePreview } from './ImagePreview';
import { ImageComposer } from './ImageComposer';
import {VideoComposer} from './VideoComposer';
import {EmoticonComposer} from './EmoticonComposer';
import {EmoticonPreview} from './EmoticonPreview';
import {VideoPreview} from './VideoPreview';
import {HelpTip} from '../components/HelpTip';
import { FileComposer } from './FileComposer';
import './outbox.css';
import {RecipientPicker} from '../components/RecipientPicker';

export {sendStates} from '../lib/outbox';
export type {SendState,SendStatus,SendPreview,EmoticonSendAsset,VideoSendAsset} from '../lib/outbox';
import {sendStates,type SendStatus,type SendPreview} from '../lib/outbox';

const modes = { native: '后台原生', visible_automation: '可见自动化 / 需要前台', synthetic: '自有模拟宿主 / 不发微信' };
const message = (error: unknown) => error instanceof Error ? error.message : String(error);
export function Outbox(props: Workbench) {
  const accountId = props.data.current_connection.account_id;
  return accountId ? <Queue key={`${accountId}/${props.data.current_connection.revision}`} accountId={accountId} {...props}/> : <p className="empty-inline">配置当前连接后，在这里预览和批准发送任务。</p>;
}
function Queue({ accountId, data, setError, refresh: refreshWorkbench }: Workbench & { accountId: string }) {
  const [items, setItems] = useState<SendStatus[]>([]);
  const [historyCursor,setHistoryCursor]=useState<number|null>(null),[historyDone,setHistoryDone]=useState(false);
  const mergeTasks=(previous:SendStatus[],next:SendStatus[])=>Array.from(new Map([...previous,...next].map(item=>[item.metadata.id,item])).values()).sort((a,b)=>b.updated_at-a.updated_at||b.metadata.id.localeCompare(a.metadata.id));
  const [workspace,setWorkspace]=useState<'compose'|'tasks'|'moments'>('compose');
  const previewHeading=useRef<HTMLHeadingElement>(null);
  const [preview, setPreview] = useState<SendPreview | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  useEffect(()=>{if(preview){previewHeading.current?.focus({preventScroll:true});previewHeading.current?.scrollIntoView({block:"nearest"});}},[preview?.status.metadata.id]);
  const [scheduleEnabled,setScheduleEnabled]=useState(false),[scheduleTime,setScheduleTime]=useState('');
  useEffect(()=>{setScheduleEnabled(false);setScheduleTime('');},[preview?.status.metadata.id]);
  const [imageReady,setImageReady]=useState(false);
  const [busy, setBusy] = useState(false);
  const [target, setTarget] = useState('filehelper');
  const [targetReady,setTargetReady]=useState(false);
  const [text, setText] = useState('');
  const [assetId,setAssetId]=useState('');
  const [draftKey, setDraftKey] = useState(() => crypto.randomUUID());
  const alive = useRef(true);
  const sequence = useRef(0);
  const actionRunning = useRef(false);
  const pendingRefresh = useRef(false);
  const canMoment=desktopAvailable&&data.capabilities.some(c=>c.id==='moments.publish'&&['ready','degraded'].includes(c.availability));
  const canSend = desktopAvailable && data.capabilities.some(c => c.id === 'messages.send_text' && c.availability === 'ready');
  const canEmoticon = desktopAvailable && data.capabilities.some(c=>c.id === "messages.send_emoticon" && c.availability === "ready");
  const canVideo = desktopAvailable && data.capabilities.some(c=>c.id === 'messages.send_video' && c.availability === 'ready');
  const canFile = desktopAvailable && data.capabilities.some(c=>c.id === 'messages.send_file' && c.availability === 'ready');
  const canImage = desktopAvailable && data.capabilities.some(c=>c.id === 'messages.send_image' && c.availability === 'ready');
  const canVoice = desktopAvailable && data.capabilities.some(c => c.id === 'messages.send_voice' && c.availability === 'ready');
  const voiceAssets=data.audio_assets.filter(a=>a.account_id===accountId && a.audio.sample_rate===16000 && a.audio.channels===1 && a.audio.duration_ms<=60000 && a.audio.frames%320===0);
  const refresh = useCallback(async () => {
    if (!desktopAvailable || pendingRefresh.current) return;
    pendingRefresh.current = true;
    try { const next = await command<SendStatus[]>('list_sends', { accountId }); if (alive.current) {setItems(previous=>mergeTasks(previous,next));setPreview(previous=>{if(!previous)return previous;const status=next.find(row=>row.metadata.id===previous.status.metadata.id);return status?{...previous,status}:previous;});} }
    finally { pendingRefresh.current = false; }
  }, [accountId]);
  useEffect(() => {
    alive.current = true;
    void refresh().catch(e => { if (alive.current) setError(message(e)); });
    return () => { alive.current = false; sequence.current++; };
  }, [refresh, setError]);
  const watching = items.some(item => item.state === 'queued' || item.state === 'submitting' || (workspace==='tasks'&&['unknown','server_accepted'].includes(item.state)));
  useEffect(() => {
    if (!watching) return;
    const timer = window.setInterval(() => { void refresh().catch(e => { if (alive.current) setError(message(e)); }); }, 2000);
    return () => window.clearInterval(timer);
  }, [watching, refresh, setError]);
  async function act(action: () => Promise<unknown>) {
    if (actionRunning.current || !desktopAvailable) return;
    actionRunning.current = true; setBusy(true); setError('');
    try { await action(); await refresh(); }
    catch (e) { if (alive.current) setError(message(e)); }
    finally { actionRunning.current = false; if (alive.current) setBusy(false); }
  }
  async function older() {
    await act(async()=>{
      const page=await command<{data:SendStatus[];next_cursor:number|null}>('list_send_history',{accountId,before:historyCursor});
      if(!alive.current)return;
      setItems(previous=>mergeTasks(previous,page.data));setHistoryCursor(page.next_cursor);setHistoryDone(page.next_cursor===null);
    });
  }
  function close() { sequence.current++; setPreview(null); setConfirmed(false); setImageReady(false); }
  async function open(id: string) {
    const ticket = ++sequence.current; setPreview(null); setConfirmed(false); setImageReady(false);
    await act(async () => {
      const value = await command<SendPreview>('preview_send', { accountId, outboxId: id });
      if (alive.current && sequence.current === ticket) setPreview(value);
    });
  }
  async function draft(event: FormEvent) {
    event.preventDefault();
    if(!validSendText(text)){setError('文字需非空且在10000个UTF-8字节内。');return;}
    await act(async () => {
      const value = await command<SendStatus>('create_text_draft', { accountId, targetId: target, text, idempotencyKey: draftKey });
      if (!alive.current) return;
      setText(''); setDraftKey(crypto.randomUUID());
      const ticket = ++sequence.current;
      const view = await command<SendPreview>('preview_send', { accountId, outboxId: value.metadata.id });
      if (alive.current && sequence.current === ticket) { setPreview(view); setConfirmed(false); }
    });
  }
  async function approve() {
    if (!preview || !confirmed || ((preview.payload.image || preview.payload.video || preview.payload.emoticon || preview.payload.moment?.images?.length) && !imageReady)) return;
    const current = preview;
    const scheduledAt=scheduleEnabled?scheduledTimestamp(scheduleTime,current.schedule_before??0):null;
    if(scheduleEnabled&&scheduledAt===null){setError('请选择未来且在当前可用期限内的本地时间。');return;}
    close();
    await act(() => command('approve_send', { accountId, outboxId: current.status.metadata.id, requestDigest: current.status.metadata.request_digest, confirmed: true, scheduledAt }));
  }
  async function voiceDraft(event:FormEvent) {
    event.preventDefault();
    await act(async()=>{
      const value=await command<SendStatus>('create_voice_draft',{accountId,targetId:target,assetId,idempotencyKey:draftKey});
      if(!alive.current)return;
      setDraftKey(crypto.randomUUID());setAssetId('');const ticket=++sequence.current;
      const view=await command<SendPreview>('preview_send',{accountId,outboxId:value.metadata.id});
      if(alive.current && sequence.current===ticket){setPreview(view);setConfirmed(false);}
    });
  }
  const selected = preview && items.find(item => item.metadata.id === preview.status.metadata.id);
  const scheduleInvalid=Boolean(scheduleEnabled&&preview&&scheduledTimestamp(scheduleTime,preview.schedule_before??0)===null);
  const expired = preview ? preview.status.metadata.expires_at <= Date.now() / 1000 : false;
  return <section className="outbox-workspace">
    <div className="section-title"><h2>任务 / {items.length}</h2><span className="state-tag">{canSend ? '逐条本机批准' : '真实发送未就绪'}</span></div>
    <div className="outbox-workspace-nav" role="group" aria-label="发送工作区">
      <button className={workspace==='compose'?'button primary':'button'} aria-pressed={workspace==='compose'} disabled={busy} onClick={()=>setWorkspace('compose')}>新建消息</button>
      <button className={workspace==='tasks'?'button primary':'button'} aria-pressed={workspace==='tasks'} disabled={busy} onClick={()=>setWorkspace('tasks')}>任务记录{items.length?` · ${items.length}`:''}</button>
      <button className={workspace==='moments'?'button primary':'button'} aria-pressed={workspace==='moments'} disabled={busy} onClick={()=>setWorkspace('moments')}>朋友圈</button>
      <HelpTip label="发送工作区说明">桌面手动发送先选择收件人和内容，再保存预览。一次授权的Bot可直接排队；旧草稿令牌仍需批准。本地入队不等于送达。任务记录可找回草稿、查看计划和回查结果；结果未知时不会自动重发。更早记录可按需加载，历史不会占用512个尚未处理请求的额度。</HelpTip>
    </div>
    <div hidden={workspace!=='moments'}><MomentComposer key={accountId} accountId={accountId} enabled={canMoment} onDraft={open}/></div>
    <div className="outbox-compose-panel" hidden={workspace!=='compose'}>
    <RecipientPicker accountId={accountId} enabled={canSend||canVoice||canImage||canFile||canVideo||canEmoticon} busy={busy} value={target} onReady={setTargetReady} onChange={value=>{close();setTarget(value);setDraftKey(crypto.randomUUID());}}/>
    <details className="outbox-compose" open><summary>文字消息</summary>
      <form onSubmit={draft}>
        <label>文字<textarea aria-label="草稿文字" value={text} disabled={!canSend || busy} onChange={e => { setText(e.target.value); setDraftKey(crypto.randomUUID()); }}/></label>
        <div className="button-row"><span className="field" role="status">{textByteLength(text)} / {MAX_TEXT_BYTES} 字节</span><HelpTip label="文字长度说明">中文通常每字3字节，表情可能更多。上限按UTF-8字节计算；超限时请缩短文字，不会自动拆成多条或改发文件。</HelpTip></div>
        <button className="button" disabled={!canSend || !targetReady || busy || !validSendText(text) || !target.trim()}>保存并预览</button>
      </form>
    </details>
    <details className="outbox-media-options"><summary>图片、语音与其他媒体</summary>
    <details className="outbox-compose"><summary>创建原生语音草稿</summary>
      <div className="button-row"><span>从已导入音频创建语音</span><HelpTip label="语音草稿要求">先在工具与诊断的音频页导入音频，再回这里选择。要求16kHz单声道PCM16、20毫秒整帧、最长60秒。保存后可预听并逐条批准。</HelpTip></div>
      <form onSubmit={voiceDraft}><label>已导入音频<select aria-label="语音草稿音频" value={assetId} disabled={!canVoice||busy} onChange={e=>{setAssetId(e.target.value);setDraftKey(crypto.randomUUID());}}><option value="">选择音频</option>{voiceAssets.map(a=><option key={a.id} value={a.id}>{(a.audio.duration_ms/1000).toFixed(2)}秒 / {a.id}</option>)}</select></label>
        <button className="button" disabled={!canVoice||!targetReady||busy||!assetId}>保存语音草稿并预览</button></form>
    </details>
    <VideoComposer key={`${accountId}:${target}`} targetId={target} accountId={accountId} assets={data.file_assets.filter(a=>a.account_id===accountId)} canSend={canVideo&&targetReady} busy={busy} act={act} refresh={refreshWorkbench} onDraft={open} onError={setError}/>
    <EmoticonComposer key={`${accountId}:${target}`} targetId={target} accountId={accountId} assets={data.file_assets.filter(a=>a.account_id===accountId)} canSend={canEmoticon&&targetReady} busy={busy} act={act} refresh={refreshWorkbench} onDraft={open} onError={setError}/>
    <FileComposer key={`${accountId}:${target}`} targetId={target} accountId={accountId} assets={data.file_assets.filter(a=>a.account_id===accountId)} canSend={canFile&&targetReady} busy={busy} act={act} refresh={refreshWorkbench} onDraft={open} onError={setError}/>
    <ImageComposer key={`${accountId}:${target}`} targetId={target} accountId={accountId} assets={data.image_assets.filter(a=>a.account_id===accountId)} canSend={canImage&&targetReady} busy={busy} act={act} refresh={refreshWorkbench} onDraft={open} onError={setError}/>
    </details></div>
    <div hidden={workspace!=='tasks'}>
    <button className="button" disabled={!desktopAvailable || busy} onClick={() => void act(refresh)}>刷新发送队列</button>
    <SendTaskList items={items} busy={busy} onPreview={id=>void open(id)} onCancel={id=>{close();void act(()=>command('cancel_send',{accountId,outboxId:id}));}} onReconcile={id=>void act(()=>command('reconcile_send',{accountId,outboxId:id}))}/>
    {!historyDone&&<button className="button" disabled={busy} onClick={()=>void older()}>加载更多历史记录</button>}
    </div>
    {preview && <section className="outbox-preview" aria-label="发送预览">
      <div className="section-title"><h2 ref={previewHeading} tabIndex={-1}>{preview.payload.moment?.interaction?'确认朋友圈互动':preview.payload.moment?'确认发布朋友圈':'确认发送内容'}</h2><button className="button" onClick={close}>关闭预览</button></div>
      <dl><dt>发送者</dt><dd>{preview.payload.context.sender_id}</dd><dt>{preview.payload.moment?.interaction?'目标动态':preview.payload.moment?'发布位置':'收件人'}</dt><dd>{preview.payload.moment?.interaction?`${momentActionLabels[preview.payload.moment.interaction.action]} · 动态 ${preview.payload.moment.interaction.sns_id}`:preview.payload.moment?`朋友圈 · ${preview.payload.moment.visibility==='public'?'公开 · 朋友可见':'仅自己可见'}`:preview.payload.context.target_display_name}<code>{preview.payload.context.target_id}</code></dd>{preview.payload.moment?.interaction?.reply&&<><dt>回复对象</dt><dd>{preview.payload.moment.interaction.reply.username}<code>{preview.payload.moment.interaction.reply.id64??preview.payload.moment.interaction.reply.id}</code></dd></>}<dt>方式</dt><dd>{modes[preview.payload.context.execution_mode]}</dd><dt>当前状态</dt><dd>{sendStates[selected?.state ?? preview.status.state]}</dd>{preview.status.scheduled_at&&<><dt>计划发送</dt><dd>{new Date(preview.status.scheduled_at*1000).toLocaleString()}</dd></>}</dl>
      {preview.local_record&&<p className="field">微信本地记录：{preview.local_record.state==='written'?'已核对写入':preview.local_record.state==='failed'?'写入失败':'未知'}<HelpTip label="本地记录与送达结果">这是按原生因果编号、账号、会话、时间和类型核对的本机记录状态，与上方发送状态分别显示。已写入不代表接收端收到；缺少记录或读取失败仍为未知，不据此重复发送。{preview.local_record.observed_at?`最近证实写入：${new Date(preview.local_record.observed_at*1000).toLocaleString()}。`:''}重新打开预览可读取最新已保存证据。</HelpTip></p>}
      {preview.payload.quote&&<section className="quote-source" aria-label="引用原文"><strong>引用原文</strong><HelpTip label="引用原文批准说明">批准绑定原消息的服务端标识和完整正文摘要，以及此回复和收件人。原消息只能来自当前认证的会话；发送前原生索引会再核验，内容变化时不发送。</HelpTip><details><summary>查看完整原文</summary><p className="outbox-body" dir="auto">{preview.payload.quote.text}</p><small>消息标识：<code>{preview.payload.quote.server_id}</code></small></details></section>}
      {Boolean(preview.payload.mentions?.length)&&<div className="outbox-body"><strong>被提醒成员（@）</strong><HelpTip label="提醒成员批准说明">批准同时绑定以下精确成员标识、正文和所选群。昵称可能变化，因此在此显示标识；结果未知时不会自动重发。</HelpTip><ul>{preview.payload.mentions!.map(id=><li key={id}><code dir="auto">{id}</code></li>)}</ul></div>}
      {preview.payload.moment?.video ? <div className="outbox-body"><p>{preview.payload.text||'未添加正文'}</p><MomentVideoPreview key={preview.status.metadata.id} accountId={accountId} video={preview.payload.moment.video} cover={preview.payload.moment.images![0]} onError={setError} onReady={setImageReady}/></div> : Boolean(preview.payload.moment?.images?.length) ? <div className="outbox-body"><strong>图片动态 · {preview.payload.moment!.images!.length} 张</strong><p className="outbox-body">{preview.payload.text||'未添加正文'}</p><MomentImagePreview key={preview.status.metadata.id} accountId={accountId} images={preview.payload.moment!.images!} onError={setError} onReady={setImageReady}/><HelpTip label="动态图片批准说明">请核对全部图片及顺序。每张图片预览成功后才能批准，批准绑定同一份加密图片、正文、发布账号和所选可见范围。图片到期、被删除或内容变化时不会提交。</HelpTip></div> : preview.payload.emoticon ? <div className="outbox-body"><strong>原生表情 · {preview.payload.emoticon.info.frames} 帧</strong><HelpTip label="表情批准说明">先点预览查看同一份GIF，再批准一次发送。批准绑定字节、尺寸、帧数和收件人。关闭预览会停止动画。</HelpTip><EmoticonPreview key={preview.payload.emoticon.file.asset_id} accountId={accountId} assetId={preview.payload.emoticon.file.asset_id} onError={setError} onReady={setImageReady}/><details><summary>表情信息与摘要</summary><p>{preview.payload.emoticon.file.file_name} · {preview.payload.emoticon.info.width} × {preview.payload.emoticon.info.height}</p><code>{preview.payload.emoticon.file.sha256}</code></details></div> : preview.payload.video ? <div className="outbox-body"><strong>原生视频 · {(preview.payload.video.info.duration_ms/1000).toFixed(2)} 秒</strong><HelpTip label="视频批准说明">预览来自加密保存的同一份MP4。批准绑定内容摘要、文件名、时长和收件人；删除、过期或变化后不会提交。视频不会自动播放。</HelpTip><VideoPreview key={preview.payload.video.file.asset_id} accountId={accountId} assetId={preview.payload.video.file.asset_id} onError={setError} onReady={setImageReady}/><details><summary>视频信息与摘要</summary><p>{preview.payload.video.file.file_name} · {preview.payload.video.info.width} × {preview.payload.video.info.height}</p><code>{preview.payload.video.file.sha256}</code></details></div> : preview.payload.file ? <div className="outbox-body"><strong>原生文件 / {preview.payload.file.size}字节</strong><p>{preview.payload.file.file_name}</p><details><summary>批准文件摘要</summary><code>{preview.payload.file.sha256}</code></details><p className="field">批准绑定名称、字节内容、大小和收件人；文件过期、被删除或变化时不提交。</p></div> : preview.payload.image ? <div className="outbox-body"><strong>原生图片 / {preview.payload.image.width}×{preview.payload.image.height}</strong>
        <ImagePreview key={preview.payload.image.asset_id} accountId={accountId} assetId={preview.payload.image.asset_id} onError={setError} onReady={setImageReady}/>
        <details><summary>批准图片摘要</summary><code>{preview.payload.image.png_sha256}</code></details>
        <p className="field">批准绑定此图片和收件人；图片被删除或内容变化时不提交。</p></div> : preview.payload.voice ? <div className="outbox-body"><strong>原生语音 / {(preview.payload.voice.duration_ms/1000).toFixed(2)} 秒</strong>
        <AudioPreview key={preview.payload.voice.asset_id} accountId={accountId} assetId={preview.payload.voice.asset_id} disabled={busy} onError={setError}/>
        <details><summary>音频摘要</summary><code>{preview.payload.voice.source_sha256}</code></details>
        <p className="field">批准绑定该音频、编码器和收件人；音频被删除或内容变化时不提交。</p></div> : <p className="outbox-body">{preview.payload.text}</p>}
      {preview.status.state === 'awaiting_approval' && <>
        <details className="send-schedule"><summary>{scheduleEnabled?'计划发送 · '+(scheduleTime.replace('T',' ')||'请选择时间'):'发送时间 · 立即发送'}</summary>
          <label className="check-line"><input type="checkbox" checked={scheduleEnabled} disabled={busy||expired||!preview.schedule_before} onChange={e=>{setScheduleEnabled(e.target.checked);setConfirmed(false);}}/>安排稍后发送</label>
          {scheduleEnabled&&<label>本地时间<input type="datetime-local" aria-label="计划发送时间" value={scheduleTime} min={localMinute(Math.floor(Date.now()/60000)*60+60)} max={localMinute((preview.schedule_before??0)-1)} disabled={busy||expired} onChange={e=>{setScheduleTime(e.target.value);setConfirmed(false);}}/></label>}
          <HelpTip label="计划发送说明">计划保存在本机队列，批准同时绑定发送时间、内容和收件人。草稿创建后最多保留一小时，计划时间需早于到期时间。联系人和群的收件人选择仅约五分钟有效，因此可选时间还会受收件人期限限制；界面显示两者中的较早期限。到点时需WeRC运行且原微信连接仍有效；系统休眠或关闭期间不会发送，恢复后只有计划时间起最多五分钟且草稿未过期时才可提交。更换账号、连接、删除附件或权限失效会取消执行。未到时间不会提交，结果未知不会重发。更改已批准计划请先取消，再创建新草稿。</HelpTip>
          {scheduleEnabled&&<p className="field">可计划至：{new Date((preview.schedule_before??preview.status.metadata.expires_at)*1000).toLocaleString()}之前 · {Intl.DateTimeFormat().resolvedOptions().timeZone}</p>}
          {scheduleInvalid&&<p className="field" role="status">请选择未来且在当前可用期限内的有效本地时间。</p>}
        </details>
        <label className="check-line"><input type="checkbox" checked={confirmed} disabled={busy || expired} onChange={e => setConfirmed(e.target.checked)}/>{preview.payload.moment?.interaction?`确认以上账号、动态标识和${momentActionLabels[preview.payload.moment.interaction.action]}，`:preview.payload.moment?`确认以上发布账号、朋友圈${preview.payload.moment.visibility==='public'?'公开（朋友可见）':'仅自己可见'}、`:'确认以上发送者、唯一收件人、'}{preview.payload.moment?.video?'正文、视频与封面':preview.payload.moment?.images?.length?'文字、图片内容和顺序':preview.payload.emoticon?'表情内容':preview.payload.video?'视频内容':preview.payload.file?'文件名称、大小及内容':preview.payload.image?'图片内容':preview.payload.voice?'语音内容':'文字'}和方式{scheduleEnabled?`，计划于${scheduleTime.replace('T',' ')}（本地时间）发送`:''}</label>
        <button className="button primary" disabled={busy || scheduleInvalid || !confirmed || (Boolean(preview.payload.image || preview.payload.video || preview.payload.emoticon || preview.payload.moment?.images?.length) && !imageReady) || expired || selected?.state !== 'awaiting_approval' || !preview.payload.context.identity_verified || !preview.payload.context.target_verified} onClick={() => void approve()}>{scheduleEnabled?'批准一次 / 按计划发送':'批准一次 / 五分钟内提交'}</button>
      </>}
    </section>}
  </section>;
}

/** Keep unresolved tasks visible; finished records remain available on demand. */
export function SendTaskList({items,busy,onPreview,onCancel,onReconcile}:{items:SendStatus[];busy:boolean;onPreview:(id:string)=>void;onCancel:(id:string)=>void;onReconcile:(id:string)=>void}) {
  const [pendingLimit,setPendingLimit]=useState(20),[historyLimit,setHistoryLimit]=useState(20);
  const finished=(item:SendStatus)=>['recipient_confirmed','rejected','canceled','expired'].includes(item.state);
  const pending=items.filter(item=>!finished(item)),history=items.filter(finished);
  const rows=(values:SendStatus[]) => <ul className="outbox-list">{values.map(item=><li key={item.metadata.id}>
    <div><strong>{item.state==='queued'&&item.scheduled_at&&item.scheduled_at>Date.now()/1000?'等待计划时间':sendStates[item.state]}</strong>{item.scheduled_at&&<p className="field">计划发送：{new Date(item.scheduled_at*1000).toLocaleString()}</p>}{[9,10,11].includes(item.metadata.schema_version??0)&&<span className="field"> · 朋友圈</span>}<details className="outbox-meta"><summary>任务信息</summary><code>{item.metadata.id}</code></details><p className="field">{new Date(item.metadata.created_at*1000).toLocaleString()} / {item.metadata.creator_id==='desktop'?'本机草稿':'API 草稿'}</p></div>
    <div className="outbox-actions"><button className="button" disabled={busy} onClick={()=>onPreview(item.metadata.id)}>预览</button>
      {['awaiting_approval','queued'].includes(item.state)&&<button className="button" disabled={busy} onClick={()=>onCancel(item.metadata.id)}>取消待发</button>}
      {['unknown','server_accepted'].includes(item.state)&&<button className="button" disabled={busy} onClick={()=>onReconcile(item.metadata.id)}>回查 / 不重发</button>}
    </div>
  </li>)}</ul>;
  if(!items.length)return <p className="empty-inline">尚无发送任务。</p>;
  return <>
    <section aria-label="待处理发送任务"><div className="section-title"><h3>待处理 / {pending.length}</h3><HelpTip label="待处理任务说明">待批准、排队中、提交中，以及结果未知或仅服务器接受的任务保留在这里。回查不会重新发送；只有接收端确认、提交前拒绝、取消或过期的任务才收入已结束记录。</HelpTip></div>
      {pending.length?rows(pending.slice(0,pendingLimit)):<p className="empty-inline">没有待处理任务。</p>}
      {pending.length>pendingLimit&&<button className="button" disabled={busy} onClick={()=>setPendingLimit(n=>n+20)}>显示更多待处理任务（{Math.min(pendingLimit,pending.length)} / {pending.length}）</button>}
    </section>
    {history.length>0&&<details className="outbox-history"><summary>已结束记录 / {history.length}</summary>{rows(history.slice(0,historyLimit))}
      {history.length>historyLimit&&<button className="button" disabled={busy} onClick={()=>setHistoryLimit(n=>n+20)}>显示更多已结束记录（{Math.min(historyLimit,history.length)} / {history.length}）</button>}
    </details>}
  </>;
}
