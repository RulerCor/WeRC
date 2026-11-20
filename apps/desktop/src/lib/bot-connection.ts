export const readScopes = ['wechat:settings:read', 'accounts:read', 'connections:read', 'sources:read', 'contacts:read', 'sessions:read', 'moments:read', 'media:read', 'operations:read', 'capabilities:read', 'events:read', 'messages:read', 'openapi:read'];
export const botScopes = [...readScopes, 'group-names:read', 'friends:read', 'directory:read', 'media:write', 'operations:create', 'messages:execute', 'messages:send', 'messages:send_voice', 'messages:send_image', 'messages:send_file', 'messages:send_video', 'messages:send_emoticon'];

export function botConnection(address: string, accountId: string, secret: string) {
  const base = address.replace(/\/$/, '');
  return {
    platform: 'WeRC',
    api_url: `${base}/api/v1`,
    account_id: accountId,
    headers: { Authorization: `Bearer ${secret}` },
    messages_url: `${base}/api/v1/messages/native/stream?account_id=${encodeURIComponent(accountId)}`,
    events_url: `${base}/api/v1/events/stream?account_id=${encodeURIComponent(accountId)}`,
    mcp: { type: 'http', url: `${base}/mcp`, headers: { Authorization: `Bearer ${secret}` } },
  };
}

export function mcpConnection(address: string, secret: string, readOnly = false) {
  return { mcpServers: { WeRC: { type: 'http', url: `${address.replace(/\/$/, '')}/mcp${readOnly ? '/read' : ''}`, headers: { Authorization: `Bearer ${secret}` } } } };
}
export function mcpStdioConnection(address:string,secret:string,readOnly=false, executable='werc-mcp-stdio'){
  return {mcpServers:{WeRC:{command:executable,args:[],env:{WERC_MCP_URL:`${address.replace(/\/$/,'')}/mcp${readOnly?'/read':''}`,WERC_API_TOKEN:secret}}}};
}
