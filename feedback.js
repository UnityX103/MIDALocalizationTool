(function(){
 'use strict';
 const REPO='nanzhaigame-xpy/MIDALocalizationTool',LIMIT=25*1024*1024;
 let dbPromise,busy=false,view=null,timer=null;
 const el=(tag,text='',cls='')=>{const node=document.createElement(tag);node.textContent=text;node.className=cls;return node;};
 async function db(){if(!dbPromise)dbPromise=new Promise((resolve,reject)=>{const r=indexedDB.open('mida-localization-editor-feedback',1);r.onupgradeneeded=()=>r.result.createObjectStore('requests');r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(new Error('反馈记录无法保存，尚未发送'));r.onblocked=()=>reject(new Error('反馈存储被其他窗口占用'));});return dbPromise;}
 async function ledger(value){const connection=await db();return new Promise((resolve,reject)=>{let tx;try{tx=connection.transaction('requests',value?'readwrite':'readonly',{durability:'strict'});}catch{tx=connection.transaction('requests',value?'readwrite':'readonly');}const store=tx.objectStore('requests');let request;if(value)request=store.put(value,'active');else request=store.get('active');tx.oncomplete=()=>resolve(value||request.result);tx.onabort=()=>reject(new Error('反馈记录保存失败，停止发送'));tx.onerror=()=>{};});}
 const base64=bytes=>{let text='';for(let i=0;i<bytes.length;i+=32768)text+=String.fromCharCode(...bytes.subarray(i,i+32768));return btoa(text);};
 function external(link){link.target='_blank';link.rel='noopener noreferrer';if(globalThis.__TAURI__)link.onclick=event=>{event.preventDefault();void MidaDiagnostics.invoke('feedback_open',{url:link.href}).catch(()=>{if(view)view.status.textContent='反馈已提交，但无法打开链接；可复制地址到浏览器查看。';});};return link;}
 async function call(record,action,data){
  const value={action,id:record.id,key:record.key,data:data||null};let reply;
  if(globalThis.__TAURI__)reply=await MidaDiagnostics.invoke('feedback_request',value);
  else{const response=await fetch('/api/feedback',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(value),signal:AbortSignal.timeout(45000)});reply=await response.json();if(!response.ok)throw new Error(reply.error||'反馈响应未确认');}
  if(!reply||!Number.isInteger(reply.status)||!reply.result||typeof reply.result!=='object')throw new Error('反馈响应格式未确认');
  if(reply.status>=400){const error=new Error(reply.result.error||reply.result.message||'反馈服务拒绝请求');error.status=reply.status;throw error;}
  if(action!=='upload'&&(reply.result.id!==record.id||reply.result.repo!==REPO))throw new Error('反馈回执身份不一致');
  return reply.result;
 }
 function describe(record){
  if(!view?.content.isConnected)return;
  view.status.textContent=record.state==='succeeded'?'反馈已提交。':record.state==='failed'?'反馈提交失败：'+(record.error||'服务明确拒绝'):record.error||record.remote?.error||'正在提交反馈，请勿重复发送。';
  view.result.replaceChildren();
  if(record.state==='succeeded'&&/^\d+$/.test(String(record.remote?.issue))){const a=el('a','查看反馈 Issue');a.href='https://cnb.cool/'+REPO+'/-/issues/'+record.remote.issue;external(a);view.result.append(a);for(const asset of record.remote.attachments||[]){if(!/^https:\/\//.test(asset.asset_link||''))continue;const p=el('p'),link=el('a',asset.name+' · 已上传');link.href=asset.asset_link;external(link);p.append(link);view.result.append(p);}}
  const finished=['succeeded','failed'].includes(record.state);
  view.check.hidden=finished;view.fresh.hidden=!finished;view.submit.hidden=finished;
  view.submit.disabled=true;view.inputs.forEach(input=>input.disabled=true);
  if(finished){view.cleanup();view.screenshots.value='';view.preview.replaceChildren();view.status.scrollIntoView({block:'center'});}
 }
 async function accept(record,remote){
  record.remote=remote;record.state=remote.state;
  record.error='';
  if(record.intent==='create' || record.intent==='commit'&&remote.state!=='receiving' || record.intent?.startsWith('upload:')&&(remote.attachments||[]).some(a=>a.id===record.intent.slice(7)&&['staged','uploaded'].includes(a.state)))record.intent=null;
  if(remote.state==='succeeded'){
   if(!/^\d+$/.test(String(remote.issue))||record.files.length&&remote.uploads_confirmed!==true)throw new Error('Issue 或附件完成状态未确认');
   record.files=[];record.error='';record.intent=null;
  }else if(remote.state==='failed'||remote.state==='expired'){record.state='failed';record.error=remote.error||'服务明确拒绝或附件暂存已过期';record.intent=null;record.files=[];}
  await ledger(record);describe(record);
 }
 async function advance(record,remote){
  await accept(record,remote);
  if(['succeeded','failed'].includes(record.state))return;
  if(record.intent){record.error='原操作结果待核对，附件和请求已保留，不会重复发送。';await ledger(record);describe(record);return;}
  if(remote.state==='receiving'){
   if(!Array.isArray(remote.attachments)||remote.attachments.length!==record.files.length)throw new Error('附件清单回执不一致');
   for(let index=0;index<record.files.length;index++){
    const file=record.files[index],asset=remote.attachments[index];
    if(asset.name!==file.name||asset.sha256!==file.sha256||asset.size!==file.size)throw new Error('附件回执内容不一致');
    if(['staged','uploaded'].includes(asset.state))continue;
    record.intent='upload:'+asset.id;await ledger(record);const reply=await call(record,'upload',{attachmentId:asset.id,bytes:base64(new Uint8Array(await file.blob.arrayBuffer()))});
    if(reply.id!==asset.id||reply.state!=='staged')throw new Error('附件接收结果未确认');
    record.intent=null;asset.state='staged';await ledger(record);
   }
   record.intent='commit';await ledger(record);await advance(record,await call(record,'commit'));
  }
  schedule();
 }
 function schedule(){clearTimeout(timer);if(view?.content.isConnected)timer=setTimeout(()=>void check(),2500);}
 async function check(){
  if(busy)return;busy=true;
  try{const record=await ledger();if(!record||['succeeded','failed'].includes(record.state))return;await advance(record,await call(record,'get'));}
  catch(error){await unknown(error);}
  finally{busy=false;}
 }
 async function unknown(error){try{const record=await ledger();if(!record)return;record.error='提交结果待核对：'+String(error.message||error)+ '。原请求和附件已保留。';if(record.state!=='succeeded')record.state='unknown';await ledger(record);describe(record);}catch{if(view?.content.isConnected)view.status.textContent='本机反馈记录无法保存，请保留此页面。';}}
 async function filesWithLogs(files,includeLogs){
  const prepared=files.map(blob=>({name:blob.name,kind:'image',blob,size:blob.size}));
  if(includeLogs){const text=await MidaDiagnostics.snapshot(),blob=new Blob([text],{type:'text/plain;charset=utf-8'});prepared.push({name:'mida-diagnostics-'+new Date().toISOString().replace(/[:.]/g,'-')+'.jsonl',kind:'file',blob,size:blob.size});}
  if(prepared.length>10||prepared.reduce((n,f)=>n+f.size,0)>LIMIT)throw new Error('最多 10 个附件，总量不能超过 25 MiB');
  for(const file of prepared){if(!file.size||file.size>(file.kind==='image'?5:10)*1024*1024)throw new Error('图片单个最多 5 MiB，日志单个最多 10 MiB');file.name=file.name.replace(/[\/\\\x00-\x1f]/g,'_');const extension=/\.[A-Za-z0-9]{1,8}$/.exec(file.name)?.[0]||'',characters=Array.from(extension?file.name.slice(0,-extension.length):file.name);while(new TextEncoder().encode(characters.join('')+extension).length>240)characters.pop();file.name=(characters.join('')||'screenshot')+extension;file.sha256=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',await file.blob.arrayBuffer())),b=>b.toString(16).padStart(2,'0')).join('');}
  return prepared;
 }
 async function open(content,{fresh:startFresh=false}={}){
  view?.cleanup?.();
  clearTimeout(timer);content.classList.add('feedback-form');content.replaceChildren();content.parentElement.scrollTop=0;
  const intro=el('p','反馈、截图和附送的日志会公开发布到当前项目的 CNB 仓库。日志默认附送，可预览或取消；截图请避开私密内容。'),title=el('input'),description=el('textarea'),steps=el('textarea'),screenshots=el('input'),logs=el('input'),status=el('p','','helper'),result=el('div'),submit=el('button','提交反馈','primary'),checkButton=el('button','核对原请求'),fresh=el('button','新建另一条反馈');
  title.maxLength=160;description.maxLength=12000;steps.maxLength=4000;description.rows=5;steps.rows=3;screenshots.type='file';screenshots.accept='image/png,image/jpeg,image/gif,image/webp';screenshots.multiple=true;logs.type='checkbox';logs.checked=true;status.setAttribute('role','status');status.setAttribute('aria-live','polite');
  const field=(label,node)=>{const box=el('label',label,'feedback-field');box.append(node);return box;};
  const logLabel=el('label','','feedback-log-choice');logLabel.append(logs,document.createTextNode(' 附送本次运行日志（自动准备，可取消）'));
  content.append(intro,el('p',REPO,'helper'),field('标题',title),field('问题描述',description),field('复现步骤（可选）',steps),field('截图（可选）',screenshots),logLabel);
  const preview=el('div','','feedback-previews');content.append(preview);
  const urls=[];screenshots.onchange=()=>{urls.splice(0).forEach(url=>URL.revokeObjectURL(url));preview.replaceChildren();for(const file of screenshots.files){if(file.size>5*1024*1024){status.textContent='截图超过 5 MiB，请重新选择';continue;}const img=el('img');const url=URL.createObjectURL(file);urls.push(url);img.src=url;img.alt=file.name;preview.append(img);} };
  const logPreview=el('button','预览自动日志'),logText=el('pre'),details=el('details'),summary=el('summary','日志预览（最近 50,000 字符）');details.append(summary,logText);content.append(logPreview,details);
  logPreview.onclick=async()=>{try{logText.textContent=(await MidaDiagnostics.snapshot()).slice(-50000);details.open=true;}catch(error){status.textContent=String(error.message||error);}};
  const actions=el('div','','dialog-actions');actions.append(submit,checkButton,fresh);content.append(status,result,actions);
  view={content,status,result,submit,check:checkButton,fresh,screenshots,preview,inputs:[title,description,steps,screenshots,logs],cleanup:()=>{urls.splice(0).forEach(url=>URL.revokeObjectURL(url));clearTimeout(timer);}};checkButton.hidden=true;fresh.hidden=true;
  checkButton.onclick=()=>void check();fresh.onclick=async()=>{if(busy)return;try{const old=await ledger();if(old&&!['succeeded','failed'].includes(old.state))return;await open(content,{fresh:true});}catch(error){status.textContent=String(error.message||error);}};
  submit.onclick=async()=>{
   if(busy)return;busy=true;submit.disabled=true;
   try{
    const existing=await ledger();if(existing&&!['succeeded','failed'].includes(existing.state))throw new Error('已有未确认反馈，请先核对原请求');
    if(!title.value.trim()||!description.value.trim())throw new Error('请填写标题和问题描述');status.textContent='正在准备附件和日志…';
    const info=await MidaDiagnostics.getInfo(),files=await filesWithLogs([...screenshots.files],logs.checked),identity=crypto.randomUUID(),key=Array.from(crypto.getRandomValues(new Uint8Array(32)),b=>b.toString(16).padStart(2,'0')).join('');
    const record={id:identity,key,state:'preparing',intent:'create',files,remote:null,error:'',createdAt:Date.now(),data:{id:identity,title:title.value.trim(),description:description.value.trim(),steps:steps.value.trim(),version:info.version,platform:info.platform,attachments:files.map(({name,kind,size,sha256})=>({name,kind,size,sha256}))}};
    await ledger(record);describe(record);MidaDiagnostics.record('feedback.create_intent');
    try{await advance(record,await call(record,'create',record.data));}catch(error){if([400,403,413,415,422,429,507].includes(error.status)){record.state='failed';record.error=String(error.message);record.intent=null;record.files=[];await ledger(record);describe(record);}else await unknown(error);}
   }catch(error){status.textContent=String(error.message||error);submit.disabled=false;}
   finally{busy=false;}
  };
  try{const record=await ledger();if(record&&(!startFresh||!['succeeded','failed'].includes(record.state))){title.value=record.data.title;description.value=record.data.description;steps.value=record.data.steps;logs.checked=record.data.attachments.some(file=>file.kind==='file');for(const file of record.files.filter(file=>file.kind==='image')){const img=el('img'),url=URL.createObjectURL(file.blob);urls.push(url);img.src=url;img.alt=file.name;preview.append(img);}describe(record);if(!['succeeded','failed'].includes(record.state))void check();} }catch(error){status.textContent=String(error.message||error);submit.disabled=true;}
 }
 globalThis.MidaFeedback={open,close:()=>{view?.cleanup?.();view=null;}};
})();
