export const FIRST_RUN_KEY = 'werc.first-run.v1';
export type FirstRunStep = 'wechat' | 'prepare' | 'bot' | 'done';
export function readFirstRun(): FirstRunStep | 'complete' {
  try {
    const value = localStorage.getItem(FIRST_RUN_KEY);
    return ['prepare', 'bot', 'done', 'complete'].includes(value ?? '') ? value as FirstRunStep | 'complete' : 'wechat';
  } catch { return 'wechat'; }
}
export function saveFirstRun(step: FirstRunStep | 'complete') {
  try { localStorage.setItem(FIRST_RUN_KEY, step); } catch { /* The current session can still finish. */ }
}
