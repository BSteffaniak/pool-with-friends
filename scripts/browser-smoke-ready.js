#!/usr/bin/env node
// Local synthetic smoke pages only: never attach this collector to production.
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {spawn} = require('node:child_process');
const [browser, url, output] = process.argv.slice(2);
if (!browser || !output || new URL(url).hostname !== '127.0.0.1') {
  throw new Error('Expected browser, loopback URL, and output path');
}
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'pwmtf-smoke-profile-'));
const child = spawn(browser, [
  '--headless=new', '--remote-debugging-pipe', `--user-data-dir=${profile}`,
  '--disable-background-networking', '--disable-component-update',
  '--disable-default-apps', '--disable-extensions', '--disable-sync',
  '--disable-gpu-sandbox', '--enable-webgl', '--enable-unsafe-swiftshader',
  '--ignore-gpu-blocklist', '--no-first-run', '--no-default-browser-check',
  '--use-angle=swiftshader', '--window-size=1280,720', 'about:blank',
], {detached: true, stdio: ['ignore', 'ignore', 'pipe', 'pipe', 'pipe']});
let nextId = 0;
let buffer = '';
let failure = null;
let stderr = '';
const pending = new Map();
const diagnostics = [];
const contexts = new Map();
const startedAt = Date.now();
function record(message) {
  if (diagnostics.length < 100) diagnostics.push(`${Date.now() - startedAt}ms ${message}`);
}
child.stderr.on('data', data => { stderr = (stderr + data).slice(-32768); });
function fail(message) {
  failure = message;
  for (const {reject, timer} of pending.values()) {
    clearTimeout(timer);
    reject(new Error(message));
  }
  pending.clear();
}
child.on('error', () => fail('Browser could not start'));
child.on('exit', (code, signal) => fail(`Browser exited: code=${code} signal=${signal}`));
child.stdio[3].on('error', () => fail('Browser protocol pipe closed'));
child.stdio[4].on('data', data => {
  buffer += data.toString();
  let end;
  while ((end = buffer.indexOf('\0')) !== -1) {
    const message = JSON.parse(buffer.slice(0, end));
    buffer = buffer.slice(end + 1);
    const request = pending.get(message.id);
    if (request) {
      clearTimeout(request.timer);
      pending.delete(message.id);
      if (message.error) {
        const error = new Error(`${request.method}: ${message.error.message}`);
        error.protocolCode = message.error.code;
        request.reject(error);
      }
      else request.resolve(message.result);
    }
    if (message.method === 'Runtime.executionContextCreated') {
      const context = message.params.context;
      if (context.auxData?.isDefault) contexts.set(context.id, context);
    }
    if (message.method === 'Runtime.executionContextDestroyed') contexts.delete(message.params.executionContextId);
    if (message.method === 'Runtime.executionContextsCleared') contexts.clear();
    if (message.method === 'Page.frameNavigated') record(`frame navigated: ${message.params.frame.id}`);
    if (message.method === 'Runtime.exceptionThrown') {
      const detail = message.params.exceptionDetails;
      record(`Application exception: ${detail.exception?.description || detail.text}`);
      fail('Uncaught browser application exception');
    }
    if (message.method === 'Network.loadingFailed' && diagnostics.length < 50) {
      diagnostics.push(`Network loading failed: ${message.params.type} ${message.params.errorText}`);
    }
  }
});
function send(method, params = {}, sessionId, timeout = 10000) {
  if (failure) return Promise.reject(new Error(failure));
  const id = ++nextId;
  return new Promise((resolve, reject) => {
    record(`send ${method}`);
    const timer = setTimeout(() => {pending.delete(id); reject(new Error(`Protocol timeout after ${timeout}ms: ${method}`));}, timeout);
    pending.set(id, {resolve, reject, timer, method});
    child.stdio[3].write(JSON.stringify({id, method, params, sessionId}) + '\0');
  });
}
(async () => {
  let session;
  let html = '';
  try {
    // Synchronize the protocol handshake before asking Chrome to create a page.
    const version = await send('Browser.getVersion', {}, undefined, 60000);
    record(`Browser ready: ${version.product}`);
    const {targetId} = await send('Target.createTarget', {url: 'about:blank'}, undefined, 60000);
    session = (await send('Target.attachToTarget', {targetId, flatten: true})).sessionId;
    await send('Runtime.enable', {}, session);
    await send('Network.enable', {}, session);
    await send('Page.enable', {}, session);
    const navigation = await send('Page.navigate', {url}, session, 60000);
    if (navigation.errorText) throw new Error(`Navigation failed: ${navigation.errorText}`);
    const deadline = Date.now() + 60000;
    let ready = false;
    while (Date.now() < deadline) {
      if (failure) throw new Error(failure);
      const context = [...contexts.values()].find(value => value.auxData.frameId === navigation.frameId);
      if (!context) {
        await new Promise(resolve => setTimeout(resolve, 250));
        continue;
      }
      let evaluated;
      try {
        evaluated = await send('Runtime.evaluate', {
          expression: 'JSON.stringify({href:location.href,loaded:document.readyState !== "loading",state:document.querySelector("#game-shell")?.dataset.clientState,html:document.documentElement?.outerHTML || ""})',
          contextId: context.id,
          returnByValue: true,
        }, session);
      } catch (error) {
        // Only navigation-invalidated contexts are retryable; all other
        // protocol errors and application exceptions remain fatal.
        if (error.protocolCode && /Cannot find context|Execution context was destroyed/.test(error.message)) {
          record('Navigation replaced execution context; waiting for its successor');
          await new Promise(resolve => setTimeout(resolve, 250));
          continue;
        }
        throw error;
      }
      const {result, exceptionDetails} = evaluated;
      if (exceptionDetails) throw new Error(`Readiness evaluation failed: ${exceptionDetails.exception?.description || exceptionDetails.text}`);
      const page = JSON.parse(result.value);
      html = page.html;
      if (page.href !== url || !page.loaded) {
        await new Promise(resolve => setTimeout(resolve, 250));
        continue;
      }
      if (page.state === 'error' || page.state === 'failed') throw new Error(`Application state: ${page.state}`);
      if (page.state === 'ready') {ready = true; break;}
      await new Promise(resolve => setTimeout(resolve, 250));
    }
    if (!ready) throw new Error('Client did not become ready within 60 seconds');
    fs.writeFileSync(output, html);
  } catch (error) {
    fs.writeFileSync(output, `${html}\n${diagnostics.join('\n')}\n${stderr}`);
    console.error(`Smoke navigation failed (${url}): ${error.message}`);
    process.exitCode = 1;
  } finally {
    try {
      // Browser.close can close the pipe before replying. Shutdown is still
      // bounded by send's timeout and the process-group fallback below.
      if (child.pid) {
        await send('Browser.close').catch(() => {});
        const deadline = Date.now() + 3000;
        while (child.exitCode === null && child.signalCode === null && Date.now() < deadline) {
          await new Promise(resolve => setTimeout(resolve, 50));
        }
        // The detached browser owns this group; never signal unrelated Chrome
        // instances. Descendants may outlive the parent and still write profiles.
        try {
          process.kill(-child.pid, 'SIGKILL');
        } catch (error) {
          if (error.code !== 'ESRCH') throw error;
        }
        if (child.exitCode === null && child.signalCode === null) {
          await new Promise(resolve => child.once('exit', resolve));
        }
      }
      await fs.promises.rm(profile, {
        recursive: true, force: true, maxRetries: 10, retryDelay: 100,
      });
    } catch (error) {
      const message = `Browser cleanup failed: ${error.code || error.message}`;
      console.error(message);
      fs.appendFileSync(output, `\n${message}\n`);
      process.exitCode = 1;
    }
  }
})();
