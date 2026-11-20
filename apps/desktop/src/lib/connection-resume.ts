export interface ResumeConnection {accountId:string;connect:boolean;apiPort:number|null}
const key='werc.connection-resume.v1';
export function readResume():ResumeConnection|null{
  try{const value=JSON.parse(localStorage.getItem(key)??'null');return value&&typeof value.accountId==='string'&&typeof value.connect==='boolean'&&(value.apiPort===null||Number.isInteger(value.apiPort)&&value.apiPort>=1024&&value.apiPort<=65535)?value:null;}catch{return null;}
}
export function rememberConnection(accountId:string,connect:boolean){try{const old=readResume();localStorage.setItem(key,JSON.stringify({accountId,connect,apiPort:old?.accountId===accountId?old.apiPort:null}));}catch{/* Optional startup preference. */}}
export function rememberApi(accountId:string,port:number|null){try{const old=readResume();localStorage.setItem(key,JSON.stringify({accountId,connect:old?.accountId===accountId?old.connect:false,apiPort:port}));}catch{/* Optional startup preference. */}}
