import { useCallback, useEffect, useRef, useState } from 'react';
import { command, desktopAvailable, type Bootstrap } from '../lib/desktop';
const initial: Bootstrap = {
  current_connection: { account_id: null, revision: '0', selection_required: false, identity_verified: false },
  status: { product: 'WeRC_Mon3trBot', version: '1.1.0', stage: 'stable', api_enabled: false, real_wechat_verified: false },
  accounts: [], tokens: [], capabilities: [], api_address: null, bindings: [], key_statuses: [], audio_assets: [], image_assets: [], file_assets: [], operations: [], history_sources: [],
};
export function useWorkbench() {
  const [data, setData] = useState(initial);
  const [loaded, setLoaded] = useState(false);
  const [completed, setCompleted] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const running = useRef(false);
  const refresh = useCallback(async () => {
    if (desktopAvailable) {setData(await command<Bootstrap>('bootstrap')); setLoaded(true);}
  }, []);
  useEffect(() => { refresh().catch(e => setError(e.message)); }, [refresh]);
  const act = async (action?: () => Promise<unknown>) => {
    if (running.current) return;
    running.current = true;
    setBusy(true);
    setError('');
    try { if (action) await action(); await refresh(); if(action) setCompleted(value=>value+1); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { running.current = false; setBusy(false); }
  };
  return { data, loaded, completed, busy, error, setError, refresh, act };
}
export type Workbench = ReturnType<typeof useWorkbench>;
