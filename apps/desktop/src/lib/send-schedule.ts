/** Native datetime-local uses the user's current time zone; never assume UTC. */
export function localMinute(timestamp:number):string {
  const date=new Date(timestamp*1000);
  if(!Number.isFinite(date.getTime()))return '';
  const pad=(value:number)=>String(value).padStart(2,'0');
  return `${date.getFullYear()}-${pad(date.getMonth()+1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}
export function scheduledTimestamp(value:string,expiresAt:number,now=Date.now()/1000):number|null {
  if(!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value))return null;
  const seconds=new Date(value).getTime()/1000;
  // Round-trip rejects nonexistent local times when daylight-saving time changes.
  if(!Number.isSafeInteger(seconds)||localMinute(seconds)!==value||seconds<=now||seconds>=expiresAt)return null;
  return seconds;
}
