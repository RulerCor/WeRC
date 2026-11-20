// Only for preparing an online read. Never use for send/approve/submit commands.
// Return no result after cancellation, including cancellation between attempts.
export async function waitForOnlineRead<T>(read: () => Promise<T>, canceled: () => boolean, timeoutMs = 5000): Promise<T | undefined> {
  const deadline = Date.now() + timeoutMs;
  while (!canceled()) {
    try { return await read(); }
    catch (error) {
      if (canceled()) return undefined;
      if (!(error instanceof Error) || !('code' in error) || error.code !== 'BUSY' || Date.now() >= deadline) throw error;
      await new Promise(resolve => setTimeout(resolve, 200));
    }
  }
  return undefined;
}
