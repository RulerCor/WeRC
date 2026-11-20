import type {MomentDetailsData} from '../pages/MomentDetails';

export interface MomentRow {
  anchor:{id:string;row_id:string;username:string};
  content_status:string;
  content:null|{details?:MomentDetailsData;timestamp:number;text:string;content_type:number|null;media_count:number;cached_like_count:number;cached_comment_count:number};
}
export interface RelatedInteraction {
  kind:'like'|'comment';
  reason:'my_post'|'reply_to_me';
  value:MomentDetailsData['comments'][number];
}

/** Exact identities only: writing a nickname or @name in prose is not a mention. */
export function relatedMomentInteractions(row:MomentRow,self:string):RelatedInteraction[]{
  if(!self||!row.content?.details)return [];
  const own=row.anchor.username===self;
  return [
    ...row.content.details.likes.filter(v=>own&&v.username&&v.username!==self&&v.deleted!==true).map(value=>({kind:'like' as const,reason:'my_post' as const,value})),
    ...row.content.details.comments.filter(v=>v.username&&v.username!==self&&v.deleted!==true&&(own||v.reply_username===self)).map(value=>({kind:'comment' as const,reason:own?'my_post' as const:'reply_to_me' as const,value})),
  ];
}
