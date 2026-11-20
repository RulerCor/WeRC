import {afterEach, expect, it, vi} from 'vitest';
import {waitForOnlineRead} from './busy-read';
afterEach(()=>vi.useRealTimers());
const busy = () => Object.assign(new Error('来源正在读取'), {code:'BUSY'});
it('waits for a contending source read and returns the successful result', async()=>{
  vi.useFakeTimers();
  const read=vi.fn().mockRejectedValueOnce(busy()).mockRejectedValueOnce(busy()).mockResolvedValue({rows:[1]});
  const result=waitForOnlineRead(read,()=>false);
  await vi.runAllTimersAsync();
  expect(await result).toEqual({rows:[1]});expect(read).toHaveBeenCalledTimes(3);
});
it('does not retry source replacement or authorization errors', async()=>{
  const error=Object.assign(new Error('source changed'),{code:'CONNECTION_CHANGED'});
  const read=vi.fn().mockRejectedValue(error);
  await expect(waitForOnlineRead(read,()=>false)).rejects.toBe(error);expect(read).toHaveBeenCalledTimes(1);
});
it('stops waiting after five seconds and retains the original BUSY error', async()=>{
  vi.useFakeTimers();const error=busy(),read=vi.fn().mockRejectedValue(error);
  const result=expect(waitForOnlineRead(read,()=>false)).rejects.toBe(error);
  await vi.runAllTimersAsync();await result;expect(read).toHaveBeenCalledTimes(26);
});
it('does not start another read after cancel or account switch', async()=>{
  vi.useFakeTimers();let canceled=false;const read=vi.fn().mockRejectedValue(busy());
  const result=waitForOnlineRead(read,()=>canceled);
  await vi.advanceTimersByTimeAsync(0);canceled=true;await vi.runAllTimersAsync();
  expect(await result).toBeUndefined();expect(read).toHaveBeenCalledTimes(1);
});
it('does not call IPC when already canceled', async()=>{
  const read=vi.fn();expect(await waitForOnlineRead(read,()=>true)).toBeUndefined();expect(read).not.toHaveBeenCalled();
});
it('allows the setup wizard a longer bounded wait while the default stays five seconds', async()=>{
  vi.useFakeTimers();const error=busy(),read=vi.fn().mockRejectedValue(error);
  const result=expect(waitForOnlineRead(read,()=>false,30000)).rejects.toBe(error);
  await vi.advanceTimersByTimeAsync(5200);expect(read.mock.calls.length).toBeGreaterThan(26);
  await vi.runAllTimersAsync();await result;expect(read).toHaveBeenCalledTimes(151);
});
