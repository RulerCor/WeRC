export const MAX_TEXT_BYTES = 10000;
export const textByteLength = (text: string) => new TextEncoder().encode(text).length;
export const validSendText = (text: string) => text.trim().length > 0 && !text.includes('\0') && textByteLength(text) <= MAX_TEXT_BYTES;
