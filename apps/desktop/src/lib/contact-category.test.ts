import {describe,expect,it} from 'vitest';
import {contactCategory,isGroupAccount,isPrivateAccount,type ContactCategoryFields} from './contact-category';
describe('local contact categories',()=>{
  const row=(username:string,fields:Partial<ContactCategoryFields>={}):ContactCategoryFields=>({anchor:{username},...fields});
  it('requires cache evidence and keeps helpers and absent fields unknown',()=>{
    expect(isPrivateAccount('gh_owned')).toBe(false);
    expect(isPrivateAccount('owned@im.chatroom')).toBe(false);
    expect(isGroupAccount('owned@im.chatroom')).toBe(true);
    expect(contactCategory(row('owned'))).toBe('other');
    expect(contactCategory(row('owned',{local_type:1}))).toBe('friend');
    expect(contactCategory(row('filehelper',{local_type:1}))).toBe('other');
    expect(contactCategory(row('owned',{local_type:3,remark:'历史',contact_flag:0}))).toBe('former');
    expect(contactCategory(row('owned',{local_type:3,remark:'历史',contact_flag:4}))).toBe('other');
    expect(contactCategory(row('owned',{local_type:3,remark:'历史'}))).toBe('other');
  });
  it('preserves deletion, group and business precedence without signed 32bit truncation',()=>{
    expect(contactCategory(row('owned@im.chatroom'))).toBe('enterprise_group');
    expect(contactCategory(row('owned@chatroom',{contact_flag:8}))).toBe('group');
    expect(contactCategory(row('owned@openim'))).toBe('enterprise');
    expect(contactCategory(row('owned',{local_type:1,contact_flag:2**40+8}))).toBe('blocked');
    expect(contactCategory(row('owned',{contact_flag:-1}))).toBe('other');
    expect(contactCategory(row('gh_owned',{business_type:0}))).toBe('subscription');
    expect(contactCategory(row('gh_owned',{business_type:1}))).toBe('official_service');
    expect(contactCategory(row('gh_owned',{business_type:9}))).toBe('service');
    expect(contactCategory(row('gh_owned',{business_type:1,delete_flag:1}))).toBe('deleted');
  });
});
