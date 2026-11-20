import type { SetupItem, SetupRun } from '../hooks/useOnboarding';

export function setupProgress(run:SetupRun,keyPhase:boolean) {
  const settled=(item:SetupItem)=>item.skipped||Boolean(item.error)||(keyPhase?item.key_ready:Boolean(item.source));
  const processed=run.items.filter(settled).length;
  const currentIndex=run.phase==='failed'?run.items.findIndex(item=>!item.skipped&&Boolean(item.error)):run.items.findIndex(item=>!settled(item));
  return {processed,total:run.items.length,currentIndex,current:run.items[currentIndex],fraction:run.items.length?processed/run.items.length:0};
}
