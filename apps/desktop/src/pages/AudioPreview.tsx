import { useEffect, useRef, useState } from 'react';
import { command } from '../lib/desktop';

export function AudioPreview({ accountId, assetId, disabled, onError }: { accountId: string; assetId: string; disabled: boolean; onError: (message: string) => void }) {
  const [url, setUrl] = useState('');
  const [loading, setLoading] = useState(false);
  const current = useRef('');
  const generation = useRef(0);
  const audio = useRef<HTMLAudioElement>(null);
  function clear() {
    generation.current += 1;
    audio.current?.pause();
    if (current.current) URL.revokeObjectURL(current.current);
    current.current = '';
    setUrl(''); setLoading(false);
  }
  useEffect(() => () => { generation.current += 1; if (current.current) URL.revokeObjectURL(current.current); current.current = ''; }, []);
  async function load() {
    const expected = ++generation.current;
    setLoading(true);
    let values: number[] = [];
    let bytes: Uint8Array<ArrayBuffer> | null = null;
    try {
      values = await command<number[]>('read_audio_preview', { accountId, id: assetId });
      if (generation.current !== expected) return;
      bytes = new Uint8Array(values);
      const next = URL.createObjectURL(new Blob([bytes], { type: 'audio/wav' }));
      if (current.current) URL.revokeObjectURL(current.current);
      current.current = next; setUrl(next);
    } catch (error) { if (generation.current === expected) onError(error instanceof Error ? error.message : '无法读取音频。'); }
    finally { values.fill(0); bytes?.fill(0); if (generation.current === expected) setLoading(false); }
  }
  return <div className="audio-preview">
    {url ? <><audio ref={audio} controls preload="none" src={url} aria-label="本地音频预听" onError={() => { clear(); onError('音频播放失败；未发送消息。'); }}/><button type="button" className="button" onClick={clear}>关闭预听</button></> : <button type="button" className="button" disabled={disabled || loading} onClick={load}>{loading ? '读取中' : '预听音频'}</button>}
  </div>;
}
