import {describe,it,expect} from 'vitest';
import {textByteLength,validSendText} from './send-text';
describe('native text byte budget',()=>{
  it('counts UTF8 across ASCII, Chinese and astral characters without truncation',()=>{
    expect(textByteLength('a中😀')).toBe(8);
    for(const value of ['x'.repeat(10000),'中'.repeat(3333)+'x','😀'.repeat(2500)])expect(validSendText(value)).toBe(true);
    for(const value of ['x'.repeat(10001),'中'.repeat(3334),'😀'.repeat(2500)+'x',' \n','a\0b'])expect(validSendText(value)).toBe(false);
  });
});
