function localizationUuid(){
 if(globalThis.crypto.randomUUID)return crypto.randomUUID();
 const bytes=crypto.getRandomValues(new Uint8Array(16));bytes[6]=(bytes[6]&15)|64;bytes[8]=(bytes[8]&63)|128;
 const hex=[...bytes].map(value=>value.toString(16).padStart(2,'0')).join('');
 return hex.slice(0,8)+'-'+hex.slice(8,12)+'-'+hex.slice(12,16)+'-'+hex.slice(16,20)+'-'+hex.slice(20);
}
class LocalizationWorkspaceStore {
 static readEnglishSnapshot(entry,language){
  if(Object.hasOwn(entry,'englishTranslationAtExport')){
   if(typeof entry.englishTranslationAtExport!=='string')throw new Error('旧英文快照 englishTranslationAtExport 必须是文本');
   return entry.englishTranslationAtExport;
  }
  return language==='en'?entry.translationAtExport:'';
 }
 static isLegacyDemo(snapshot){return Boolean(snapshot)&&(typeof snapshot.currentProjectId!=='string'||!snapshot.currentProjectId.trim()||snapshot.fileVersion?.lineageId==='demo-task-package'||snapshot.currentPackageId==='demo-import-v1');}
 constructor(){this.database=null;this.revision=0;this.pendingMediaImport=null;this.pendingMediaTerminal=false;this.spaceId='';this.cacheEpoch=0;}
 static validSpace(value){return typeof value==='string'&&/^[a-z][a-z0-9_-]{0,31}$/.test(value);}
 storageKey(key){return this.spaceId&&key!=='cache-generation'?'space:'+this.spaceId+':'+key:key;}
 async clearAll(){
  if(globalThis.__TAURI__){await globalThis.__TAURI__.core.invoke('clear_all_cache',{confirmed:true});return;}
  const response=await fetch('/api/cache/clear',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({confirmed:true})});
  const result=await response.json();if(!response.ok)throw new Error(result.error||'视频缓存清理失败');
  await this.open();
  await new Promise((resolve,reject)=>{
   const transaction=this.database.transaction(['workspace','backups'],'readwrite');const workspace=transaction.objectStore('workspace');const generation=workspace.get('cache-generation');
   generation.onsuccess=()=>{workspace.clear();transaction.objectStore('backups').clear();workspace.put((generation.result||0)+1,'cache-generation');};
   transaction.oncomplete=resolve;transaction.onabort=()=>reject(transaction.error||new Error('清空本地存档失败'));
  });
 }
 async spaces(){
  if(globalThis.__TAURI__)return globalThis.__TAURI__.core.invoke('workspace_spaces');
  await this.open();return new Promise((resolve,reject)=>{const transaction=this.database.transaction('workspace','readonly');const request=transaction.objectStore('workspace').getAllKeys();transaction.oncomplete=()=>resolve(request.result.filter(key=>typeof key==='string'&&/^space:[a-z][a-z0-9_-]{0,31}:current$/.test(key)).map(key=>key.split(':')[1]));transaction.onabort=()=>reject(transaction.error);});
 }
 async mediaRequest(action,payload){
  const response=await fetch('/api/media/'+action,{method:'POST',headers:{'Content-Type':'application/json','X-Localization-Space':this.spaceId},body:JSON.stringify(payload)});
  const result=await response.json();if(!response.ok){const error=new Error(result.error||'预览视频处理失败');error.status=response.status;throw error;}if(result.url&&this.spaceId)result.url+='?space='+encodeURIComponent(this.spaceId);return result;
 }
 async finishMediaImport(record){
  if(!this.pendingMediaImport)return record;
  try{const result=await this.mediaRequest('commit',this.pendingMediaImport);this.pendingMediaTerminal=false;record.mediaUpdated=true;if(result.cleanupPending){record.mediaWarning='新视频已导入，但旧媒体尚未全部清理；下次保存或重启会重试。';}else this.pendingMediaImport=null;}
  catch(error){this.pendingMediaTerminal=[404,409,410].includes(error.status);record.mediaWarning=this.pendingMediaTerminal?'译文已保存，视频暂存已失效或存在版本冲突。请重新导入 ZIP；确认新导入会放弃旧的未完成媒体事务，不影响已保存译文。':'翻译数据已保存，但视频替换尚未完成：'+error.message+'；下次保存或重启将重试，旧视频暂未清理。';}
  return record;
 }
 async open(){
  if(this.database)return;
  this.database=await new Promise((resolve,reject)=>{
   const request=indexedDB.open('mida-localization-editor-workspace',1);
   request.onupgradeneeded=()=>{request.result.createObjectStore('workspace');request.result.createObjectStore('backups');};
   request.onerror=()=>reject(request.error);
   request.onblocked=()=>reject(new Error('本地存储被其他页面占用，请关闭其他编辑器页面后重试'));
   request.onsuccess=()=>resolve(request.result);
  });
  this.database.onversionchange=()=>{this.database.close();this.database=null;};
 }
 async read(store,key){
  if(globalThis.__TAURI__)return globalThis.__TAURI__.core.invoke('workspace_read',{store,key,spaceId:this.spaceId}).catch(error=>{throw new Error(String(error));});
  await this.open();
  return new Promise((resolve,reject)=>{
   const transaction=this.database.transaction(store,'readonly');
   const request=transaction.objectStore(store).get(this.storageKey(key));
   transaction.oncomplete=()=>resolve(request.result);
   transaction.onabort=()=>reject(transaction.error||new Error('读取本地保存失败'));
  });
 }
 async expandRecord(record){
  if(!record?.snapshot?.taskRefs)return record;
  const refs=record.snapshot.taskRefs;
  const tasks=await Promise.all(refs.map(async ref=>{
   const task=await this.read('workspace','task:'+ref.blob);
   if(!task||task.partName!==ref.partName||task.language!==ref.language||task.entries?.length!==ref.entryCount)throw new Error('片段存档缺失或与索引不一致，未覆盖已有内容');
   return task;
  }));
  record.snapshot.tasks=tasks;delete record.snapshot.taskRefs;return record;
 }
 splitSnapshot(snapshot){
  const compact=structuredClone(snapshot),blobs=[];
  compact.taskRefs=compact.tasks.map(task=>{const blob=localizationUuid();blobs.push({blob,task});return {blob,partName:task.partName,language:task.language,entryCount:task.entries.length};});
  delete compact.tasks;return {compact,blobs};
 }
 async saveTask(task,taskIndex,projectId,view,confirmedKeys){
  if(this.spaceId&&task.language!==this.spaceId)throw new Error('片段目标语言不匹配');
  for(const entry of task.entries)LocalizationWorkspaceStore.readEnglishSnapshot(entry,task.language);
  if(globalThis.__TAURI__){
   const result=await globalThis.__TAURI__.core.invoke('workspace_save_task',{task,taskIndex,projectId,view,confirmedKeys,expectedRevision:this.revision,spaceId:this.spaceId,expectedCacheEpoch:this.cacheEpoch}).catch(error=>{throw new Error(String(error));});
   this.revision=result.revision;return result;
  }
  await this.open();const expectedRevision=this.revision,confirmed=new Set(confirmedKeys);
  const result=await new Promise((resolve,reject)=>{
   let transaction;try{transaction=this.database.transaction('workspace','readwrite',{durability:'strict'});}catch{transaction=this.database.transaction('workspace','readwrite');}
   const workspace=transaction.objectStore('workspace');
   const current=workspace.get(this.storageKey('current')),generation=workspace.get('cache-generation'),protectedRefs=workspace.get(this.storageKey('task-backup-refs'));
   let reads=0,result=null,failure=null;
   const update=()=>{
    if(++reads!==3)return;
    try{
     const previous=current.result;
     if((generation.result||0)!==this.cacheEpoch)throw new Error('缓存已被清空，请刷新页面');
     if(previous?.revision!==expectedRevision)throw new Error('另一个编辑器已保存更新，未覆盖当前片段');
     if(previous.snapshot.currentProjectId!==projectId)throw new Error('片段不属于当前项目');
     if(!previous.snapshot.taskRefs){const {compact,blobs}=this.splitSnapshot(previous.snapshot);for(const {blob,task} of blobs)workspace.put(task,this.storageKey('task:'+blob));previous.snapshot=compact;}
     const snapshot=previous.snapshot,ref=snapshot.taskRefs[taskIndex];
     if(!ref||ref.partName!==task.partName||ref.language!==task.language||ref.entryCount!==task.entries.length)throw new Error('片段身份或词条数量发生变化，请重新导入');
     const blob=localizationUuid();workspace.put(task,this.storageKey('task:'+blob));
     snapshot.taskRefs[taskIndex]={...ref,blob};Object.assign(snapshot,view);snapshot.drafts=snapshot.drafts.filter(draft=>draft.taskIndex!==taskIndex||!confirmed.has(draft.key));
     result={...previous,savedAt:Date.now(),revision:expectedRevision+1};workspace.put(result,this.storageKey('current'));
     if(!(protectedRefs.result||[]).includes(ref.blob))workspace.delete(this.storageKey('task:'+ref.blob));
    }catch(error){failure=error;transaction.abort();}
   };
   current.onsuccess=update;generation.onsuccess=update;protectedRefs.onsuccess=update;
   transaction.oncomplete=()=>resolve({revision:result.revision,savedAt:result.savedAt});
   transaction.onabort=()=>reject(failure||transaction.error||new Error('片段保存失败'));
  });
  this.revision=result.revision;return result;
 }
 async load(){this.cacheEpoch=globalThis.__TAURI__?await globalThis.__TAURI__.core.invoke('workspace_cache_epoch'):(await this.read('workspace','cache-generation'))||0;const record=await this.expandRecord(await this.read('workspace','current'));this.revision=record?.revision||0;this.pendingMediaImport=null;this.pendingMediaTerminal=false;if(LocalizationWorkspaceStore.isLegacyDemo(record?.snapshot))return null;if(!globalThis.__TAURI__&&record){this.pendingMediaImport=record.snapshot.mediaImportPending||null;return this.finishMediaImport(record);}return record;}
 async history(){const history=(await this.read('workspace','history'))||[];const visible=await Promise.all(history.map(async summary=>{const record=await this.read('backups',summary.id);return LocalizationWorkspaceStore.isLegacyDemo(record?.snapshot)?null:summary;}));return visible.filter(Boolean);}
 async backup(id){const record=await this.expandRecord(await this.read('backups',id));return LocalizationWorkspaceStore.isLegacyDemo(record?.snapshot)?null:record;}
 async save(snapshot,backupReason=null,mediaImportToken=null){
  if(!Array.isArray(snapshot?.tasks)||!snapshot.tasks.length||snapshot.tasks.length>1000)throw new Error('自动保存任务数量必须为 1 至 1000');
  for(const task of snapshot.tasks)for(const entry of task.entries)LocalizationWorkspaceStore.readEnglishSnapshot(entry,task.language);
  if(LocalizationWorkspaceStore.isLegacyDemo(snapshot))throw new Error('请先导入 Unity 导出的 ZIP，空白或旧示例工作区不会保存');
  const languages=new Set(snapshot.tasks.map(task=>task.language));if(languages.size!==1||(this.spaceId&&![...languages].every(language=>language===this.spaceId)))throw new Error('当前空间只允许保存一种匹配的目标语言');
  if(globalThis.__TAURI__){const result=await globalThis.__TAURI__.core.invoke('workspace_save',{snapshot,expectedRevision:this.revision,backupReason,mediaImportToken,spaceId:this.spaceId,expectedCacheEpoch:this.cacheEpoch}).catch(error=>{throw new Error(String(error));});this.revision=result.revision;const warnings=[result.mediaCleanupWarning,result.mediaStagingWarning].filter(Boolean);if(warnings.length)result.mediaWarning='数据已保存，媒体清理尚未全部完成：'+warnings.join('；');return result;}
  if(this.pendingMediaImport&&mediaImportToken&&this.pendingMediaImport.token!==mediaImportToken){const retry=await this.finishMediaImport({});if(retry.mediaWarning&&!this.pendingMediaTerminal)throw new Error(retry.mediaWarning);if(this.pendingMediaTerminal){await this.mediaRequest('discard',{token:this.pendingMediaImport.token}).catch(()=>{});this.pendingMediaImport=null;this.pendingMediaTerminal=false;}}
  const pendingImport=mediaImportToken?{token:mediaImportToken,projectId:snapshot.currentProjectId}:this.pendingMediaImport;
  snapshot=structuredClone(snapshot);if(pendingImport)snapshot.mediaImportPending=pendingImport;else delete snapshot.mediaImportPending;
  await this.open();
  const {compact,blobs}=this.splitSnapshot(snapshot);
  const expectedRevision=this.revision;
  const result=await new Promise((resolve,reject)=>{
   let transaction;
   try{transaction=this.database.transaction(['workspace','backups'],'readwrite',{durability:'strict'});}
   catch{transaction=this.database.transaction(['workspace','backups'],'readwrite');}
   const workspace=transaction.objectStore('workspace');
   const backups=transaction.objectStore('backups');
   const currentRequest=workspace.get(this.storageKey('current'));
   const historyRequest=workspace.get(this.storageKey('history'));
   const generationRequest=workspace.get('cache-generation');
   let reads=0,result=null,failure=null;
   const update=()=>{
    if(++reads!==3)return;
    try{
     const previous=currentRequest.result,history=historyRequest.result||[];
     if((generationRequest.result||0)!==this.cacheEpoch)throw new Error('缓存已被清空，旧页面不会重新保存数据；请刷新页面');
     if((previous?.revision||0)!==expectedRevision)throw new Error('另一个编辑器页面已保存更新，已暂停本页保存以避免覆盖；请保留本页内容并关闭其他页面后重新打开');
     const replacingDemo=LocalizationWorkspaceStore.isLegacyDemo(previous?.snapshot);
     if(replacingDemo)workspace.put(previous,this.storageKey('legacy-demo-current'));
     for(const {blob,task} of blobs)workspace.put(task,this.storageKey('task:'+blob));
     const now=Date.now();let lastBackupAt=previous?.lastBackupAt||0;
     if(backupReason||!previous||replacingDemo){
      const savedSnapshot=compact;
      const savedAt=backupReason||!previous||replacingDemo?now:previous.savedAt;
      const id=localizationUuid();
      backups.put({snapshot:savedSnapshot,savedAt},this.storageKey(id));
      history.unshift({id,savedAt,reason:backupReason||'首次保存',taskCount:savedSnapshot.taskRefs.length});
      for(const removed of history.splice(10))backups.delete(this.storageKey(removed.id));
      workspace.put(history,this.storageKey('history'));lastBackupAt=now;
     }
     result={snapshot:compact,savedAt:now,lastBackupAt,revision:expectedRevision+1};
     workspace.put(result,this.storageKey('current'));
     // Keep only task blobs referenced by current data and the retained backups.
     const summaries=history.slice(),protectedBlobs=new Set();let remaining=summaries.length;
     const collect=()=>{
      workspace.put([...protectedBlobs],this.storageKey('task-backup-refs'));
      const keep=new Set([...protectedBlobs,...compact.taskRefs.map(ref=>ref.blob)]),prefix=this.storageKey('task:');
      const keys=workspace.getAllKeys();keys.onsuccess=()=>{for(const key of keys.result)if(typeof key==='string'&&key.startsWith(prefix)&&!keep.has(key.slice(prefix.length)))workspace.delete(key);};
     };
     if(!remaining)collect();
     for(const summary of summaries){const request=backups.get(this.storageKey(summary.id));request.onsuccess=()=>{for(const ref of request.result?.snapshot?.taskRefs||[])protectedBlobs.add(ref.blob);if(!--remaining)collect();};}

    }catch(error){failure=error;transaction.abort();}
   };
   currentRequest.onsuccess=update;historyRequest.onsuccess=update;generationRequest.onsuccess=update;
   transaction.oncomplete=()=>resolve(result);
   transaction.onabort=()=>reject(failure||transaction.error||new Error('本地保存事务未完成'));
  });
  this.revision=result.revision;
  this.pendingMediaImport=pendingImport;
  return this.finishMediaImport({...result,snapshot});
 }
}
