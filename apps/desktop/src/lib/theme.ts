export type Theme = 'light' | 'dark' | 'system';
const key = 'werc-theme-v1';

export function readTheme(): Theme {
  try { const value=localStorage.getItem(key);return value==='system'||value==='dark'?value:'light'; }
  catch { return 'light'; }
}

export function applyTheme(theme: Theme) {
  document.documentElement.dataset.theme = theme==='system' ? (typeof matchMedia==='function'&&matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light') : theme;
  try { localStorage.setItem(key, theme); }
  catch { /* 本机存储不可用时，主题仍在当前窗口生效。 */ }
}

export function followSystem(theme:Theme):()=>void {
  if(theme!=='system'||typeof matchMedia!=='function')return ()=>{};
  const query=matchMedia('(prefers-color-scheme: dark)');
  const update=()=>applyTheme('system');
  query.addEventListener('change',update);
  return ()=>query.removeEventListener('change',update);
}
