'use strict';
let port;
let selectedTabId = null;
function connect() {
  if (port) return;
  port = browser.runtime.connectNative('org.rustune.bridge');
  port.onDisconnect.addListener(() => { port = null; });
  port.onMessage.addListener(async message => {
    const current = port;
    let stage = 'validate';
    let hint = 'none';
    try {
      if (!Number.isSafeInteger(message.id) || !['snapshot','dispatch','login'].includes(message.operation)) throw new Error('invalid command');
      stage = 'permissions';
      if (!await browser.permissions.contains({origins:['https://music.apple.com/*']})) throw new Error('host permission missing');
      stage = 'find_tab';
      const tabs = await browser.tabs.query({url: 'https://music.apple.com/*'});
      if (message.operation === 'login') selectedTabId = null;
      const tab = selectedTabId === null
        ? (tabs.find(t => t.audible) || tabs.find(t => t.active) || tabs[0])
        : tabs.find(t => t.id === selectedTabId);
      if (!tab) { hint = selectedTabId === null ? 'no_music_tab' : 'pinned_tab_closed'; throw new Error('no music tab'); }
      // The browser-enforced URL query and host permission constrain the target.
      // Do not require reading Tab.url (Firefox can redact it without tabs permission).
      stage = 'install_adapter';
      selectedTabId = tab.id;
      await browser.scripting.executeScript({target:{tabId:tab.id},world:'MAIN',files:['adapter.js']});
      if (message.operation === 'login') {
        await browser.tabs.update(tab.id,{active:true});
        await browser.windows.update(tab.windowId,{focused:true});
      }
      stage = 'dispatch';
      const results = await browser.scripting.executeScript({
        target:{tabId:tab.id},world:'MAIN',
        func: async (operation,args) => {
          if (!window.rustune) throw new Error('controller unavailable');
          if (operation === 'snapshot' || operation === 'login') return window.rustune.snapshot();
          const allowed = ['browse','play_track','play','pause','seek','volume','logout'];
          if (!allowed.includes(args.action)) throw new Error('unsupported command');
          return await window.rustune.dispatch(args.action,args.parameters);
        },
        args:[message.operation,message.args || {}]
      });
      if (!results.length || results[0].error) throw new Error('page operation failed');
      current?.postMessage({id:message.id,ok:true,result:results[0].result});
    } catch (error) {
      // Raw page errors can contain personal data. Return a fixed failure only.
      current?.postMessage({id:message.id,ok:false,stage,hint,error_name:['Error','TypeError','ReferenceError','SecurityError','NotAllowedError'].includes(error?.name) ? error.name : 'Error'});
    }
  });
}
browser.action.onClicked.addListener(() => { selectedTabId = null; connect(); });
connect();
