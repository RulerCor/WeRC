export const contactCategories = [
  ['friend','好友标记'],['group','群聊'],['enterprise','企业账号'],['enterprise_group','企业群'],
  ['subscription','订阅号'],['official_service','服务号'],['service','其他公众号'],
  ['former','历史 / 非好友线索'],['blocked','黑名单标记'],['deleted','删除标记'],['other','其他联系人'],
] as const;
export type ContactCategory = typeof contactCategories[number][0];
export const isGroupAccount=(username:string)=>username.endsWith('@chatroom')||username.endsWith('@im.chatroom');
export const isPrivateAccount=(username:string)=>!isGroupAccount(username)&&!username.startsWith('gh_');
export interface ContactCategoryFields {
  anchor:{username:string};local_type?:number|null;delete_flag?:number|null;
  contact_flag?:number|null;business_type?:number|null;alias?:string|null;remark?:string|null;
}
const helpers = new Set(['filehelper','fmessage','medianote','floatbottle','newsapp','weixin','notifymessage','brandsessionholder','qqmail','weibo','feedsapp','voipapp','lbsapp','shakeapp','readerapp','blogapp','facebookapp','masssendapp','meishiapp','linkedinplugin','qqsync']);
export function contactCategory(row:ContactCategoryFields):ContactCategory {
  const username=row.anchor.username;
  if(row.delete_flag!=null&&row.delete_flag!==0)return 'deleted';
  if(username.endsWith('@im.chatroom'))return 'enterprise_group';
  if(username.endsWith('@chatroom'))return 'group';
  if(username.startsWith('gh_'))return row.business_type===0?'subscription':row.business_type===1?'official_service':'service';
  if(row.contact_flag!=null&&Number.isSafeInteger(row.contact_flag)&&row.contact_flag>=0&&(BigInt(row.contact_flag)&8n)!==0n)return 'blocked';
  if(username.endsWith('@openim'))return 'enterprise';
  if(row.local_type===1&&!helpers.has(username))return 'friend';
  if(row.local_type===3&&row.contact_flag!=null&&row.contact_flag!==4&&(row.alias||row.remark))return 'former';
  return 'other';
}
export function contactCategoryLabel(row:ContactCategoryFields):string {
  return contactCategories.find(([key])=>key===contactCategory(row))![1];
}
