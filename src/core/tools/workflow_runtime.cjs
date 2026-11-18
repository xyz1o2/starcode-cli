'use strict';
// 子进程仅负责 JS 控制流；所有 agent 派发通过父进程的工具权限门。
const readline = require('node:readline');
const vm = require('node:vm');
const input = readline.createInterface({ input: process.stdin });
let initialized = false;
let sequence = 0;
const pending = new Map();
const emit = value => process.stdout.write(JSON.stringify(value) + '\n');
input.on('line', async line => {
  try {
    const message = JSON.parse(line);
    if (initialized) {
      const request = pending.get(message.id);
      if (!request) throw new Error('Unknown workflow response');
      pending.delete(message.id);
      if (message.error) request.reject(new Error(message.error));
      else request.resolve(message.value);
      return;
    }
    initialized = true;
    const labels = new Set();
    const agent = (prompt, opts = {}) => {
      if (typeof prompt !== 'string' || !prompt.trim()) throw new Error('agent prompt must be nonempty');
      if (typeof opts.label !== 'string' || !opts.label.trim()) throw new Error('agent requires a unique label for replay');
      if (labels.has(opts.label)) throw new Error('Duplicate agent label: ' + opts.label);
      labels.add(opts.label);
      const id = sequence++;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        emit({ type: 'agent', id, prompt, opts });
      });
    };
    const parallel = tasks => Promise.all(tasks.map(task => task()));
    const pipeline = (items, ...stages) => parallel(items.map((item, index) => async () => {
      let value = item;
      for (const stage of stages) value = await stage(value, item, index);
      return value;
    }));
    const phase = title => emit({ type: 'phase', title: String(title) });
    const log = message => emit({ type: 'log', message: String(message) });
    // 工作流与 .sh 一样是经审批的代码执行；vm 只隔离名字空间，不作为安全沙箱。
    const context = vm.createContext({ agent, parallel, pipeline, phase, log, args: message.args });
    const source = message.script.replace(/^\s*export\s+const\s+meta\s*=/, 'const meta =');
    const script = new vm.Script('(async () => {\n' + source + '\n})()', { filename: 'workflow.js' });
    const value = await script.runInContext(context);
    if (pending.size) throw new Error('Workflow returned with unawaited agent calls');
    emit({ type: 'result', value: value === undefined ? null : value });
    input.close();
    process.stdin.destroy();
  } catch (error) {
    emit({ type: 'error', error: String(error.message || error) });
    input.close();
    process.stdin.destroy();
    process.exitCode = 1;
  }
});
