import {describe,it,expect} from 'vitest';
import {validMentionDraft,validMentionMember} from './mentions';
describe('group mention selection',()=>{
  it('accepts only exact individual member IDs',()=>{
    expect(validMentionMember('member-long_123456789012345')).toBe(true);
    for(const id of [null,'','@all','group@chatroom','a,b','昵称','a '.trimEnd()+' ','x'.repeat(128)])expect(validMentionMember(id)).toBe(false);
  });
  it('binds bounded distinct members and UTF8 body limits',()=>{
    expect(validMentionDraft(['a','b'],'自动化测试')).toBe(true);
    expect(validMentionDraft(['a','a'],'自动化测试')).toBe(false);
    expect(validMentionDraft([], '自动化测试')).toBe(false);
    expect(validMentionDraft(Array.from({length:9},(_,i)=>`member-${i}`),'自动化测试')).toBe(false);
    expect(validMentionDraft(['a'],'中'.repeat(3333)+'x')).toBe(true);
    expect(validMentionDraft(['a'],'中'.repeat(3333)+'xx')).toBe(false);
    expect(validMentionDraft(['a'],' \n')).toBe(false);
    expect(validMentionDraft(['a'],'a\0b')).toBe(false);
  });
});
