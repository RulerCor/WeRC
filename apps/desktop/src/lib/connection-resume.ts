export interface ResumeConnection {accountId:string;connect:boolean;apiPort:number|null}
export const componentRepairHint='WeRC 原生组件不可用。请用正式安装包修复，或完整解压便携包后从解压目录启动。';

export function nativeConnectionErrorText(error:unknown):string{
  const code=typeof error==='string'?error:(error as {code?:string}|null)?.code;
  if(code==='NATIVE_COMPONENT_INVALID')return `${componentRepairHint}（${code}）`;
  return error instanceof Error?error.message:String(error);
}

const key='werc.connection-resume.v1';
export function readResume():ResumeConnection|null{
  try{const value=JSON.parse(localStorage.getItem(key)??'null');return value&&typeof value.accountId==='string'&&typeof value.connect==='boolean'&&(value.apiPort===null||Number.isInteger(value.apiPort)&&value.apiPort>=1024&&value.apiPort<=65535)?value:null;}catch{return null;}
}
export function rememberConnection(accountId:string,connect:boolean){try{const old=readResume();localStorage.setItem(key,JSON.stringify({accountId,connect,apiPort:old?.accountId===accountId?old.apiPort:null}));}catch{/* Optional startup preference. */}}
export function rememberApi(accountId:string,port:number|null){try{const old=readResume();localStorage.setItem(key,JSON.stringify({accountId,connect:old?.accountId===accountId?old.connect:false,apiPort:port}));}catch{/* Optional startup preference. */}}
