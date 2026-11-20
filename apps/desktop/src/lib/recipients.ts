export interface PreparedRecipient {
  target_id: string;
  display_name: string;
  expires_at: number | null;
}
export function recipientAvailable(recipient: PreparedRecipient, now = Date.now()): boolean {
  return recipient.expires_at === null || recipient.expires_at * 1000 > now;
}
