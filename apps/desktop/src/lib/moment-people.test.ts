import {describe,it,expect} from 'vitest';
import {relatedMomentInteractions,type MomentRow} from './moment-people';
const interaction=(username:string,reply_username='',deleted:boolean|null=null)=>({username,reply_username,deleted,nickname:'同名用户',text:'@我的昵称',timestamp:null,id:'1',id64:null,reply_id:null,reply_id64:null,image_count:null,emoji_count:null});
function post(author:string):MomentRow{return {anchor:{id:'100',row_id:'1',username:author},content_status:'parsed',content:{timestamp:1,text:'自动化测试',content_type:2,media_count:0,cached_like_count:3,cached_comment_count:4,details:{title:'',description:'',link:null,source_name:'',location:{},media:[],likes:[interaction('other'),interaction('me'),interaction('deleted','',true)],comments:[interaction('other'),interaction('replier','me'),interaction('me'),interaction('deleted','me',true)]}}};}
describe('exact cached moment interactions',()=>{
  it('includes other people on my post and explicit replies; excludes self, deleted and nickname guesses',()=>{
    expect(relatedMomentInteractions(post('me'),'me').map(r=>r.value.username)).toEqual(['other','other','replier']);
    const others=relatedMomentInteractions(post('third-person'),'me');
    expect(others).toHaveLength(1);expect(others[0].reason).toBe('reply_to_me');expect(others[0].value.username).toBe('replier');
    expect(relatedMomentInteractions(post('me'),'')).toEqual([]);
  });
});
