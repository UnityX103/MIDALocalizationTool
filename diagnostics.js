(function(){
 'use strict';
 const rawFetch=globalThis.fetch.bind(globalThis),native=globalThis.__TAURI__?.core.invoke;
 const rawInvoke=native?native.bind(globalThis.__TAURI__.core):null;
 let info=null,queue=[],flushing=null,problem='',dropped=0,ready;
 const id=()=>crypto.randomUUID();
 const redact=text=>String(text).replace(/(?:Bearer\s+)[^\s,;"']+/gi,'Bearer <redacted>').replace(/(token|password|secret|authorization|accessKey)(\s*[=:]\s*)[^\s,;]+/gi,'$1$2<redacted>').replace(/(?:\/Users\/|\/home\/|[A-Za-z]:\\Users\\)[^\s)]+/g,'<path>').slice(0,8000);
 function record(event,fields={}){
  const row={source:'frontend',level:fields.level||'INFO',event:String(event).slice(0,160),operationId:fields.operationId||id(),occurredAt:Date.now()};
  for(const key of ['durationMs','status','errorType','stack'])if(fields[key]!==undefined)row[key]=typeof fields[key]==='string'?redact(fields[key]):fields[key];
  queue.push(row);if(queue.length>128){queue.shift();dropped++;}
 }
 function failed(event,error,operationId,start){record(event,{level:'ERROR',operationId,durationMs:Math.round(performance.now()-start),errorType:error?.name||'operation_failed',status:Number(error?.status)||0,stack:String(error?.stack||'').split('\n').slice(1).join('\n')});}
 async function invoke(command,args={}){
  const operationId=id(),start=performance.now();record(command+'.start',{operationId});
  try{const value=await rawInvoke(command,{...args,operationId});record(command+'.success',{operationId,durationMs:Math.round(performance.now()-start)});return value;}
  catch(error){failed(command+'.failed',error,operationId,start);throw error;}
 }
 async function flush(){
  await ready;
  if(flushing)return flushing;
  if(!queue.length)return;
  const batch=queue.splice(0,128);
  flushing=(async()=>{try{
   if(rawInvoke)await rawInvoke('diagnostics_write',{events:batch});
   else{const response=await rawFetch('/api/diagnostics/write',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({events:batch}),signal:AbortSignal.timeout(8000)});if(!response.ok)throw new Error('日志保存失败');}
   problem='';
  }catch{queue.unshift(...batch);dropped+=Math.max(0,queue.length-128);queue=queue.slice(-128);problem='运行日志暂未保存，请检查日志状态和磁盘空间';}finally{flushing=null;}})();
  return flushing;
 }
 async function snapshot(){
  await ready;await flush();if(queue.length)await flush();
  if(problem||info?.error)throw new Error(problem||info.error);
  let text;
  if(rawInvoke)text=await rawInvoke('diagnostics_snapshot');
  else{const response=await rawFetch('/api/diagnostics/snapshot',{signal:AbortSignal.timeout(10000)});const value=await response.json();if(!response.ok)throw new Error(value.error||'无法读取运行日志');text=value.text;}
  if(typeof text!=='string'||!text.trim())throw new Error('本次运行日志尚不可用');
  return JSON.stringify({type:'mida-diagnostics',formatVersion:1,timeUnit:'unix-ms',version:info.version,platform:info.platform,runId:info.runId,droppedEvents:dropped,exportedAt:new Date().toISOString()})+'\n'+text;
 }
 ready=(async()=>{try{
  if(rawInvoke)info=await rawInvoke('diagnostics_info');else{const response=await rawFetch('/api/diagnostics/info',{signal:AbortSignal.timeout(8000)});info=await response.json();if(!response.ok)throw new Error('日志初始化失败');}
  record('frontend.start');
 }catch{info={version:'unknown',platform:rawInvoke?'desktop':'browser',error:'日志初始化失败，无法准备自动日志附件'};problem=info.error;}})();
 globalThis.fetch=async function(input,options){
  let url;try{url=new URL(typeof input==='string'||input instanceof URL?input:input.url,location.href);}catch{return rawFetch(input,options);}
  if(url.origin!==location.origin||!url.pathname.startsWith('/api/')||url.pathname.startsWith('/api/diagnostics/'))return rawFetch(input,options);
  const operationId=id(),start=performance.now(),headers=new Headers(options?.headers||(!(input instanceof Request)?undefined:input.headers));headers.set('X-Diagnostics-Operation',operationId);
  const event='http.'+url.pathname.replace(/\/api\//,'').replace(/[0-9a-f-]{32,}/g,'request');record(event+'.start',{operationId});
  try{const response=await rawFetch(input,{...options,headers});record(event+'.'+(response.ok?'success':'failed'),{operationId,durationMs:Math.round(performance.now()-start),status:response.status,level:response.ok?'INFO':'ERROR'});return response;}
  catch(error){failed(event+'.failed',error,operationId,start);throw error;}
 };
 window.addEventListener('error',event=>failed('frontend.unhandled_error',event.error,id(),performance.now()));
 window.addEventListener('unhandledrejection',event=>failed('frontend.unhandled_rejection',event.reason,id(),performance.now()));
 document.addEventListener('click',event=>{const button=event.target.closest?.('#mida-loc-app button');if(button&&!button.closest('.feedback-form'))record('ui.activate.'+(button.id||'button'));},true);
 for(const event of ['play','pause','ended','error'])document.addEventListener(event,e=>{if(e.target.tagName==='VIDEO')record('video.'+event,{level:event==='error'?'ERROR':'INFO'});},true);
 window.addEventListener('pagehide',()=>{record('frontend.pagehide');if(!rawInvoke&&queue.length){const batch=queue.splice(0,128);if(!navigator.sendBeacon('/api/diagnostics/write',new Blob([JSON.stringify({events:batch})],{type:'application/json'})))queue.unshift(...batch);}else void flush();});
 setInterval(()=>void flush(),1000);
 function instrument(owner){if(!owner?.prototype)return;for(const name of Object.getOwnPropertyNames(owner.prototype)){const fn=owner.prototype[name];if(typeof fn!=='function'||fn.constructor.name!=='AsyncFunction')continue;owner.prototype[name]=async function(...args){const identity=id(),start=performance.now(),event='workspace.'+name;record(event+'.start',{operationId:identity});try{const value=await fn.apply(this,args);record(event+'.success',{operationId:identity,durationMs:Math.round(performance.now()-start)});return value;}catch(error){failed(event+'.failed',error,identity,start);throw error;}};}}
 function operation(name,fn){return async function(...args){const identity=id(),start=performance.now();record(name+'.start',{operationId:identity});try{const value=await fn.apply(this,args);record(name+'.complete',{operationId:identity,durationMs:Math.round(performance.now()-start)});return value;}catch(error){failed(name+'.failed',error,identity,start);throw error;}};}
 function download(text,name){const url=URL.createObjectURL(new Blob([text],{type:'text/plain;charset=utf-8'}));const a=document.createElement('a');a.href=url;a.download=name;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);}
 async function show(content){
  content.replaceChildren();await ready;
  const el=(tag,text)=>{const node=document.createElement(tag);node.textContent=text;return node;};
  content.append(el('p','日志记录界面及后端操作、耗时和异常。单份最多 5 MiB，最多保留 20 份，总量不超过 50 MiB。'));
  content.append(el('p','本机位置：'+(info.directory||'不可用')),el('p',problem||info.error||(dropped?'日志正在记录；队列曾丢失 '+dropped+' 个事件。':'日志正在本机记录，页面刷新沿用当前运行。')));
  const preview=el('pre',''),exportButton=el('button','导出当前运行日志'),view=el('button','查看当前运行日志');
  const read=async()=>{try{const text=await snapshot();preview.textContent=text.slice(-50000);return text;}catch(error){preview.textContent=String(error.message||error);return null;}};
  view.onclick=()=>void read();exportButton.onclick=async()=>{const text=await read();if(text)download(text,'mida-diagnostics-'+info.runId+'.jsonl');};content.append(view,exportButton);
  if(rawInvoke){const open=el('button','打开日志目录');open.onclick=async()=>{try{await rawInvoke('diagnostics_open');}catch{preview.textContent='无法打开日志目录，可使用上方导出按钮。';}};content.append(open);}
  const details=el('details',''),summary=el('summary','日志预览（最近 50,000 字符）');details.append(summary,preview);content.append(details);
 }
 globalThis.MidaDiagnostics={record,report:(error,event="frontend.handled_error")=>failed(event,error,id(),performance.now()),invoke,flush,snapshot,instrument,operation,show,download,ready,getInfo:async()=>{await ready;return {...info,error:problem||info.error,droppedEvents:dropped};}};
})();
