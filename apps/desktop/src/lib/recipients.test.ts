import {describe,it,expect} from 'vitest';
import {recipientAvailable} from './recipients';

describe('recipient expiry',()=>{
  it('expires exactly at the lease deadline without changing the selected ID',()=>{
    const r={target_id:'owned-group@chatroom',display_name:'owned',expires_at:300};
    expect(recipientAvailable(r,299999)).toBe(true);
    expect(recipientAvailable(r,300000)).toBe(false);
    expect(r.target_id).toBe('owned-group@chatroom');
  });
  it('builtin availability is independent of lease time, connection is checked by the caller',()=>{
    expect(recipientAvailable({target_id:'filehelper',display_name:'文件传输助手',expires_at:null},Number.MAX_SAFE_INTEGER)).toBe(true);
  });
});
