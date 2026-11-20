import {HelpTip} from './HelpTip';

export function LiveSender({username}:{username?:string|null}) {
  return <span className="live-sender"><strong dir="auto">{username??'发送者未解析'}</strong>{username&&<HelpTip label="查看发送者标识"><span dir="auto">{username}</span><p>这是该消息分片记录的发送者账号标识，不是昵称；尚未判定是否为你自己。</p></HelpTip>}</span>;
}
