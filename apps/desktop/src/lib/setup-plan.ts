import type { ClientInstallation, DataSourceCandidate, DataSourceCatalog, DiscoveryReport } from './desktop';

export interface SetupDetection { clients:DiscoveryReport; catalog:DataSourceCatalog; main_process_ids:number[] }
export const MAX_SETUP_DATABASES=512;
// Locale-independent ordering keeps recommendations stable across machines and discovery order.
const compare=(a:string,b:string)=>a<b?-1:a>b?1:0;
const pathKey=(path:string)=>path.replace(/\\/g,'/').toLowerCase();
export function processCandidates(client:ClientInstallation|undefined,mainProcessIds:readonly number[]) {
  return [...new Set(client?.process_ids??[])].filter(id=>Number.isInteger(id)&&id>0&&id<=0xffffffff)
    .sort((a,b)=>Number(mainProcessIds.includes(b))-Number(mainProcessIds.includes(a))||a-b);
}
export function recommendSetup(detection:SetupDetection) {
  const clients=[...detection.clients.installations].sort((a,b)=>{
    const rank=(client:ClientInstallation)=>{const ids=processCandidates(client,detection.main_process_ids);return ids.some(id=>detection.main_process_ids.includes(id))?0:ids.length?1:2;};
    return rank(a)-rank(b)||compare(pathKey(a.executable_path),pathKey(b.executable_path))||compare(a.id,b.id);
  });
  const sources=[...detection.catalog.sources].sort((a,b)=>compare(pathKey(a.account_directory),pathKey(b.account_directory))||compare(a.id,b.id));
  const client=clients[0],source=sources[0];
  return {installation:client?.id??'',source:source?.id??'',pid:client?String(processCandidates(client,detection.main_process_ids)[0]??''):''};
}
// No size/category filtering: native processing reports unsupported and resource-deferred items.
// Do not truncate at the limit; callers must refuse an oversized plan rather than omit files.
export function selectAllDatabases(source:DataSourceCandidate|undefined) {
  return [...new Set(source?.databases.map(db=>db.relative_path)??[])].sort(compare);
}
