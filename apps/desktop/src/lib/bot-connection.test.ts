import {describe,it,expect} from 'vitest';
import {botScopes,readScopes,botConnection,mcpConnection} from './bot-connection';
describe('Bot一次授权连接',()=>{
  it('收发预设包含执行和媒体，旧只读不升级，也不附加朋友圈发布',()=>{
    expect(botScopes).toContain('messages:execute');expect(botScopes).toContain('media:write');
    expect(readScopes).not.toContain('messages:execute');expect(botScopes).not.toContain('moments:publish');
  });
  it('默认 Bot 令牌包含三项通讯录权限，不扩大只读预设',()=>{
    for(const scope of ['group-names:read','friends:read','directory:read']){
      expect(botScopes).toContain(scope);
      expect(readScopes).not.toContain(scope);
    }
    expect(botScopes).not.toContain('wechat:settings:write');
  });
  it('从当前服务地址生成配置，凭据仅在headers，两个流游标不混用',()=>{
    const config=botConnection('http://127.0.0.1:6101/','a b','owned-test-token');
    expect(config.messages_url).toBe('http://127.0.0.1:6101/api/v1/messages/native/stream?account_id=a%20b');
    expect(config.events_url).toContain('/events/stream?');expect(config.mcp.url).toBe('http://127.0.0.1:6101/mcp');
    expect(config.messages_url).not.toContain('owned-test-token');
    expect(mcpConnection('http://127.0.0.1:6101','owned',true).mcpServers.WeRC.url).toMatch(/\/mcp\/read$/);
  });
});
