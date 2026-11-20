import {validSendText} from './send-text';
export function validMentionMember(value: string | null): value is string {
  return value !== null && /^[A-Za-z0-9_.-]{1,127}$/.test(value);
}
export function validMentionDraft(members: string[], text: string): boolean {
  return members.length > 0 && members.length <= 8 && new Set(members).size === members.length
    && members.every(validMentionMember) && new TextEncoder().encode(members.join(',')).length < 1024
    && validSendText(text);
}
