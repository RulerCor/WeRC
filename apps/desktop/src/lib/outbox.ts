export type MomentActionKind='like'|'comment'|'unlike'|'delete'|'reply';
export interface MomentReplyTarget {username:string;id?:string;id64?:string}
export interface MomentAction {sns_id:string;action:MomentActionKind;reply?:MomentReplyTarget}
export const momentActionLabels:Record<MomentActionKind,string>={like:'点赞',comment:'评论',unlike:'取消赞',delete:'删除本人动态',reply:'回复评论'};
import type {MomentImageAsset} from '../pages/MomentImagePreview';

export type SendState = 'awaiting_approval' | 'queued' | 'submitting' | 'unknown' | 'server_accepted' | 'recipient_confirmed' | 'rejected' | 'canceled' | 'expired';
export interface SendStatus {
  metadata: { schema_version?:number; id: string; account_id: string; creator_id: string; request_digest: string; context_digest: string; connection_revision: number; expires_at: number; created_at: number };
  scheduled_at?: number | null;
  state: SendState; approval_expires_at: number | null; attempt_id: string | null; evidence: string | null; updated_at: number;
}
export interface EmoticonSendAsset {file:{asset_id:string;sha256:string;size:number;file_name:string};info:{width:number;height:number;duration_ms:number;frames:number}}
export interface VideoSendAsset {file:{asset_id:string;sha256:string;size:number;file_name:string};info:{width:number;height:number;duration_ms:number;audio:boolean}}
export interface SendPreview {
  local_record?:{state:'written'|'failed'|'unknown';observed_at:number|null}|null;
  schedule_before?:number|null;
  status: SendStatus;
  payload: { text: string; moment?:{video?:VideoSendAsset;interaction?:MomentAction;visibility:'private'|'public';images?:MomentImageAsset[]}; quote?:{server_id:string;text:string;sha256:string}; mentions?:string[]; video?:VideoSendAsset; emoticon?:EmoticonSendAsset; file?: {asset_id:string;sha256:string;size:number;file_name:string}; image?: {asset_id:string;source_sha256:string;png_sha256:string;width:number;height:number}; voice?: {asset_id:string;source_sha256:string;silk_sha256:string;codec_sha256:string;duration_ms:number}; context: { sender_id: string; target_id: string; target_display_name: string; execution_mode: 'native' | 'visible_automation' | 'synthetic'; identity_verified: boolean; target_verified: boolean; epoch: string } };
}
export const sendStates: Record<SendState, string> = { awaiting_approval: '待本机批准', queued: '已批准 / 排队中', submitting: '提交中', unknown: '结果未知 / 不会重发', server_accepted: '服务器已接受', recipient_confirmed: '接收端已确认', rejected: '提交前已拒绝', canceled: '已取消', expired: '批准已过期' };
