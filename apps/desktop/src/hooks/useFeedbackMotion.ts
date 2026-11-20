import { useCallback, useEffect, useRef, useState } from 'react';
import { motionAllowed } from '../lib/motion';

export function useFeedbackMotion() {
  const pointer = useRef(false);
  useEffect(() => {
    const onPointer = () => { pointer.current = true; };
    const onKey = () => { pointer.current = false; };
    window.addEventListener('pointerdown', onPointer, true);
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.removeEventListener('pointerdown', onPointer, true);
      window.removeEventListener('keydown', onKey, true);
    };
  }, []);
  return useCallback(() => pointer.current && motionAllowed(), []);
}

// 只保留非敏感提示的退出画面；令牌等敏感内容不能使用此钩子。
export function useFeedbackPresence(value: string | null) {
  const canAnimate = useFeedbackMotion();
  const [state, setState] = useState({ source: value, value, animated: false });
  if (state.source !== value) {
    const animated = canAnimate();
    setState({ source: value, value: value ?? (animated ? state.value : null), animated });
  }
  const exiting = state.source === null && state.value !== null;
  const finish = useCallback(() => {
    setState(current => current.source === null ? { ...current, value: null } : current);
  }, []);
  useEffect(() => {
    if (!exiting) return;
    const preference = window.matchMedia('(prefers-reduced-motion: reduce)');
    const stop = () => { if (preference.matches) finish(); };
    const onKey = () => finish();
    if (preference.matches) { finish(); return; }
    // transitionend 在窗口隐藏或过渡被打断时不一定触发。
    const timer = window.setTimeout(finish, 120);
    preference.addEventListener('change', stop);
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.clearTimeout(timer);
      preference.removeEventListener('change', stop);
      window.removeEventListener('keydown', onKey, true);
    };
  }, [exiting, finish]);
  return { value: state.value, exiting, animated: state.animated, finish };
}
