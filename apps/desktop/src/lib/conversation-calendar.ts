import {calendarBoundary} from './conversation-query';

export interface CalendarDay {from_timestamp:number;until_timestamp:number}
export function calendarMonth(value:string):{days:CalendarDay[];offset:number} {
  if(!/^\d{4}-\d{2}$/.test(value))throw new Error('请选择有效月份');
  const [year,month]=value.split('-').map(Number);
  if(year<1970||year>9998||month<1||month>12)throw new Error('月份超出可读取范围');
  const count=new Date(year,month,0).getDate();
  const days=Array.from({length:count},(_,i)=>{
    const date=`${value}-${String(i+1).padStart(2,'0')}`;
    return {from_timestamp:calendarBoundary(date)!,until_timestamp:calendarBoundary(date,true)!};
  });
  return {days,offset:(new Date(year,month-1,1).getDay()+6)%7};
}
export function shiftMonth(value:string,delta:number):string {
  const [year,month]=value.split('-').map(Number);
  const date=new Date(year,month-1+delta,1);
  return `${date.getFullYear().toString().padStart(4,'0')}-${String(date.getMonth()+1).padStart(2,'0')}`;
}
export function currentMonth():string {
  const date=new Date();
  return `${date.getFullYear()}-${String(date.getMonth()+1).padStart(2,'0')}`;
}
