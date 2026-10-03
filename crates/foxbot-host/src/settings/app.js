'use strict';
const $ = id => document.getElementById(id);
const token = location.hash.slice(1) || sessionStorage.getItem('foxbot-settings-session') || '';
if (location.hash) { sessionStorage.setItem('foxbot-settings-session', token); history.replaceState(null, '', '/'); }
let view = null, selected = null, dirty = false, testing = false;
function status(message, error = false) { $('status').textContent = message; $('status').classList.toggle('error', error); }
async function api(path, data) {
  const response = await fetch(path, {method:data === undefined ? 'GET' : 'POST', cache:'no-store',
    headers:{'X-FoxBot-Session':token, ...(data === undefined ? {} : {'Content-Type':'application/json'})},
    body:data === undefined ? undefined : JSON.stringify(data)});
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || '请求失败。');
  return value;
}
function renderList() {
  $('connections').replaceChildren();
  $('count').textContent = view.config.connections.length;
  $('empty').hidden = view.config.connections.length !== 0;
  for (const c of view.config.connections) {
    const button = document.createElement('button'); button.type = 'button'; button.className = 'connection-item';
    button.classList.toggle('active', c.id === selected); button.dataset.connectionId = c.id;
    if (c.id === view.config.default_connection) { const pill = document.createElement('span'); pill.className = 'pill'; pill.textContent = '默认'; button.append(pill); }
    const name = document.createElement('strong'); name.textContent = c.name;
    const detail = document.createElement('small'); detail.textContent = c.model || '自定义业务接口';
    button.append(name, detail); button.addEventListener('click', () => { if (!dirty || confirm('放弃尚未保存的修改？')) editConnection(c); });
    $('connections').append(button);
  }
}
function protocolChanged() {
  const business = $('protocol').value === 'business_v1';
  $('model-label').hidden = business; $('model').required = !business;
  $('business-options').hidden = !business;
}
function editConnection(c = null) {
  selected = c?.id || null; dirty = false;
  $('connection-form').reset();
  $('name').value = c?.name || ''; $('endpoint').value = c?.endpoint || '';
  $('protocol').value = c?.protocol || 'chat_completions'; $('model').value = c?.model || '';
  $('context-mode').value = c?.context_mode || 'client_managed'; $('receipt-endpoint').value = c?.receipt_endpoint || '';
  $('idempotency').checked = c?.idempotency_supported || false; $('staging').checked = c?.staging_contract || false;
  $('api-key').value = ''; $('api-key').type = 'password'; $('api-key').disabled = false; $('toggle-key').textContent = '显示';
  $('api-key').placeholder = c?.has_api_key ? '已保存，留空保持原 Key' : '直接粘贴 API Key';
  $('key-state').textContent = c?.has_api_key ? '已保存' : '可选';
  $('editor-title').textContent = c ? '编辑接口' : '添加接口'; $('editor-kind').textContent = c ? 'CONNECTION DETAILS' : 'NEW CONNECTION';
  $('default-badge').hidden = !c || c.id !== view.config.default_connection;
  $('duplicate').disabled = !c; $('delete').disabled = !c; $('set-default').disabled = !c || c.id === view.config.default_connection;
  protocolChanged(); renderList();
}
function inputConnection() {
  const business = $('protocol').value === 'business_v1';
  const c = {id:selected || `connection-${crypto.randomUUID()}`, name:$('name').value.trim(), protocol:$('protocol').value,
    endpoint:$('endpoint').value.trim(), model:business ? null : $('model').value.trim(),
    context_mode:business ? $('context-mode').value : 'client_managed', receipt_endpoint:business ? ($('receipt-endpoint').value.trim() || null) : null,
    idempotency_supported:business && $('idempotency').checked, staging_contract:business && $('staging').checked};
  if ($('no-key').checked) c.api_key = '';
  else if ($('api-key').value) c.api_key = $('api-key').value;
  else if (!selected) c.api_key = '';
  return c;
}
async function change(edit, next) {
  const updated = await api('/api/edit', {revision:view.revision, edit}); view = updated;
  const c = view.config.connections.find(c => c.id === next) || null;
  editConnection(c); $('system-prompt').value = view.config.reply.system_prompt;
}
async function guard(fn) { try { await fn(); } catch (e) { status(e.message || '操作失败。', true); } }
$('connection-form').addEventListener('input', () => { dirty = true; });
$('protocol').addEventListener('change', protocolChanged);
$('no-key').addEventListener('change', () => { $('api-key').disabled = $('no-key').checked; });
$('toggle-key').addEventListener('click', () => { const hidden = $('api-key').type === 'password'; $('api-key').type = hidden ? 'text' : 'password'; $('toggle-key').textContent = hidden ? '隐藏' : '显示'; });
$('add').addEventListener('click', () => { if (!dirty || confirm('放弃尚未保存的修改？')) { editConnection(); $('name').focus(); } });
$('connection-form').addEventListener('submit', event => { event.preventDefault(); guard(async () => {
  const c = inputConnection(); $('save').disabled = true;
  try { await change({action:'save',connection:c}, c.id); status('接口已保存到本机。保存没有调用模型，也没有发送聊天消息。'); }
  finally { $('save').disabled = false; }
}); });
$('set-default').addEventListener('click', () => guard(async () => { await change({action:'set_default',id:selected}, selected); status('已设为默认接口，仅影响之后的新任务。'); }));
$('duplicate').addEventListener('click', () => guard(async () => {
  const ids = new Set(view.config.connections.map(c => c.id));
  view = await api('/api/edit', {revision:view.revision,edit:{action:'duplicate',id:selected}});
  editConnection(view.config.connections.find(c => !ids.has(c.id))); status('副本已保存，包含原接口的 Key；默认接口没有改变。');
}));
$('delete').addEventListener('click', () => guard(async () => {
  if (confirm('删除这个接口？已开始的任务不会自动切换到其他接口。')) { await change({action:'delete',id:selected}, null); status('接口已删除。'); }
}));
$('test').addEventListener('click', () => guard(async () => {
  if (testing || !$('connection-form').reportValidity()) return;
  if (!confirm('将发送一次固定的连接测试请求，可能产生少量 API 费用。不读取微信、不发送聊天消息，也不自动保存表单。继续？')) return;
  testing = true; $('test').disabled = true; $('test').textContent = '测试中…'; status('正在测试接口，最多等待约 15 秒…');
  try { const r = await api('/api/test', {connection:inputConnection(),confirm_billable:true}); status(`${r.message} 耗时 ${r.elapsed_ms} ms。`, !r.ok); }
  finally { testing = false; $('test').disabled = false; $('test').textContent = '测试连接'; }
}));
$('save-prompt').addEventListener('click', () => guard(async () => {
  view = await api('/api/edit',{revision:view.revision,edit:{action:'reply',system_prompt:$('system-prompt').value}});
  status('回复提示词已保存。自定义业务接口不会额外加入这段提示词。');
}));
$('folder').addEventListener('click', () => guard(async () => { await api('/api/open-folder',{}); }));
$('export').addEventListener('click', () => guard(async () => {
  const config = await api('/api/export'); const url = URL.createObjectURL(new Blob([JSON.stringify(config,null,2)+'\n'],{type:'application/json'}));
  const a = document.createElement('a'); a.href=url; a.download='foxbot-config-without-keys.json'; a.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  status('已导出配置，所有 API Key 均已移除。');
}));
$('shutdown').addEventListener('click', () => guard(async () => {
  if (dirty && !confirm('有未保存的修改，仍要关闭设置服务？')) return;
  await api('/api/shutdown',{}); dirty = false; sessionStorage.removeItem('foxbot-settings-session');
  document.querySelectorAll('button,input,select,textarea').forEach(e => e.disabled=true);
  status('设置服务已关闭。配置已保存的部分仍保留在本机。');
}));
window.addEventListener('beforeunload', e => { if(dirty) {e.preventDefault(); e.returnValue='';} });
guard(async () => {
  view = await api('/api/config'); $('config-path').textContent = view.config_path;
  $('system-prompt').value = view.config.reply.system_prompt;
  editConnection(view.config.connections.find(c => c.id === view.config.default_connection));
  status(view.config.connections.length ? '本地配置已载入。可以直接修改和保存，无需联网。' : '添加第一个接口后即可保存为默认接口，无需初始化密钥。');
});
