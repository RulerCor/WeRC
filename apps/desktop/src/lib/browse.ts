import type { HistoryQuery } from './history';
import type { HistorySource } from './desktop';
export type ReadMode='realtime'|'on_demand_history'|'imported_snapshot';
export type BrowseDirection='latest'|'older'|'newer'|'around';
export interface ReadContext { mode:ReadMode; account_id:string; connection_revision:string; generation:string; observed_at:string; sources:HistorySource[]; identity_status:string; query_strategy:string;query_metrics?:{authenticated_batches:number;elapsed_ms:number;projection_built:boolean}|null }
export interface BrowseRequest { request_id:string; mode:ReadMode; source_id:string|null; direction:BrowseDirection; anchor_id:string|null; anchor_timestamp:number|null; query:HistoryQuery }
export interface MessageView { schema_version:number;id:string;conversation_id:string;sender_id:string|null;sender_name:string|null;direction:'self'|'peer'|'unknown';identity_status:string;server_id:string;local_id:string;sequence:string;timestamp:number;message_type:string;text:string|null;content_status:string }
export interface ConversationView {id:string;display_name:string;identity_status:string;summary:string|null;timestamp:number;historical_unread_count:string|null}
export interface ConversationPage {data:ConversationView[];next_cursor:string|null;context:ReadContext}
export interface TimelinePage {data:MessageView[];older_cursor:string|null;newer_cursor:string|null;context:ReadContext}
export const messageTypeNames:Record<string,string>={'1':'文字','3':'图片','34':'语音','43':'视频','47':'表情','48':'位置','49':'分享 / 文件','10000':'系统','10002':'系统事件'};
export function messageLabel(type:string) {return messageTypeNames[type]??`未知类型 ${type}`;}
export function safeTime(timestamp:number) {const date=new Date(timestamp*1000);return Number.isFinite(date.getTime())?date.toLocaleString('zh-CN',{hour12:false}):'时间未解析';}
export function baseQuery():HistoryQuery {return {conversation_id:null,sender_id:null,message_type:null,search:null,from:null,until:null,cursor:null,limit:60};}
