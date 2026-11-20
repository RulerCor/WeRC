import { describe, expect, it } from 'vitest';
import { emptyHistoryFilters, historyQuery } from './history';

describe('记录组合查询', () => {
  it('空筛选不伪造身份或日期', () => {
    expect(historyQuery(emptyHistoryFilters)).toEqual({ conversation_id: null, sender_id: null, message_type: null, search: null, from: null, until: null, cursor: null, limit: 50 });
  });
  it('组合条件和游标完整保留，日期转为秒', () => {
    const q = historyQuery({ conversation: 'peer', sender: 'member', type: '34', search: '语音', from: '2026-01-01T00:00:00Z', until: '2026-01-02T00:00:00Z' }, 'cursor');
    expect(q.sender_id).toBe('member'); expect(q.message_type).toBe(34); expect(q.search).toBe('语音'); expect(q.until! - q.from!).toBe(86400); expect(q.cursor).toBe('cursor');
  });
  it('拒绝无效日期、倒序范围和不安全类型', () => {
    for (const patch of [{ from: 'invalid' }, { from: '2026-01-02', until: '2026-01-01' }, { type: '-1' }, { type: '1e3' }, { type: '9007199254740992' }]) expect(() => historyQuery({ ...emptyHistoryFilters, ...patch })).toThrow();
  });
});
