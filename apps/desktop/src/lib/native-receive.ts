import type {CardContent} from '../components/MessageCard';
export interface ReceivedEnvelope {sender:string;receiver:string;conversation:string}
export interface NativeRow {position:number;card?:CardContent|null;payload:{id:string;sender_id:string;target_id:string;route?:ReceivedEnvelope;observed_at:string;message:{text:string;native_message_id:string;message_type?:number;metadata?:string}}}
export function conversationName(target:string):string {
  if(target==='filehelper')return '文件传输助手';
  const name=target.endsWith('@chatroom')?'群聊':'会话';
  return `${name} · ${target.length>22?`${target.slice(0,12)}…${target.slice(-8)}`:target}`;
}
export function observationDirection(row:NativeRow):string {
  const {route,sender_id}=row.payload;
  if(!route)return '原生观测';
  if(route.sender===sender_id&&route.receiver===sender_id)return '自己的设备';
  if(route.sender===sender_id)return '我发出的消息';
  return route.conversation.endsWith('@chatroom')?'群内消息':'收到的消息';
}
