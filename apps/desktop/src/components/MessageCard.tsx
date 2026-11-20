import {type ReactNode,useState} from 'react';
import {HelpTip} from './HelpTip';

export interface ForwardRecordEntry {author:string;timestamp:string;data_type:number;text:string|null;image_available?:boolean;file_available?:boolean;detail?:Extract<CardContent,{kind:'link'|'location'}>;nested?:{entries:ForwardRecordEntry[];omitted:number;depth_limited:boolean}}
export type CardContent=
 |{kind:'record';detail:{kind:string;title:string;text:string;fields:Record<string,string>;url:string|null}}
 |{kind:'contact';username:string;nickname:string}
 |{kind:'location';latitude:string;longitude:string;label:string;place_name:string}
 |{kind:'app_share';category:'music'|'video'|'mini_program';title:string;description:string;source_name:string;url:string|null}
 |{kind:'link';title:string;description:string;url:string|null}
 |{kind:'quote';text:string;author:string;quoted_type:number;quoted_text:string|null}
 |{kind:'forward';title:string;entries:ForwardRecordEntry[];omitted:number};
export interface ConversationCard {local_id:string;source:string|null;content:CardContent}
export function MessageCard({card,renderImage,renderFile}:{card:CardContent;renderImage?:(path:number[])=>ReactNode;renderFile?:(path:number[])=>ReactNode}){
 const [notice,setNotice]=useState('');
 async function copy(value:string,label='链接'){try{await navigator.clipboard.writeText(value);setNotice(`${label}已复制`);}catch{setNotice('复制失败，可以选中上方内容手动复制');}}
 if(card.kind==='record')return <div className="message-card record-card">
  <strong className="card-title" dir="auto">{card.detail.title||(({transfer:'转账消息',red_packet:'红包消息',friend_request:'好友验证记录',call:'通话记录',revoke:'撤回通知',system:'系统通知'} as Record<string,string>)[card.detail.kind]??'消息记录')}</strong>
  {card.detail.text&&<p className="card-body" dir="auto">{card.detail.text}</p>}
  {Object.keys(card.detail.fields).length>0&&<details><summary>查看记录详情</summary><dl>{Object.entries(card.detail.fields).map(([label,value])=><div key={label}><dt>{label}</dt><dd className="card-body" dir="auto">{value}</dd></div>)}</dl></details>}
  <HelpTip label="消息记录说明">这里显示来源消息保存的可读内容及原始字段。转账、红包和好友验证状态只代表该条记录，不代表当前资金或好友关系；撤回通知无法补出从未保存的原文。不会执行收款、领红包或添加好友。</HelpTip>
 </div>;
 if(card.kind==='contact')return <div className="message-card contact-card"><details>
  <summary><span dir="auto">{card.nickname||card.username}</span><small>联系人名片</small></summary>
  <p className="card-body" dir="auto">{card.nickname||'未保存昵称'}</p>
  <p className="card-url" dir="ltr">{card.username}</p>
  <button type="button" className="button" onClick={()=>void copy(card.username,'联系人标识')}>复制联系人标识</button>
  {notice&&<p role="status">{notice}</p>}
 </details><HelpTip label="联系人名片说明">显示消息里保存的昵称和联系人标识。标识不一定是可搜索的微信号，名片不代表已添加好友；请在微信确认并添加。头像不会自动加载。</HelpTip></div>;
 if(card.kind==='location')return <div className="message-card location-card"><details>
  <summary><span dir="auto">{card.place_name||card.label||'分享的位置'}</span><small>位置详情</small></summary>
  {card.place_name&&<p className="card-body" dir="auto">{card.place_name}</p>}
  {card.label&&<p className="card-body" dir="auto">{card.label}</p>}
  <p className="card-url" dir="ltr">纬度 {card.latitude} · 经度 {card.longitude}</p>
  <button type="button" className="button" onClick={()=>void copy(`${card.latitude}, ${card.longitude}`,'坐标')}>复制坐标</button>
  {notice&&<p role="status">{notice}</p>}
 </details><HelpTip label="位置消息说明">这是消息发送时保存的地点和原始坐标，不是实时位置。坐标未转换地图制式，复制到其他地图可能有偏移；需要导航时请在微信打开原消息。这里不会自动加载地图。</HelpTip></div>;
 if(card.kind==='quote')return <div className="message-card quote-card">
  <span className="message-body" dir="auto">{card.text||'引用回复'}</span>
  <details><summary><span dir="auto">{card.author||'原消息发送者未解析'}：{card.quoted_text??`消息类型 ${card.quoted_type}`}</span></summary>
   <p className="card-body" dir="auto">{card.quoted_text??'原消息为媒体或应用内容，暂不在引用内展开。'}</p>
  </details>
  <HelpTip label="引用消息说明">这里显示回复中保存的原消息摘要。发送者名称来自该条引用记录，不表示已核验其当前身份；原消息可能已删除或撤回。媒体引用暂只显示类型，不会自动下载。</HelpTip>
 </div>;
 if(card.kind==='forward')return <div className="message-card forward-card"><details>
  <summary><span dir="auto">{card.title||'合并聊天记录'}</span><small>{card.entries.length+card.omitted} 条缓存记录</small></summary>
  <ForwardEntries entries={card.entries} renderImage={renderImage} renderFile={renderFile} path={[]}/>
  {card.omitted>0&&<p className="empty-inline">还有 {card.omitted} 条本层记录未展开，请在微信查看完整内容。</p>}
  {!card.entries.length&&<p className="empty-inline">这条消息未保存可展示的记录。</p>}
 </details><HelpTip label="合并记录说明">显示这条消息内保存的文本、发送者名称和时间，整条消息最多展开100条、4层记录。再次转发的记录可按层展开，链接与位置详情可单独展开或复制；各层支持的图片可逐张读取、预览和导出；各层附件可读取微信本地已缓存的完整文件并导出。尚未缓存的附件需先在微信打开。其他尚未适配的媒体显示类型。名称与时间来自转发内容，不代表当前联系人身份或服务器核验结果。</HelpTip></div>;
 const shareName=card.kind==='app_share'?({music:'音乐分享',video:'视频分享',mini_program:'小程序'}[card.category]):'分享链接';
 return <div className={`message-card ${card.kind==='app_share'?'app-share-card':'link-card'}`}><details>
  <summary><span dir="auto">{card.title||shareName}</span><small>{card.kind==='app_share'?shareName:'链接详情'}</small></summary>
  {card.kind==='app_share'&&card.source_name&&<p className="card-source card-body" dir="auto">{card.source_name}</p>}
  {card.kind==='app_share'&&card.title&&<p className="card-title card-body" dir="auto">{card.title}</p>}
  {card.description&&<p className="card-body" dir="auto">{card.description}</p>}
  {card.url?<><p className="card-url" dir="ltr">{card.url}</p><button type="button" className="button" onClick={()=>void copy(card.url!)}>复制链接</button></>:<p className="empty-inline">{card.kind==='app_share'?'此卡片没有网页链接，请在微信打开原消息。':'没有可用的 HTTP 或 HTTPS 链接。'}</p>}
  {notice&&<p role="status">{notice}</p>}
 </details><HelpTip label={card.kind==='app_share'?'分享卡片说明':'链接消息说明'}>{card.kind==='app_share'?'这里显示消息保存的标题、来源和描述；音乐、视频与小程序本身请在微信打开。可用的网页链接可以复制，不会自动播放、跳转或加载封面。':'展开后可查看描述和完整地址，复制后可在浏览器打开。这里不自动访问链接或加载外部缩略图；标题和描述来自消息内容。'}</HelpTip></div>;
}

function ForwardEntries({entries,renderImage,renderFile,path}:{entries:ForwardRecordEntry[];renderImage?:(path:number[])=>ReactNode;renderFile?:(path:number[])=>ReactNode;path:number[]}){
 return <div className="forward-entries" role="list">{entries.map((entry,index)=><div role="listitem" key={index}>
  <div className="forward-heading"><div className="forward-author"><strong dir="auto">{entry.author||'发送者未解析'}</strong>{entry.author&&<HelpTip label="查看转发记录发送者"><span dir="auto">{entry.author}</span></HelpTip>}</div><span>{entry.timestamp}</span></div>
  {entry.nested?<details className="forward-nested">
   <summary><span>再次转发的记录</span><small>{entry.nested.entries.length+entry.nested.omitted} 条缓存记录</small></summary>
   <ForwardEntries entries={entry.nested.entries} renderImage={renderImage} renderFile={renderFile} path={[...path,index]}/>
   {entry.nested.depth_limited?<p className="empty-inline">已达到展开层级上限，请在微信查看更深层内容。</p>:entry.nested.omitted>0?<p className="empty-inline">还有 {entry.nested.omitted} 条本层记录未展开，请在微信查看完整内容。</p>:!entry.nested.entries.length?<p className="empty-inline">这层没有可展示的记录。</p>:null}
  </details>:entry.detail?<MessageCard card={entry.detail}/>:<p className="card-body" dir="auto">{entry.text??(entry.image_available?'图片':entry.file_available?'附件':`媒体或应用记录（类型 ${entry.data_type}），暂未展开`)}</p>}
  {entry.image_available&&renderImage?.([...path,index])}
  {entry.file_available&&renderFile?.([...path,index])}
 </div>)}</div>;
}
