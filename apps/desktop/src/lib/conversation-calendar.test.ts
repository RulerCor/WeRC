import {describe,it,expect} from 'vitest';
import {calendarMonth,shiftMonth} from './conversation-calendar';
import {queryFields} from './conversation-query';
describe('conversation calendar',()=>{
  it('maps leap months and each selected day back to the same date filter',()=>{
    for(const month of ['2024-02','2026-03','2026-11']){
      const result=calendarMonth(month);
      expect(result.days).toHaveLength(month==='2024-02'?29:month==='2026-03'?31:30);
      result.days.forEach((day,i)=>{
        const date=`${month}-${String(i+1).padStart(2,'0')}`;
        expect(queryFields(day).from).toBe(date);expect(queryFields(day).through).toBe(date);
        if(i)expect(result.days[i-1].until_timestamp).toBe(day.from_timestamp);
      });
    }
    expect(calendarMonth('2024-02').offset).toBe(3);
    expect(shiftMonth('2026-01',-1)).toBe('2025-12');
    expect(shiftMonth('2026-12',1)).toBe('2027-01');
    expect(()=>calendarMonth('2026-13')).toThrow();
  });
});
