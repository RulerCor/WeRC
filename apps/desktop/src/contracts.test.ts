import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
const root = new URL('../../../', import.meta.url);
const api = JSON.parse(readFileSync(new URL('contracts/openapi.json', root), 'utf8'));
const requirements = JSON.parse(readFileSync(new URL('docs/requirements/traceability.yaml', root), 'utf8'));
describe('新工程的诚实状态与契约', () => {
  it('保留历史需求与新增原生语音，不固定台账总数', () => {
    const ids = requirements.requirements.map((r: { id: string }) => r.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids).toEqual(expect.arrayContaining(['ACC-01', 'SND-09', 'SND-10', 'NEW-05']));
    expect(ids.length).toBeGreaterThanOrEqual(211);
  });
  it('客户端执行权限区分自动排队与草稿，200不声明送达', () => {
    for (const path of ['/api/v1/moments/publish']) {
      expect(api.paths[path].post.responses['501']).toBeDefined();
      expect(api.paths[path].post.responses['200'].description).toContain('未发布');
      expect(api.paths[path].post.requestBody.content['application/json'].schema.properties.visibility.enum).toEqual(['private','public']);
      expect(api.paths[path].post.description).toContain('moments:publish');
    }
    const text = api.paths['/api/v1/messages/send-text'].post;
    expect(text.responses['501']).toBeDefined();
    expect(text.responses['200'].description).toContain('同一任务当前状态');
    expect(text.responses['200'].description).toContain('不自动提交/重发');
    const wait=text.requestBody.content['application/json'].schema.properties.wait_seconds;
    expect(wait).toMatchObject({type:'integer',minimum:0,maximum:120,default:30});
    expect(text.description).toContain('messages:execute');
    expect(text.description).toContain('旧仅草稿令牌仍待本机批准');
    expect(api.components.schemas.SendStatus.properties.state.enum).toContain('awaiting_approval');
    expect(text.description).toContain('operations:create');
    expect(api.components.schemas.SendStatus.properties.state.enum).toContain('unknown');
    const voice=api.paths['/api/v1/messages/send-voice'].post;
    expect(voice.responses['200'].description).toContain('旧仅草稿令牌保持待批准');
    expect(voice.description).toContain('messages:send_voice');
    expect(voice.description).toContain('operations:create');
    expect(voice.description).toContain('Unknown不重发');
    const image=api.paths['/api/v1/messages/send-image'].post;
    expect(image.description).toContain('messages:send_image');
    expect(image.description).toContain('Unknown不重发');
    expect(image.responses['200'].description).toContain('旧仅草稿令牌保持待批准');
    const action=api.paths['/api/v1/moments/interact'].post;
    expect(action.description).toContain('moments:publish');
    expect(action.description).toContain('messages:execute');
    expect(action.description).toContain('原生bool不作为服务端回执');
    expect(action.responses['200'].description).toContain('schema11');
    expect(Object.keys(api.paths).some(path => path.includes('/approve'))).toBe(false);
  });
  it('只公开最小健康检查，业务接口使用 Bearer', () => {
    expect(api.security).toEqual([{ bearerAuth: [] }]);
    expect(api.paths['/health'].get.security).toEqual([]);
    expect(api.servers[0].url).toContain('127.0.0.1');
  });
});
