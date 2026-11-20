export interface ConversationQuery {
  from_timestamp?: number;
  until_timestamp?: number;
  message_types?: number[];
  app_types?: number[];
  text?: string;
  server_id?: string;
  sender_username?: string;
}

export const messageTypeChoices = [
  ['', '全部消息'], ['1', '文字'], ['3', '图片'], ['34', '语音'], ['43,62', '视频'],
  ['47', '表情'], ['49', '全部应用卡片'], ['app:6', '文件'], ['app:5', '链接'], ['app:3', '音乐'], ['app:4', '分享视频'], ['app:33,36', '小程序'], ['app:19', '合并聊天记录'], ['app:57', '引用回复'], ['app:2000,2001', '转账与红包卡片'], ['37', '好友验证'], ['42', '名片'], ['48', '位置'], ['50', '通话记录'], ['10000,10002', '系统消息'],
] as const;

export function queryFields(query?:ConversationQuery|null) {
  const date=(seconds:number|undefined,last=false)=>{
    if(seconds===undefined)return '';
    const value=new Date(seconds*1000-(last?1:0));
    if(!Number.isFinite(value.getTime()))return '';
    return `${value.getFullYear().toString().padStart(4,'0')}-${(value.getMonth()+1).toString().padStart(2,'0')}-${value.getDate().toString().padStart(2,'0')}`;
  };
  const values=query?.app_types?.length?`app:${query.app_types.join(',')}`:query?.message_types?.join(',')??'';
  return {from:date(query?.from_timestamp),through:date(query?.until_timestamp,true),text:query?.text??'',sender:query?.sender_username??'',serverId:query?.server_id??'',types:messageTypeChoices.find(([choice])=>choice===values)?.[0]??''};
}

// Calendar arithmetic uses the user's local timezone, including daylight-saving
// transitions. Adding 86400 seconds would silently shift the end of some days.
export function calendarBoundary(value: string, nextDay = false): number | undefined {
  if (!value) return undefined;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) throw new Error('请选择有效日期');
  const [year, month, day] = value.split('-').map(Number);
  if (year < 1970 || year > 9998) throw new Error('日期应在1970至9998年之间');
  const date = new Date(year, month - 1, day);
  if (date.getFullYear() !== year || date.getMonth() !== month - 1 || date.getDate() !== day) throw new Error('请选择有效日期');
  if (nextDay) date.setDate(date.getDate() + 1);
  const seconds = Math.floor(date.getTime() / 1000);
  if (!Number.isSafeInteger(seconds) || seconds < 0 || seconds > 253402300800) throw new Error('日期超出可读取范围');
  return seconds;
}

export function makeConversationQuery(from: string, through: string, types: string, text: string, serverId = '', sender = ''): ConversationQuery {
  const from_timestamp = calendarBoundary(from), until_timestamp = calendarBoundary(through, true);
  if (from_timestamp !== undefined && until_timestamp !== undefined && from_timestamp >= until_timestamp) throw new Error('结束日期不能早于开始日期');
  const needle = text.trim();
  if (new TextEncoder().encode(needle).length > 256 || needle.includes('\0')) throw new Error('搜索文字最多256个UTF-8字节');
  if (!messageTypeChoices.some(([value]) => value === types)) throw new Error('请选择有效消息类型');
  const id = serverId.trim();
  const sender_username = sender.trim();
  if (new TextEncoder().encode(sender_username).length > 256 || /[\u0000-\u001f\u007f-\u009f]/u.test(sender_username)) throw new Error('发送者账号标识无效');
  if (id && (!/^-?[1-9]\d{0,18}$/.test(id) || BigInt(id) < -9223372036854775808n || BigInt(id) > 9223372036854775807n)) throw new Error('请填写原始服务器消息ID，不要使用本地ID');
  return {
    ...(from_timestamp === undefined ? {} : {from_timestamp}),
    ...(until_timestamp === undefined ? {} : {until_timestamp}),
    ...(types.startsWith('app:') ? {message_types:[49],app_types:types.slice(4).split(',').map(Number)} : types ? {message_types: types.split(',').map(Number)} : {}),
    ...(needle ? {text: needle} : {}),
    ...(id ? {server_id: id} : {}),
    ...(sender_username ? {sender_username} : {}),
  };
}

export function queryActive(query?: ConversationQuery | null): boolean {
  return Boolean(query && (query.from_timestamp !== undefined || query.until_timestamp !== undefined || query.text || query.server_id || query.sender_username || query.message_types?.length || query.app_types?.length));
}

export function querySummary(query: ConversationQuery): string {
  const date = (seconds: number) => new Date(seconds * 1000).toLocaleDateString();
  const labels = [];
  if (query.from_timestamp !== undefined) labels.push(`${date(query.from_timestamp)}起`);
  if (query.until_timestamp !== undefined) labels.push(`至${date(query.until_timestamp - 1)}`);
  if (query.app_types?.length) labels.push(messageTypeChoices.find(([v])=>v===`app:${query.app_types!.join(',')}`)?.[1]??'指定卡片类型');
  else if (query.message_types?.length) labels.push(messageTypeChoices.find(([v]) => v === query.message_types?.join(','))?.[1] ?? '指定类型');
  if (query.text) labels.push(`含“${query.text}”`);
  if (query.server_id) labels.push(`消息ID ${query.server_id}`);
  if (query.sender_username) labels.push(`发送者 ${query.sender_username}`);
  return labels.join(' · ');
}
