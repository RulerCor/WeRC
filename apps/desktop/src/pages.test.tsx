import {MomentImagePreview} from './pages/MomentImagePreview';
import {MomentComposer} from './pages/MomentComposer';
import {QuoteComposer} from './pages/QuoteComposer';
import {NativeChat} from './pages/NativeChat';
import {conversationName,observationDirection,type NativeRow} from './lib/native-receive';
import {MessageCard} from './components/MessageCard';
import {ConversationMedia} from './pages/ConversationMedia';
import {ReceivedEmoticon} from './pages/ReceivedEmoticon';
import { describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import App from './App';
import { History } from './pages/History';
import { DataSources } from './pages/DataSources';
import { KeyAcquisition } from './pages/KeyAcquisition';
import { AudioPreview } from './pages/AudioPreview';
import { Connections } from './pages/Connections';
import { ApiSettings } from './pages/ApiSettings';
import { Outbox, SendTaskList, sendStates, type SendStatus } from './pages/Outbox';
import { Capabilities } from './pages/Capabilities';
import type { Workbench } from './hooks/useWorkbench';

vi.mock('./lib/desktop', () => ({ desktopAvailable: false, command: vi.fn() }));

const base: Workbench = {
  data: {
    current_connection:{account_id:null,revision:'0',selection_required:false,identity_verified:false},
    status: { product: 'WeRC_Mon3trBot', version: '0.1.0', stage: 'foundation_development', api_enabled: false, real_wechat_verified: false },
    accounts: [], tokens: [], capabilities: [], api_address: null, bindings: [], key_statuses: [], audio_assets: [], image_assets: [], file_assets: [], operations: [], history_sources: [],
  },
  loaded: true, completed: 0, busy: false, error: '', setError: vi.fn(), refresh: async () => {}, act: async () => {},
};

describe('精简界面与真实状态', () => {
  it('发送队列分批展示全部保留任务，未知与服务器接受不藏入已结束记录', () => {
    const items:SendStatus[]=Array.from({length:512},(_,i)=>({metadata:{id:`owned-${i}`,account_id:'owned',creator_id:'desktop',request_digest:'a'.repeat(64),context_digest:'b'.repeat(64),connection_revision:1,expires_at:9999999999,created_at:1,schema_version:i===0?10:1},state:i===0?'unknown':i===1?'server_accepted':i<212?'awaiting_approval':'canceled',approval_expires_at:null,attempt_id:null,evidence:null,updated_at:1}));
    const action=vi.fn();
    const html=renderToStaticMarkup(<SendTaskList items={items} busy={false} onPreview={action} onCancel={action} onReconcile={action}/>);
    expect(html).toContain('待处理 / 212');expect(html).toContain('已结束记录 / 300');
    expect(html).toContain('显示更多待处理任务（20 / 212）');expect(html).toContain('显示更多已结束记录（20 / 300）');
    expect((html.match(/<li>/g)||[]).length).toBe(40);
    expect(html.indexOf('结果未知 / 不会重发')).toBeLessThan(html.indexOf('class="outbox-history"'));
    expect(html.indexOf('服务器已接受')).toBeLessThan(html.indexOf('class="outbox-history"'));
    expect(html).toContain('朋友圈');
    expect(html).not.toContain('class="outbox-history" open');
    expect(html).not.toContain('owned-40');expect(action).not.toHaveBeenCalled();
  });
  it('发送队列区分待批准与送达，预览不装配发送或自行生成任务', () => {
    expect(renderToStaticMarkup(<Outbox {...base}/>)).toContain('配置当前连接后');
    const props: Workbench = {...base,data:{...base.data,current_connection:{...base.data.current_connection,account_id:'synthetic-current'}}};
    const markup = renderToStaticMarkup(<Outbox {...props}/>);
    expect(markup).toContain('真实发送未就绪');
    expect(markup).toContain('aria-label="发送工作区说明"');
    expect(markup).toContain('aria-expanded="false"');
    expect(markup).toContain('任务记录');
    expect(markup).toMatch(/<textarea[^>]*disabled=""/);
    expect(markup).toContain('保存并预览');
    expect(markup).toContain('<details class="outbox-media-options"><summary>图片、语音与其他媒体</summary>');
    expect(markup).not.toContain('保存并预览 / 不直接发送');
    expect(markup).not.toContain('确认发送内容');
    expect(sendStates.unknown).toContain('不会重发');
    expect(sendStates.server_accepted).not.toBe(sendStates.recipient_confirmed);
  });
  it('自动取钥默认折叠、必须显式确认，预览不读取进程', () => {
    const installation={id:'fixture',executable_path:'C:/synthetic/Weixin.exe',version:'4.1.11.65535',architecture:'x86_64',product_name:'synthetic',sha256:'1'.repeat(64),process_ids:[123],sources:['fixture'],compatibility:'unsupported',reason:'synthetic'};
    const markup=renderToStaticMarkup(<KeyAcquisition accountId="synthetic" installation={installation} busy={false} act={base.act}/>);
    expect(markup).toContain('确认只读取钥');
    expect(markup).toContain('查找数据库位置');
    expect(markup).toContain('确认检测数据位置');
    expect(markup.match(/<form\b/g)).toHaveLength(2);
    expect(markup).not.toContain('<details open');
    expect(markup).toMatch(/<button class="button" disabled=""/);
    expect(markup).not.toContain('value="123"');
    expect(markup).toContain('不提权、不注入、不写来源');
    expect(renderToStaticMarkup(<KeyAcquisition accountId="synthetic" installation={{...installation,version:'4.0.0.1'}} busy={false} act={base.act}/>)).toContain('此安装版本或架构尚未适配取钥布局');
    expect(renderToStaticMarkup(<KeyAcquisition accountId="synthetic" installation={{...installation,version:'4.2.0.52'}} busy={false} act={base.act}/>)).not.toContain('此安装版本或架构尚未适配取钥布局');
  });
  it('数据位置检测默认折叠并需要确认，不猜目录身份', () => {
    const markup=renderToStaticMarkup(<DataSources busy={false} act={base.act} onChoose={vi.fn()}/>);
    expect(markup).toContain('查找数据库位置');
    expect(markup).toContain('仅检测目录和文件元数据');
    expect(markup).not.toContain('<details open');
    expect(markup).toMatch(/<button class="button" disabled=""/);
    expect(markup).not.toContain('wxid_');
  });
  it('预听按需解密，不自动加载或播放音频', () => {
    const markup=renderToStaticMarkup(<AudioPreview accountId="synthetic" assetId="fixture" disabled={false} onError={vi.fn()}/>);
    expect(markup).toContain('预听音频'); expect(markup).not.toContain('<audio'); expect(markup).not.toContain('autoplay');
  });
  it('记录页明确离线边界，预览不启用来源操作或加载假数据', () => {
    const markup = renderToStaticMarkup(<History {...base}/>);
    expect(markup).toContain('离线快照 / 非实时收信');
    expect(markup).toContain('选择归档范围后导入和查询记录');
    expect(markup).toMatch(/<select[^>]*disabled=""/);
    expect(markup).not.toContain('synthetic');
  });

  it('预览只显示必要边界，不注入演示账号或产品宣言', () => {
    const markup = renderToStaticMarkup(<App/>);
    expect(markup).toContain('界面预览');
    expect(markup).toContain('微信待连接');
    expect(markup).toContain('连接当前微信');
    expect(markup).toContain('会话准备连接');
    expect(markup).not.toContain('聊天读取模式');
    expect(markup).toContain('工具与诊断');
    expect(markup).toContain('使用指南');
    expect(markup).not.toContain('能力在本机，智能在外部');
    expect(markup).not.toContain('连接，不止一个开关');
    expect(markup).not.toContain('CONNECTION CHECKLIST');
    expect(markup.match(/class="nav-item/g)).toHaveLength(8);
  });

  it('预览中的登记操作禁用，账号标签按文本转义', () => {
    const props: Workbench = { ...base, data: { ...base.data, current_connection:{...base.data.current_connection,account_id:'synthetic-account'}, accounts: [{ id: 'synthetic-account', label: '<script>test</script>', state: 'registered', created_at: '2026-01-01T00:00:00Z' }] } };
    const markup = renderToStaticMarkup(<Connections {...props}/>);
    expect(renderToStaticMarkup(<Connections {...base}/>)).toMatch(/<button class="button primary" disabled=""/);
    expect(markup).toMatch(/<button class="button" disabled=""/);
    expect(markup).toContain('&lt;script&gt;test&lt;/script&gt;');
    expect(markup).not.toContain('<script>test</script>');
    expect(markup).toContain('未连接');
  });

  it('连接状态来自核心而非在页面硬编码', () => {
    const props: Workbench = { ...base, data: { ...base.data, current_connection:{...base.data.current_connection,account_id:'synthetic-account'}, accounts: [{ id: 'synthetic-account', label: '合成账号', state: 'needs_key', created_at: '2026-01-01T00:00:00Z' }] } };
    expect(renderToStaticMarkup(<Connections {...props}/>)).toContain('待配置');
  });

  it('实现证据与当前可用性分别显示，详情默认折叠', () => {
    const props: Workbench = { ...base, data: { ...base.data, capabilities: [{ id: 'synthetic.capability', evidence: 'fixture_verified', availability: 'blocked', reason: '合成测试原因', execution_mode: null, requires_foreground: false, requires_user_action: false }] } };
    const markup = renderToStaticMarkup(<Capabilities {...props}/>);
    expect(markup).toContain('合成测试通过');
    expect(markup).toContain('受阻');
    expect(markup).toContain('<details><summary>原因</summary>');
    expect(markup).not.toContain('<details open');
  });

  it('重新进入 API 页时显示实际监听端口', () => {
    const props: Workbench = { ...base, data: { ...base.data, api_address: 'http://127.0.0.1:6100', status: { ...base.data.status, api_enabled: true } } };
    const markup = renderToStaticMarkup(<ApiSettings {...props}/>);
    expect(markup).toContain('value="6100"');
    expect(markup).toContain('http://127.0.0.1:6100');
    expect(markup).toContain('运行中');
  });

  it('令牌显示撤销与过期状态，不将过期令牌显示为有效', () => {
    const token = { id: 'synthetic-token', label: '合成令牌', account_ids: ['synthetic-account'], scopes: ['accounts:read'], created_at: '2000-01-01T00:00:00Z', expires_at: '2000-01-02T00:00:00Z', revoked: false };
    const props: Workbench = { ...base, data: { ...base.data, tokens: [token] } };
    expect(renderToStaticMarkup(<ApiSettings {...props}/>)).toContain('已过期');
    props.data = { ...props.data, tokens: [{ ...token, expires_at: '9999-12-31T23:59:59+00:00' }] };
    expect(renderToStaticMarkup(<ApiSettings {...props}/>)).toContain('永久有效');
    props.data = { ...props.data, tokens: [{ ...props.data.tokens[0], revoked: true }] };
    expect(renderToStaticMarkup(<ApiSettings {...props}/>)).toContain('已撤销');
  });
});

it('接收表情默认静止、显示明确读取入口和收纳说明',()=>{
  const markup=renderToStaticMarkup(<ReceivedEmoticon accountId="owned" viewId="owned-view" localId="1"/>);
  expect(markup).toContain('读取表情');
  expect(markup).toContain('接收表情说明');
  expect(markup).not.toContain('<img');
  expect(markup).not.toContain('另存 GIF');
  expect(renderToStaticMarkup(<ReceivedEmoticon accountId="owned" position={1} connected={false}/>)).toMatch(/<button class="button" disabled=""/);
});

it('所选会话媒体默认仅显示读取入口和问号，五类均不自动读取或播放',()=>{
 for(const kind of ['image','voice','video','file','emoticon'] as const){const html=renderToStaticMarkup(<ConversationMedia accountId="owned" viewId="owned-view" hint={{local_id:'1',source:null,kind}}/>);expect(html).toContain('读取');expect(html).toContain('读取说明');expect(html).not.toMatch(/<(img|video|audio)\b/);expect(html).not.toContain('导出文件名');expect(html).not.toContain('autoplay');}
});


describe('按需展开会话卡片',()=>{
 it('链接仅显示可复制详情，正文不作为HTML或自动网络资源',()=>{
  const html=renderToStaticMarkup(<MessageCard card={{kind:'link',title:'自动化测试链接',description:'<img src="https://example.test/tracker">',url:'https://example.test/path'}}/>);
  expect(html).toContain('复制链接');expect(html).toContain('<details>');expect(html).not.toContain('<details open');expect(html).not.toContain('<img');expect(html).not.toContain('href=');expect(html).toContain('&lt;img');
 });
 it('引用与合并记录保持层级和未展开媒体提示',()=>{
  const quote=renderToStaticMarkup(<MessageCard card={{kind:'quote',text:'自动化测试回复',author:'发送者',quoted_type:49,quoted_text:null}}/>);
  expect(quote).toContain('自动化测试回复');expect(quote).toContain('消息类型 49');expect(quote).toContain('<details>');
  const forward=renderToStaticMarkup(<MessageCard card={{kind:'forward',title:'自动化测试记录',entries:[{author:'测试者',timestamp:'2026-11-12',data_type:1,text:'<script>not html</script>'},{author:'',timestamp:'',data_type:2,text:null}],omitted:2}}/>);
  expect(forward).toContain('4 条缓存记录');expect(forward).toContain('还有 2 条本层记录未展开');expect(forward).toContain('暂未展开');expect(forward).not.toContain('<script>');expect(forward).not.toContain('<details open');expect(forward).toContain('role="listitem"');
 });
});

it('实时接收按会话收纳，旧记录不伪造方向，群信封不伪造成员',()=>{
  const markup=renderToStaticMarkup(<NativeChat {...base} accountId="owned"/>);
  expect(markup).toContain('原生实时接收');expect(markup).toContain('接收会话筛选');
  expect(markup).toContain('全部最近会话');expect(markup).toContain('连接当前微信后');
  expect(conversationName('filehelper')).toBe('文件传输助手');
  expect(conversationName('x'.repeat(110)+'@chatroom').length).toBeLessThan(40);
  const row:NativeRow={position:1,payload:{id:'owned',sender_id:'me',target_id:'g@chatroom',observed_at:'',message:{text:'',native_message_id:'1'}}};
  expect(observationDirection(row)).toBe('原生观测');
  row.payload.route={sender:'g@chatroom',receiver:'me',conversation:'g@chatroom'};
  expect(observationDirection(row)).toBe('群内消息');
  row.payload.route={sender:'me',receiver:'contact',conversation:'contact'};
  expect(observationDirection(row)).toBe('我发出的消息');
});

it('引用回复默认收纳，帮助按需出现且空回复不可保存', () => {
  const markup=renderToStaticMarkup(<QuoteComposer accountId="owned" viewId="view" localId="9007199254740993" source={null} username="owned-target"/>);
  expect(markup).toContain('<details class="quote-compose"><summary>引用回复</summary>');
  expect(markup).not.toContain('<details open');
  expect(markup).toContain('引用回复说明');
  expect(markup).toContain('0 / 10000 字节');
  expect(markup).toMatch(/<button class="button" disabled=""/);
  expect(markup).not.toContain('owned-target');
});

it('转发图片按原条目序号提供读取入口，未读取时不创建媒体元素',()=>{
 const indices:number[][]=[];
 const html=renderToStaticMarkup(<MessageCard card={{kind:'forward',title:'自动化测试',entries:[{author:'',timestamp:'',data_type:1,text:'文字'},{author:'',timestamp:'',data_type:2,text:null,image_available:true},{author:'',timestamp:'',data_type:2,text:null,image_available:false}],omitted:0}} renderImage={index=>{indices.push(index);return <ConversationMedia accountId="owned" viewId="owned-view" hint={{local_id:'1',source:'owned-source',kind:'image'}} forwardPath={index}/>;}}/>);
 expect(indices).toEqual([[1]]);expect(html).toContain('读取图片');expect(html).not.toMatch(/<(img|video|audio)\b/);expect(html).not.toContain('<details open');
});

it('再次转发逐层收纳，文字转义，深层图片使用完整路径',()=>{
 const calls:number[][]=[];
 const html=renderToStaticMarkup(<MessageCard card={{kind:'forward',title:'自动化测试',omitted:0,entries:[{author:'外层',timestamp:'',data_type:17,text:null,nested:{entries:[{author:'内层',timestamp:'',data_type:1,text:'<img src="https://example.test/private">'},{author:'',timestamp:'',data_type:2,text:null,image_available:true}],omitted:3,depth_limited:false}},{author:'',timestamp:'',data_type:17,text:null,nested:{entries:[],omitted:1,depth_limited:true}}]}} renderImage={i=>{calls.push(i);return null;}}/>);
 expect(html).toContain('再次转发的记录');expect(html).toContain('还有 3 条本层记录未展开');expect(html).toContain('已达到展开层级上限');expect(html).toContain('&lt;img');expect(html).not.toContain('<img');expect(html).not.toMatch(/<details[^>]* open/);expect(calls).toEqual([[0,1]]);
});

it('合并附件以完整层级路径读取，默认收起并说明本地缓存来源',()=>{
 const paths:number[][]=[];
 const html=renderToStaticMarkup(<MessageCard card={{kind:'forward',title:'自动化测试',entries:[{author:'',timestamp:'',data_type:8,text:null,file_available:true},{author:'',timestamp:'',data_type:17,text:null,nested:{entries:[{author:'',timestamp:'',data_type:1,text:'文字'},{author:'',timestamp:'',data_type:8,text:null,file_available:true}],omitted:0,depth_limited:false}}],omitted:0}} renderFile={path=>{paths.push(path);return <ConversationMedia accountId="owned" viewId="owned-view" hint={{local_id:'1',source:'owned-source',kind:'file'}} forwardPath={path}/>;}}/>);
 expect(paths).toEqual([[0],[1,1]]);expect(html.match(/读取附件<\/button>/g)).toHaveLength(2);expect(html).toContain('附件读取说明');expect(html).not.toContain('仅读取当前账号的微信本地缓存');expect(html).not.toMatch(/<details[^>]* open/);expect(html).not.toMatch(/<(img|video|audio)\b/);
});

it('私密动态按需展开，明确发布范围且保存不会直接发布',()=>{
 const html=renderToStaticMarkup(<MomentComposer accountId="owned" enabled={false}/>);
 expect(html).toContain('写一条朋友圈动态');expect(html).toContain('仅自己可见');
 expect(html).toContain('保存到发送队列');expect(html).toContain('朋友圈发布说明');
 expect(html).not.toMatch(/<details[^>]* open/);expect(html).not.toContain('服务器已接受表示');
 expect(html).toContain('0 / 10000 字节');expect(html).toMatch(/<textarea[^>]* disabled/);
});

it('名片位置和应用分享默认收纳，按需帮助且不执行远程内容',()=>{
 const cards=[{kind:'contact' as const,username:'owned-id',nickname:'<img src="https://example.test/private">'}, {kind:'location' as const,latitude:'31',longitude:'121',label:'<script>test</script>',place_name:'测试位置'}, ...(['music','video','mini_program'] as const).map(category=>({kind:'app_share' as const,category,title:'测试分享',description:'<iframe>literal</iframe>',source_name:'测试应用',url:null}))];
 for(const card of cards){const html=renderToStaticMarkup(<MessageCard card={card}/>);expect(html).toContain('<details>');expect(html).not.toContain('<details open');expect(html).toContain('aria-expanded="false"');expect(html).not.toMatch(/<(img|script|iframe|audio|video)\b/);expect(html).not.toContain('href=');expect(html).toContain('说明');}
 expect(renderToStaticMarkup(<MessageCard card={cards[0]}/>)).toContain('复制联系人标识');
 expect(renderToStaticMarkup(<MessageCard card={cards[1]}/>)).toContain('复制坐标');
});

it('图片动态默认收纳，显示PNG与JPEG选择和按需帮助',()=>{
 const html=renderToStaticMarkup(<MomentComposer accountId="owned"/>);
 expect(html).toContain('添加图片（可选）');expect(html).toContain('动态图片说明');
 expect(html).toMatch(/<input[^>]*type="file"[^>]*multiple=""[^>]*accept=".png,.jpg,.jpeg,image\/png,image\/jpeg"/);
 expect(html).not.toContain('选择后会在本机加密保存24小时');expect(html).not.toMatch(/<details[^>]* open/);
});
it('图片动态按数组顺序显示所有预览且不提前显示敏感路径',()=>{
 const asset={asset_id:'owned-asset',source_sha256:'a'.repeat(64),png_sha256:'b'.repeat(64),width:640,height:360};
 const html=renderToStaticMarkup(<MomentImagePreview accountId="owned" images={[asset,{...asset,width:360,height:640}]} onError={()=>{}} onReady={()=>{}}/>);
 expect(html).toContain('待发布图片顺序');expect(html.indexOf('640×360')).toBeLessThan(html.indexOf('360×640'));
 expect(html.match(/正在读取图片预览/g)).toHaveLength(2);expect(html).not.toContain('owned-asset');expect(html).not.toMatch(/<details[^>]* open/);
});


it('转发内链接与位置复用默认折叠卡片，不访问外部资源',()=>{
 const html=renderToStaticMarkup(<MessageCard card={{kind:'forward',title:'自动化测试',omitted:0,entries:[{author:'',timestamp:'',data_type:17,text:null,nested:{omitted:0,depth_limited:false,entries:[{author:'',timestamp:'',data_type:5,text:null,detail:{kind:'link',title:'自动化测试链接',description:'<img src="https://example.test/tracker">',url:'https://example.test/'}},{author:'',timestamp:'',data_type:6,text:null,detail:{kind:'location',latitude:'31.25',longitude:'121.5',label:'自动化测试地址',place_name:'自动化测试地点'}}]}}]}}/>);
 expect(html).toContain('再次转发的记录');expect(html).toContain('自动化测试链接');expect(html).toContain('复制链接');expect(html).toContain('复制坐标');expect(html).toContain('纬度 31.25');expect(html).toContain('&lt;img');expect(html).not.toMatch(/<(img|iframe|a|script)\b/);expect(html).not.toMatch(/<details[^>]* open/);expect(html).toContain('链接消息说明');expect(html).toContain('位置消息说明');
});
