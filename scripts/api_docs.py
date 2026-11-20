"""Build a self-contained offline API reference from the shipping contract."""
from pathlib import Path
import argparse
import hashlib
import html
import json
import re
import shutil

ROOT = Path(__file__).resolve().parent.parent
FILENAME = 'API接口文档.html'
METHODS = {'get', 'post', 'put', 'patch', 'delete', 'options', 'head', 'trace'}
SOURCES = {'openapi.json': ROOT / 'contracts/openapi.json',
           'bot-client.md': ROOT / 'docs/bot-client.md'}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def operations(spec):
    return [(path, method, operation) for path, item in spec['paths'].items()
            for method, operation in item.items() if method in METHODS]


def render(contract, guide):
    spec = json.loads(contract)
    esc = html.escape

    def code(value):
        result = esc(json.dumps(value, ensure_ascii=False, indent=2))
        return re.sub(r'(#/components/schemas/)([A-Za-z0-9_.-]+)',
                      r'<a href="#schema-\2">\1\2</a>', result)

    entries = []
    for index, (path, method, op) in enumerate(operations(spec), 1):
        parameters = spec['paths'][path].get('parameters', []) + op.get('parameters', [])
        sections = [('参数（required 表示必填）', parameters),
                    ('请求体', op.get('requestBody')),
                    ('响应状态与数据结构', op.get('responses')),
                    ('授权', op.get('security', spec.get('security', [])))]
        extra = {key: value for key, value in op.items() if key.startswith('x-')}
        if extra:
            sections.append(('附加约束', extra))
        details = ''.join('<details><summary>' + esc(title) + '</summary><pre>'
                          + code(value) + '</pre></details>'
                          for title, value in sections if value is not None)
        entries.append(f'<article class="endpoint" data-operation="{esc(method.upper()+" "+path, quote=True)}">'
                       f'<h3 id="operation-{index}"><b>{method.upper()}</b> <code>{esc(path)}</code></h3>'
                       f'<p>{esc(op.get("summary", op.get("operationId", "")))}</p>'
                       f'<p>{esc(op.get("description", ""))}</p>{details}</article>')
    schemas = ''.join(f'<details id="schema-{esc(name, quote=True)}" class="schema">'
                      f'<summary>{esc(name)}</summary><pre>{code(value)}</pre></details>'
                      for name, value in spec.get('components', {}).get('schemas', {}).items())
    metadata = json.dumps({'contract_sha256': digest(contract), 'guide_sha256': digest(guide),
                           'path_count': len(spec['paths']), 'operation_count': len(entries)},
                          ensure_ascii=False)
    return '''<!doctype html>
<html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>WeRC API 接口文档</title>
<style>
:root{font:15px/1.65 system-ui,"Microsoft YaHei",sans-serif;color:#20221f;background:#f1f2ed}
*{box-sizing:border-box}body{max-width:1120px;margin:auto;padding:28px}a{color:#335d19}h1{font-size:30px;margin:0}h2{margin-top:32px}h3{font-size:16px;overflow-wrap:anywhere}
header{border-top:7px solid #d6ed38;padding:20px 0}nav{display:flex;gap:16px;flex-wrap:wrap}p{white-space:pre-wrap}article,section.guide{background:#fff;border:1px solid #d4d7cb;padding:20px;margin:14px 0}
b{background:#e7ee9e;padding:4px 8px}pre{white-space:pre-wrap;overflow-wrap:anywhere;background:#f4f5ef;padding:16px;font-size:13px;line-height:1.6}summary{cursor:pointer;padding:10px 0;font-weight:600}details{border-top:1px solid #e1e3db}input{width:100%;padding:13px;border:1px solid #949b83;background:white;font:inherit}label{font-weight:600}.search{position:sticky;top:0;background:#f1f2ed;padding:10px 0}small{color:#545c49}button{padding:8px;font:inherit}a:focus-visible,summary:focus-visible,input:focus-visible{outline:3px solid #849321}details:target{border:2px solid #849321}article[hidden]{display:none}@media(max-width:600px){body{padding:12px}article,section.guide{padding:12px}}
</style>
<header><small>WeRC // DEVELOPER REFERENCE</small><h1>API 接口文档</h1>
<p>离线可用 · 版本 ''' + esc(spec['info']['version']) + ' · ' + str(len(spec['paths'])) + ' 条路径 / ' + str(len(entries)) + ''' 个操作</p>
<nav><a href="#quickstart">接入与示例</a><a href="#endpoints">HTTP 接口</a><a href="#schemas">数据结构</a><a href="openapi.json">OpenAPI 原始契约</a><a href="bot-client.md">Bot 接入说明</a></nav></header>
<section class="guide" id="quickstart"><h2>接入与示例</h2>
<p>在桌面「Bot 接入」启动本机 API，创建令牌并复制实际地址。下方所有接口的请求、响应及限制由随包 OpenAPI 自动生成。端口以桌面显示为准，默认 http://127.0.0.1:5031。没有令牌或令牌撤销后返回 401。</p>
<p>原生收发支持微信 4.2.0.52（推荐）、4.1.15.13 和 4.1.13.7 的匹配 x64 构建；实际能力以桌面兼容性检测为准。发送以任务状态为准；unknown 不应新建任务重发。独立在线查询可并行，与来源变更冲突时短时排队，后台补读让位；达到并发额度或等待超时仍返回 BUSY（429），只读请求可稍后重试；写请求应保留幂等键并先查原任务。</p>
<details><summary>展开完整接入说明与请求示例（HTTP / SSE / MCP）</summary><pre>''' + esc(guide.decode('utf-8-sig').replace('\r\n', '\n')) + '''</pre></details></section>
<h2 id="endpoints">HTTP 接口</h2><div class="search"><label for="filter">搜索路径、方法、字段或说明</label><input id="filter" type="search" placeholder="例如 send-text、account_id、SSE"><small id="count" role="status" aria-live="polite"></small></div>
''' + ''.join(entries) + '<h2 id="schemas">数据结构</h2>' + schemas + '''
<script id="documentation-metadata" type="application/json">''' + metadata + '''</script>
<script>
const entries=[...document.querySelectorAll('.endpoint')];
document.querySelector('#filter').addEventListener('input',event=>{const q=event.target.value.toLowerCase().trim();let count=0;for(const entry of entries){entry.hidden=!entry.textContent.toLowerCase().includes(q);if(!entry.hidden)count++;}document.querySelector('#count').textContent=`显示 ${count} / ${entries.length} 个操作`;});
function revealSchema(){const id=decodeURIComponent(location.hash.slice(1));const el=document.getElementById(id);if(el?.classList.contains('schema')){el.open=true;el.scrollIntoView();}}
window.addEventListener('hashchange',revealSchema);revealSchema();
</script></html>'''


def stage_docs(destination):
    destination = Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    contract, guide = SOURCES['openapi.json'].read_bytes(), SOURCES['bot-client.md'].read_bytes()
    for name, source in SOURCES.items():
        shutil.copy2(source, destination / name)
    (destination / FILENAME).write_text(render(contract, guide), encoding='utf-8')
    verify_docs(destination)


def verify_docs(destination):
    destination = Path(destination)
    contract = (destination / 'openapi.json').read_bytes()
    guide = (destination / 'bot-client.md').read_bytes()
    assert contract == SOURCES['openapi.json'].read_bytes(), 'Stale packaged OpenAPI contract'
    assert guide == SOURCES['bot-client.md'].read_bytes(), 'Stale packaged Bot guide'
    actual = (destination / FILENAME).read_text(encoding='utf-8')
    assert actual == render(contract, guide), 'Missing or stale offline API reference'
    return {'paths': len(json.loads(contract)['paths']), 'operations': len(operations(json.loads(contract)))}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('destination', type=Path)
    parser.add_argument('--verify', action='store_true')
    args = parser.parse_args()
    if not args.verify:
        stage_docs(args.destination)
    print(json.dumps(verify_docs(args.destination), ensure_ascii=False))
