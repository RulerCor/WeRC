export type HistoryQuery = ReturnType<typeof historyQuery>;
export interface HistoryFilters { conversation: string; sender: string; type: string; search: string; from: string; until: string }
export const emptyHistoryFilters: HistoryFilters = { conversation: '', sender: '', type: '', search: '', from: '', until: '' };
export function historyQuery(filters: HistoryFilters, cursor: string | null = null) {
  function timestamp(value: string) {
    if (!value) return null;
    const result = Math.floor(new Date(value).getTime() / 1000);
    if (!Number.isSafeInteger(result) || result < 0) throw new Error('日期范围无效。');
    return result;
  }
  const from = timestamp(filters.from), until = timestamp(filters.until);
  if (from !== null && until !== null && from >= until) throw new Error('结束时间必须晚于开始时间。');
  const type = filters.type === '' ? null : Number(filters.type);
  if (type !== null && (!/^\d+$/.test(filters.type) || !Number.isSafeInteger(type))) throw new Error('消息类型必须是非负安全整数。');
  return { conversation_id: filters.conversation || null, sender_id: filters.sender || null, message_type: type, search: filters.search || null, from, until, cursor, limit: 50 };
}
