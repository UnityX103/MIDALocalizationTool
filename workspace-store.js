function localizationUuid(){
 if(globalThis.crypto.randomUUID)return crypto.randomUUID();
 const bytes=crypto.getRandomValues(new Uint8Array(16));bytes[6]=(bytes[6]&15)|64;bytes[8]=(bytes[8]&63)|128;
 const hex=[...bytes].map(value=>value.toString(16).padStart(2,'0')).join('');
 return hex.slice(0,8)+'-'+hex.slice(8,12)+'-'+hex.slice(12,16)+'-'+hex.slice(16,20)+'-'+hex.slice(20);
}
class LocalizationWorkspaceStore {
 static isLegacyDemo(snapshot){return Boolean(snapshot)&&(typeof snapshot.currentProjectId!=='string'||!snapshot.currentProjectId.trim()||snapshot.fileVersion?.lineageId==='demo-task-package'||snapshot.currentPackageId==='demo-import-v1');}
 constructor(){this.database=null;this.revision=0;this.pendingMediaImport=null;this.pendingMediaTerminal=false;}
 async mediaRequest(action,payload){
  const response=await fetch('/api/media/'+action,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(payload)});
  const result=await response.json();if(!response.ok){const error=new Error(result.error||'预览视频处理失败');error.status=response.status;throw error;}return result;
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
   const request=indexedDB.open('mida-localization-workspace',1);
   request.onupgradeneeded=()=>{request.result.createObjectStore('workspace');request.result.createObjectStore('backups');};
   request.onerror=()=>reject(request.error);
   request.onblocked=()=>reject(new Error('本地存储被其他页面占用，请关闭其他编辑器页面后重试'));
   request.onsuccess=()=>resolve(request.result);
  });
  this.database.onversionchange=()=>{this.database.close();this.database=null;};
 }
 async read(store,key){
  if(globalThis.__TAURI__)return globalThis.__TAURI__.core.invoke('workspace_read',{store,key}).catch(error=>{throw new Error(String(error));});
  await this.open();
  return new Promise((resolve,reject)=>{
   const transaction=this.database.transaction(store,'readonly');
   const request=transaction.objectStore(store).get(key);
   transaction.oncomplete=()=>resolve(request.result);
   transaction.onabort=()=>reject(transaction.error||new Error('读取本地保存失败'));
  });
 }
 async load(){const record=await this.read('workspace','current');this.revision=record?.revision||0;this.pendingMediaImport=null;this.pendingMediaTerminal=false;if(LocalizationWorkspaceStore.isLegacyDemo(record?.snapshot))return null;if(!globalThis.__TAURI__&&record){this.pendingMediaImport=record.snapshot.mediaImportPending||null;return this.finishMediaImport(record);}return record;}
 async history(){const history=(await this.read('workspace','history'))||[];const visible=await Promise.all(history.map(async summary=>{const record=await this.read('backups',summary.id);return LocalizationWorkspaceStore.isLegacyDemo(record?.snapshot)?null:summary;}));return visible.filter(Boolean);}
 async backup(id){const record=await this.read('backups',id);return LocalizationWorkspaceStore.isLegacyDemo(record?.snapshot)?null:record;}
 async save(snapshot,backupReason=null,mediaImportToken=null){
  if(!Array.isArray(snapshot?.tasks)||!snapshot.tasks.length||snapshot.tasks.length>1000)throw new Error('自动保存任务数量必须为 1 至 1000');
  if(LocalizationWorkspaceStore.isLegacyDemo(snapshot))throw new Error('请先导入 Unity 导出的 ZIP，空白或旧示例工作区不会保存');
  if(globalThis.__TAURI__){const result=await globalThis.__TAURI__.core.invoke('workspace_save',{snapshot,expectedRevision:this.revision,backupReason,mediaImportToken}).catch(error=>{throw new Error(String(error));});this.revision=result.revision;const warnings=[result.mediaCleanupWarning,result.mediaStagingWarning].filter(Boolean);if(warnings.length)result.mediaWarning='数据已保存，媒体清理尚未全部完成：'+warnings.join('；');return result;}
  if(this.pendingMediaImport&&mediaImportToken&&this.pendingMediaImport.token!==mediaImportToken){const retry=await this.finishMediaImport({});if(retry.mediaWarning&&!this.pendingMediaTerminal)throw new Error(retry.mediaWarning);if(this.pendingMediaTerminal){await this.mediaRequest('discard',{token:this.pendingMediaImport.token}).catch(()=>{});this.pendingMediaImport=null;this.pendingMediaTerminal=false;}}
  const pendingImport=mediaImportToken?{token:mediaImportToken,projectId:snapshot.currentProjectId}:this.pendingMediaImport;
  snapshot=structuredClone(snapshot);if(pendingImport)snapshot.mediaImportPending=pendingImport;else delete snapshot.mediaImportPending;
  await this.open();
  const expectedRevision=this.revision;
  const result=await new Promise((resolve,reject)=>{
   let transaction;
   try{transaction=this.database.transaction(['workspace','backups'],'readwrite',{durability:'strict'});}
   catch{transaction=this.database.transaction(['workspace','backups'],'readwrite');}
   const workspace=transaction.objectStore('workspace');
   const backups=transaction.objectStore('backups');
   const currentRequest=workspace.get('current');
   const historyRequest=workspace.get('history');
   let reads=0,result=null,failure=null;
   const update=()=>{
    if(++reads!==2)return;
    try{
     const previous=currentRequest.result,history=historyRequest.result||[];
     if((previous?.revision||0)!==expectedRevision)throw new Error('另一个编辑器页面已保存更新，已暂停本页保存以避免覆盖；请保留本页内容并关闭其他页面后重新打开');
     const replacingDemo=LocalizationWorkspaceStore.isLegacyDemo(previous?.snapshot);
     if(replacingDemo)workspace.put(previous,'legacy-demo-current');
     const now=Date.now();let lastBackupAt=previous?.lastBackupAt||0;
     if(backupReason||!previous||replacingDemo||now-lastBackupAt>=5*60*1000){
      const savedSnapshot=backupReason||!previous||replacingDemo?snapshot:previous.snapshot;
      const savedAt=backupReason||!previous||replacingDemo?now:previous.savedAt;
      const id=localizationUuid();
      backups.put({snapshot:savedSnapshot,savedAt},id);
      history.unshift({id,savedAt,reason:backupReason||'定时备份',taskCount:savedSnapshot.tasks.length});
      for(const removed of history.splice(10))backups.delete(removed.id);
      workspace.put(history,'history');lastBackupAt=now;
     }
     result={snapshot,savedAt:now,lastBackupAt,revision:expectedRevision+1};
     workspace.put(result,'current');
    }catch(error){failure=error;transaction.abort();}
   };
   currentRequest.onsuccess=update;historyRequest.onsuccess=update;
   transaction.oncomplete=()=>resolve(result);
   transaction.onabort=()=>reject(failure||transaction.error||new Error('本地保存事务未完成'));
  });
  this.revision=result.revision;
  this.pendingMediaImport=pendingImport;
  return this.finishMediaImport(result);
 }
}
