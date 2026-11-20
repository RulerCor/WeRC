import {describe,it,expect} from 'vitest';
import {calendarBoundary,makeConversationQuery,querySummary,queryFields} from './conversation-query';
import {localMinute,scheduledTimestamp} from './send-schedule';
describe('source query and local plans',()=>{
  it('restores the exact displayed query when returning from a located message',()=>{
    const query=makeConversationQuery('2026-03-08','2026-11-01','app:33,36','原条件','9223372036854775807','owned-sender');
    expect(queryFields(query)).toEqual({from:'2026-03-08',through:'2026-11-01',types:'app:33,36',text:'原条件',serverId:'9223372036854775807',sender:'owned-sender'});
  });
  it('keeps full signed IDs and validates combinations',()=>{
    expect(makeConversationQuery('','','app:33,36','', '9223372036854775807')).toEqual({message_types:[49],app_types:[33,36],server_id:'9223372036854775807'});
    expect(()=>makeConversationQuery('','','','', '9223372036854775808')).toThrow();
    expect(()=>makeConversationQuery('2026-11-12','2026-11-11','','')).toThrow();
    expect(()=>makeConversationQuery('','','','中'.repeat(86))).toThrow();
    expect(querySummary({message_types:[49],app_types:[6]})).toBe('文件');
  });
  it('includes the entire last calendar day and rejects rolled dates',()=>{
    expect(calendarBoundary('2026-11-12',true)).toBe(calendarBoundary('2026-11-13'));
    expect(()=>calendarBoundary('2026-02-30')).toThrow();
  });
  it('does not approve past, expired or invalid local plan times',()=>{
    const now=Math.floor(Date.now()/60000)*60;
    const value=localMinute(now+120);
    expect(scheduledTimestamp(value,now+180,now)).toBe(now+120);
    expect(scheduledTimestamp(value,now+120,now)).toBeNull();
    expect(scheduledTimestamp(value,now+180,now+120)).toBeNull();
    expect(scheduledTimestamp('2026-02-30T12:00',9999999999,0)).toBeNull();
  });
});
