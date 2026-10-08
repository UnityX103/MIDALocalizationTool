function localizationUuid(){
 if(globalThis.crypto.randomUUID)return crypto.randomUUID();
 const bytes=crypto.getRandomValues(new Uint8Array(16));bytes[6]=(bytes[6]&15)|64;bytes[8]=(bytes[8]&63)|128;
 const hex=[...bytes].map(value=>value.toString(16).padStart(2,'0')).join('');
 return hex.slice(0,8)+'-'+hex.slice(8,12)+'-'+hex.slice(12,16)+'-'+hex.slice(16,20)+'-'+hex.slice(20);
}
class LocalizationWorkspaceStore {
 static allowsEmptyTranslation(task,entry){
  return task?.sourceKind==='unity-minigame'&&task.assetProtocolVersion===1&&entry?.allowEmpty===true;
 }
 static acceptsTranslation(task,entry,text=entry.translation){
  return typeof text==='string'&&(this.hasText(text)||this.allowsEmptyTranslation(task,entry));
 }
 static hasText(text){return typeof text==='string'&&/[^\u0009-\u000d\u001c-\u0020\u0085\u00a0\u1680\u2000-\u200a\u2028\u2029\u202f\u205f\u3000\ufeff]/u.test(text);}
 static readEnglishSnapshot(entry,language){
  if(Object.hasOwn(entry,'englishTranslationAtExport')){
   if(typeof entry.englishTranslationAtExport!=='string')throw new Error('旧英文快照 englishTranslationAtExport 必须是文本');
   return entry.englishTranslationAtExport;
  }
  return language==='en'?entry.translationAtExport:'';
 }
 static isLegacyDemo(snapshot){return Boolean(snapshot)&&(typeof snapshot.currentProjectId!=='string'||!snapshot.currentProjectId.trim()||snapshot.fileVersion?.lineageId==='demo-task-package'||snapshot.currentPackageId==='demo-import-v1');}
 constructor(options={}){this.tutorial=options.tutorial===true;this.recoveryToken=null;this.database=null;this.revision=0;this.pendingMediaImport=null;this.pendingMediaTerminal=false;this.spaceId='';this.cacheEpoch=0;}
 get native(){return Boolean(globalThis.__TAURI__)&&!this.tutorial;}
 static validSpace(value){return typeof value==='string'&&/^[a-z][a-z0-9_-]{0,31}$/.test(value);}
 storageKey(key){return this.spaceId&&key!=='cache-generation'?'space:'+this.spaceId+':'+key:key;}
 async clearAll(){
  if(this.native){await MidaDiagnostics.invoke('clear_all_cache',{confirmed:true});return;}
  if(!this.tutorial){
  const response=await fetch('/api/cache/clear',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({confirmed:true})});
  const result=await response.json();if(!response.ok)throw new Error(result.error||'视频缓存清理失败');
  }
  await this.open();
  await new Promise((resolve,reject)=>{
   const transaction=this.database.transaction(['workspace','backups'],'readwrite');const workspace=transaction.objectStore('workspace');const generation=workspace.get('cache-generation');
   generation.onsuccess=()=>{workspace.clear();transaction.objectStore('backups').clear();workspace.put((generation.result||0)+1,'cache-generation');};
   transaction.oncomplete=resolve;transaction.onabort=()=>reject(transaction.error||new Error('清空本地存档失败'));
  });
 }
 async spaces(){
  if(this.tutorial)return [];
  if(this.native)return MidaDiagnostics.invoke('workspace_spaces');
  await this.open();return new Promise((resolve,reject)=>{const transaction=this.database.transaction('workspace','readonly');const request=transaction.objectStore('workspace').getAllKeys();transaction.oncomplete=()=>resolve(request.result.filter(key=>typeof key==='string'&&/^space:[a-z][a-z0-9_-]{0,31}:current$/.test(key)).map(key=>key.split(':')[1]));transaction.onabort=()=>reject(transaction.error);});
 }
 static mediaClient(){
  if(this._mediaClient)return this._mediaClient;const key='mida-localization-editor-media-client';let id=null;try{id=localStorage.getItem(key);}catch{}
  if(!/^[0-9a-f]{32}$/.test(id||'')){id=localizationUuid().replaceAll('-','');try{localStorage.setItem(key,id);}catch{}}
  this._mediaClient=id;return id;
 }
 async reconcileMedia(){
  if(this.tutorial)return;
  if(this.native)return;
  await this.open();const records=await new Promise((resolve,reject)=>{const tx=this.database.transaction('workspace','readonly'),ws=tx.objectStore('workspace'),keys=ws.getAllKeys();const records=[];keys.onsuccess=()=>{for(const key of keys.result)if(typeof key==='string'&&(key==='current'||key.endsWith(':current')||key.includes('recovery:'))){const request=ws.get(key);request.onsuccess=()=>records.push({key,record:request.result});}};tx.oncomplete=()=>resolve(records);tx.onabort=()=>reject(tx.error);});
  const spaces=new Map([['',[]]]);for(const {key,record} of records){const space=key.startsWith('space:')?key.split(':')[1]:'';if(!spaces.has(space))spaces.set(space,[]);const token=record?.snapshot?.mediaImportPending?.token;if(token)spaces.get(space).push(token);}
  for(const [space,tokens] of spaces){const store=new LocalizationWorkspaceStore();store.spaceId=space;await store.mediaRequest('reconcile',{owner:LocalizationWorkspaceStore.mediaClient(),tokens:[...new Set(tokens)]});}
 }
 async mediaRequest(action,payload){
  if(this.tutorial)throw new Error('教程媒体与真实媒体缓存独立');
  const response=await fetch('/api/media/'+action,{method:'POST',headers:{'Content-Type':'application/json','X-Localization-Space':this.spaceId},body:JSON.stringify(payload),signal:AbortSignal.timeout(30000)});
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
   const request=indexedDB.open(this.tutorial?'mida-localization-editor-tutorial':'mida-localization-editor-workspace',1);
   request.onupgradeneeded=()=>{request.result.createObjectStore('workspace');request.result.createObjectStore('backups');};
   request.onerror=()=>reject(request.error);
   request.onblocked=()=>reject(new Error('本地存储被其他页面占用，请关闭其他编辑器页面后重试'));
   request.onsuccess=()=>resolve(request.result);
  });
  this.database.onversionchange=()=>{this.database.close();this.database=null;};
 }
 async read(store,key){
  if(this.native)return MidaDiagnostics.invoke('workspace_read',{store,key,spaceId:this.spaceId}).catch(error=>{throw new Error(String(error));});
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
 async saveEntry(task,taskIndex,projectId,entry,view,ledgerDelta){
  if(this.spaceId&&task.language!==this.spaceId)throw new Error('片段语言不匹配');
  if(this.native){const result=await MidaDiagnostics.invoke('workspace_save_entry',{task,taskIndex,projectId,entry,view,ledgerDelta,expectedRevision:this.revision,spaceId:this.spaceId,expectedCacheEpoch:this.cacheEpoch}).catch(error=>{throw new Error(String(error));});this.revision=result.revision;return result;}
  await this.open();const expectedRevision=this.revision;
  const result=await new Promise((resolve,reject)=>{
   let tx;try{tx=this.database.transaction('workspace','readwrite',{durability:'strict'});}catch{tx=this.database.transaction('workspace','readwrite');}const ws=tx.objectStore('workspace'),current=ws.get(this.storageKey('current')),generation=ws.get('cache-generation'),protectedRefs=ws.get(this.storageKey('task-backup-refs'));let reads=0,failure,result;
   const update=()=>{if(++reads!==3)return;try{
    const previous=current.result;if((generation.result||0)!==this.cacheEpoch||previous?.revision!==expectedRevision)throw new Error('存档已更改，未覆盖片段');
    if(previous.snapshot.currentProjectId!==projectId)throw new Error('片段项目不匹配');
    if(!previous.snapshot.taskRefs){const split=this.splitSnapshot(previous.snapshot);for(const {blob,task} of split.blobs)ws.put(task,this.storageKey('task:'+blob));previous.snapshot=split.compact;}
    const snapshot=previous.snapshot,ref=snapshot.taskRefs[taskIndex];if(!ref||ref.partName!==task.partName||ref.language!==task.language)throw new Error('片段身份不匹配');
    const request=ws.get(this.storageKey('task:'+ref.blob));request.onsuccess=()=>{try{
     const savedTask=request.result;if(!savedTask||savedTask.partName!==ref.partName||savedTask.language!==ref.language||savedTask.entries?.length!==ref.entryCount)throw new Error('片段存档缺失或损坏');
     const entries=Array.isArray(entry)?entry:[entry],byKey=new Map(savedTask.entries.map((item,index)=>[item.key,index]));for(const value of entries){const index=byKey.get(value.key);if(index===undefined)throw new Error('词条不属于片段');savedTask.entries[index]=value;LocalizationWorkspaceStore.readEnglishSnapshot(value,savedTask.language);}const confirmed=new Set(entries.map(value=>value.key));
     const ledger=snapshot.workLedger;if(ledger?.id!==ledgerDelta.id)throw new Error('台账身份不匹配');const ids=new Set(ledger.records.map(item=>item.id));for(const record of ledgerDelta.records){if(ids.has(record.id))throw new Error('工作记录重复');ledger.records.push(record);ids.add(record.id);}if(ledger.records.length>100000)throw new Error('请先归档工作量');
     const blob=localizationUuid();ws.put(savedTask,this.storageKey('task:'+blob));snapshot.taskRefs[taskIndex]={...ref,blob};Object.assign(snapshot,view);snapshot.drafts=snapshot.drafts.filter(draft=>draft.taskIndex!==taskIndex||!confirmed.has(draft.key));snapshot.importedTranslations=(snapshot.importedTranslations||[]).filter(candidate=>candidate.partName!==task.partName||candidate.language!==task.language||!confirmed.has(candidate.key));
     result={...previous,savedAt:Date.now(),revision:expectedRevision+1};if(!Number.isSafeInteger(result.revision))throw new Error('保存版本超限，请从备份恢复');ws.put(result,this.storageKey('current'));if(protectedRefs.result!==null&&!(protectedRefs.result||[]).includes(ref.blob))ws.delete(this.storageKey('task:'+ref.blob));
    }catch(error){failure=error;tx.abort();}};
   }catch(error){failure=error;tx.abort();}};current.onsuccess=update;generation.onsuccess=update;protectedRefs.onsuccess=update;
   tx.oncomplete=()=>resolve({savedAt:result.savedAt,revision:result.revision});tx.onabort=()=>reject(failure||tx.error||new Error('片段保存失败'));
  });this.revision=result.revision;return result;
 }
 async saveTask(task,taskIndex,projectId,view,confirmedKeys){
  if(this.spaceId&&task.language!==this.spaceId)throw new Error('片段目标语言不匹配');
  for(const entry of task.entries)LocalizationWorkspaceStore.readEnglishSnapshot(entry,task.language);
  if(this.native){
   const result=await MidaDiagnostics.invoke('workspace_save_task',{task,taskIndex,projectId,view,confirmedKeys,expectedRevision:this.revision,spaceId:this.spaceId,expectedCacheEpoch:this.cacheEpoch}).catch(error=>{throw new Error(String(error));});
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
     result={...previous,savedAt:Date.now(),revision:expectedRevision+1};if(!Number.isSafeInteger(result.revision))throw new Error('保存版本超限，请从备份恢复');workspace.put(result,this.storageKey('current'));
     if(protectedRefs.result!==null&&!(protectedRefs.result||[]).includes(ref.blob))workspace.delete(this.storageKey('task:'+ref.blob));
    }catch(error){failure=error;transaction.abort();}
   };
   current.onsuccess=update;generation.onsuccess=update;protectedRefs.onsuccess=update;
   transaction.oncomplete=()=>resolve({revision:result.revision,savedAt:result.savedAt});
   transaction.onabort=()=>reject(failure||transaction.error||new Error('片段保存失败'));
  });
  this.revision=result.revision;return result;
 }
 async load(){let mediaWarning=null;if(!this.native)try{await this.reconcileMedia();}catch(error){mediaWarning='媒体暂存核对未完成：'+error.message;}this.cacheEpoch=this.native?await MidaDiagnostics.invoke('workspace_cache_epoch'):(await this.read('workspace','cache-generation'))||0;const record=await this.expandRecord(await this.read('workspace','current'));if(record&&(!Number.isSafeInteger(record.revision)||record.revision<1))throw new Error('存档保存版本无效，请从备份恢复');await this.loadArchives(record?.snapshot);this.revision=record?.revision||0;this.pendingMediaImport=null;this.pendingMediaTerminal=false;if(LocalizationWorkspaceStore.isLegacyDemo(record?.snapshot))return null;if(record&&this.tutorial!==Boolean(record.snapshot.currentProjectId==='mida-editor-tutorial-v1'))throw new Error('教程包与真实工作空间不能混用');if(record&&mediaWarning)record.mediaWarning=mediaWarning;if(!this.native&&record){this.pendingMediaImport=record.snapshot.mediaImportPending||null;return this.finishMediaImport(record);}return record;}
 async history(){
  if(this.native)return this.read('workspace','history');
  await this.open();const keys=await new Promise((resolve,reject)=>{const tx=this.database.transaction('backups','readonly'),request=tx.objectStore('backups').getAllKeys();tx.oncomplete=()=>resolve(request.result);tx.onabort=()=>reject(tx.error);});
  const prefix=this.storageKey('');const summaries=await Promise.all(keys.filter(key=>typeof key==='string'&&key.startsWith(prefix)&&!key.slice(prefix.length).includes(':')).map(async key=>{
   const id=key.slice(prefix.length);try{const record=await this.backup(id);if(!record)return null;return {id,savedAt:record.savedAt,reason:record.reason||'本机备份',taskCount:record.snapshot.tasks.length};}catch(error){return {id,savedAt:0,reason:'不可用备份',taskCount:0,unavailable:error.message};}
  }));return summaries.filter(Boolean).sort((a,b)=>b.savedAt-a.savedAt);
 }
 async archiveLedger(archive){
  if(this.native)return MidaDiagnostics.invoke('workspace_archive_ledger',{archive,spaceId:this.spaceId,expectedRevision:this.revision,expectedCacheEpoch:this.cacheEpoch});
  await this.open();const compact=structuredClone(archive),blobs=[];for(const packet of compact.packets){const split=this.splitSnapshot(packet);blobs.push(...split.blobs);Object.assign(packet,split.compact);delete packet.tasks;delete packet.workload.records;}
  await new Promise((resolve,reject)=>{const tx=this.database.transaction('workspace','readwrite'),ws=tx.objectStore('workspace'),request=ws.get(this.storageKey('current')),generation=ws.get('cache-generation'),archiveRefs=ws.get(this.storageKey('task-archive-refs')),backupRefs=ws.get(this.storageKey('task-backup-refs'));let reads=0,failure;const write=()=>{if(++reads!==4)return;try{if(request.result?.revision!==this.revision||(generation.result||0)!==this.cacheEpoch)throw new Error('存档已更改，归档未提交');for(const {blob,task} of blobs)ws.put(task,this.storageKey('task:'+blob));ws.put(compact,this.storageKey('ledger:'+archive.id));const refs=[...new Set([...(archiveRefs.result||[]),...blobs.map(item=>item.blob)])];ws.put(refs,this.storageKey('task-archive-refs'));ws.put(backupRefs.result===null?null:[...new Set([...(backupRefs.result||[]),...refs])],this.storageKey('task-backup-refs'));}catch(error){failure=error;tx.abort();}};request.onsuccess=write;generation.onsuccess=write;archiveRefs.onsuccess=write;backupRefs.onsuccess=write;tx.oncomplete=resolve;tx.onabort=()=>reject(failure||tx.error);});
 }
 async readArchive(id,ledgerOnly=false){
  const archive=this.native?await MidaDiagnostics.invoke('workspace_read',{store:ledgerOnly?'ledger-index':'ledgers',key:id,spaceId:this.spaceId}):await this.read('workspace','ledger:'+id);
  if(!archive)throw new Error('归档文件缺失：'+id);if(ledgerOnly){delete archive.packets;return LocalizationWorkload.registerArchive(archive);}LocalizationWorkload.registerArchive(archive);for(const packet of archive.packets){if(packet.taskRefs){const record=await this.expandRecord({snapshot:packet});Object.assign(packet,record.snapshot);}const delivery=archive.ledger.deliveries.find(value=>value.id===packet.delivery.id);if(!delivery)throw new Error('归档交付缺失');const byId=new Map(archive.ledger.records.map(record=>[record.id,record]));packet.workload.records=delivery.recordIds.map(id=>byId.get(id));}
  return archive;
 }
 async loadArchives(snapshot){for(const item of snapshot?.workLedger?.archives||[])await this.readArchive(item.id,true);}
 async recoveryInfo(){
  this.cacheEpoch=this.native?await MidaDiagnostics.invoke('workspace_cache_epoch'):(await this.read('workspace','cache-generation'))||0;
  if(this.native){const info=await MidaDiagnostics.invoke('workspace_recovery',{spaceId:this.spaceId});this.recoveryToken=info.token;return info;}
  const current=await this.read('workspace','current');this.recoveryToken=JSON.stringify(current??null);let expanded=null;try{expanded=await this.expandRecord(structuredClone(current));}catch{}
  return {current:expanded||current,history:await this.history()};
 }
 async recover(snapshot,extraSnapshots=[]){
  if(this.tutorial!==Boolean(snapshot.currentProjectId==='mida-editor-tutorial-v1'))throw new Error('教程包与真实工作空间不能混用');
  LocalizationWorkspaceStore.validateCapacity(snapshot);
  if(this.native){const record=await MidaDiagnostics.invoke('workspace_recover',{snapshot,token:this.recoveryToken,spaceId:this.spaceId,expectedCacheEpoch:this.cacheEpoch});this.revision=record.revision;this.cacheEpoch=record.cacheEpoch;return record;}
  await this.open();const snapshots=[snapshot,...extraSnapshots];snapshots.forEach(LocalizationWorkspaceStore.validateCapacity);const splits=snapshots.map(value=>this.splitSnapshot(value));
  const result=await new Promise((resolve,reject)=>{
   let tx;try{tx=this.database.transaction(['workspace','backups'],'readwrite',{durability:'strict'});}catch{tx=this.database.transaction(['workspace','backups'],'readwrite');}
   const ws=tx.objectStore('workspace'),bs=tx.objectStore('backups'),current=ws.get(this.storageKey('current')),generation=ws.get('cache-generation');let reads=0,failure=null,result;
   const update=()=>{if(++reads!==2)return;try{
    if((generation.result||0)!==this.cacheEpoch||JSON.stringify(current.result??null)!==this.recoveryToken)throw new Error('存档已发生变化，请重新打开恢复列表');
    ws.put(current.result??null,this.storageKey('recovery:'+localizationUuid()));const keys=ws.getAllKeys();keys.onsuccess=()=>{const prefix=this.storageKey('task:'),refs=keys.result.filter(key=>typeof key==='string'&&key.startsWith(prefix)).map(key=>key.slice(prefix.length));ws.put(refs,this.storageKey('task-quarantine-refs'));ws.put([...new Set([...refs,...splits.flatMap(split=>split.blobs.map(item=>item.blob))])],this.storageKey('task-backup-refs'));};
    const now=Date.now(),history=[];
    for(const {compact,blobs} of splits){for(const {blob,task} of blobs)ws.put(task,this.storageKey('task:'+blob));const id=localizationUuid();bs.put({snapshot:compact,savedAt:now,reason:'恢复或拆分存档',recoveryGroup:snapshots.length>1},this.storageKey(id));history.push({id,savedAt:now,reason:'恢复或拆分存档',taskCount:compact.taskRefs.length});}
    if(snapshots.length>1){const retained=ws.get(this.storageKey('backup-retained-ids'));retained.onsuccess=()=>ws.put([...new Set([...(retained.result||[]),...history.map(item=>item.id)])],this.storageKey('backup-retained-ids'));}
    const oldRevision=Number.isSafeInteger(current.result?.revision)&&current.result.revision>=0&&current.result.revision<Number.MAX_SAFE_INTEGER?current.result.revision:0;if(!Number.isSafeInteger(oldRevision+1))throw new Error('存档版本超限');
    const cacheEpoch=Number.isSafeInteger(generation.result||0)&&(generation.result||0)<Number.MAX_SAFE_INTEGER?(generation.result||0)+1:localizationUuid();ws.put(cacheEpoch,'cache-generation');result={snapshot:splits[0].compact,savedAt:now,lastBackupAt:now,revision:oldRevision+1,cacheEpoch};ws.put(result,this.storageKey('current'));ws.put(history,this.storageKey('history'));
   }catch(error){failure=error;tx.abort();}};current.onsuccess=update;generation.onsuccess=update;
   tx.oncomplete=()=>resolve({...result,snapshot});tx.onabort=()=>reject(failure||tx.error||new Error('恢复未提交'));
  });this.revision=result.revision;this.cacheEpoch=result.cacheEpoch;return result;
 }
 static validateCapacity(snapshot){
  if(!Array.isArray(snapshot?.tasks)||!snapshot.tasks.length||snapshot.tasks.length>1000)throw new Error('工作区任务数量必须为 1 至 1000');
  let count=0;for(const task of snapshot.tasks){if(!Array.isArray(task.entries)||!task.entries.length)throw new Error('任务词条无效');count+=task.entries.length;if(count>100000)throw new Error('合并后工作区超过 100000 条对话；请缩小本次导入范围或使用其他工作区，原数据和视频未被覆盖');}
 }

 async backup(id){const record=await this.expandRecord(await this.read('backups',id));await this.loadArchives(record?.snapshot);return LocalizationWorkspaceStore.isLegacyDemo(record?.snapshot)?null:record;}
 async save(snapshot,backupReason=null,mediaImportToken=null){
  if(this.tutorial!==Boolean(snapshot.currentProjectId==='mida-editor-tutorial-v1'))throw new Error('教程包与真实工作空间不能混用');
  LocalizationWorkspaceStore.validateCapacity(snapshot);
  for(const task of snapshot.tasks)for(const entry of task.entries)LocalizationWorkspaceStore.readEnglishSnapshot(entry,task.language);
  if(LocalizationWorkspaceStore.isLegacyDemo(snapshot))throw new Error('请先导入 Unity 导出的 ZIP，空白或旧示例工作区不会保存');
  const languages=new Set(snapshot.tasks.map(task=>task.language));if(languages.size!==1||(this.spaceId&&![...languages].every(language=>language===this.spaceId)))throw new Error('当前空间只允许保存一种匹配的目标语言');
  if(this.native){const result=await MidaDiagnostics.invoke('workspace_save',{snapshot,expectedRevision:this.revision,backupReason,mediaImportToken,spaceId:this.spaceId,expectedCacheEpoch:this.cacheEpoch}).catch(error=>{throw new Error(String(error));});this.revision=result.revision;const warnings=[result.mediaCleanupWarning,result.mediaStagingWarning].filter(Boolean);if(warnings.length)result.mediaWarning='数据已保存，媒体清理尚未全部完成：'+warnings.join('；');return result;}
  if(this.pendingMediaImport&&mediaImportToken&&this.pendingMediaImport.token!==mediaImportToken){const retry=await this.finishMediaImport({});if(retry.mediaWarning&&!this.pendingMediaTerminal)throw new Error(retry.mediaWarning);if(this.pendingMediaTerminal){await this.mediaRequest('discard',{token:this.pendingMediaImport.token}).catch(()=>{});this.pendingMediaImport=null;this.pendingMediaTerminal=false;}}
  const pendingImport=mediaImportToken?{token:mediaImportToken,projectId:snapshot.currentProjectId}:this.pendingMediaImport;
  snapshot=structuredClone(snapshot);if(pendingImport)snapshot.mediaImportPending=pendingImport;else delete snapshot.mediaImportPending;
  await this.open();
  const {compact,blobs}=this.splitSnapshot(snapshot);
  if(mediaImportToken)await this.mediaRequest('lease',{token:mediaImportToken,owner:LocalizationWorkspaceStore.mediaClient(),protect:true});
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
     const removedBackups=[];if(backupReason||!previous||replacingDemo){
      const savedSnapshot=compact;
      const savedAt=backupReason||!previous||replacingDemo?now:previous.savedAt;
      const id=localizationUuid();
      backups.put({snapshot:savedSnapshot,savedAt,reason:backupReason||'首次保存'},this.storageKey(id));
      history.unshift({id,savedAt,reason:backupReason||'首次保存',taskCount:savedSnapshot.taskRefs.length});
      removedBackups.push(...history.splice(10));
      workspace.put(history,this.storageKey('history'));lastBackupAt=now;
     }
     result={snapshot:compact,savedAt:now,lastBackupAt,revision:expectedRevision+1};if(!Number.isSafeInteger(result.revision))throw new Error('保存版本超限，请从备份恢复');
     workspace.put(result,this.storageKey('current'));
     // Keep only task blobs referenced by current data and the retained backups.
     const summaries=history.slice(),protectedBlobs=new Set(),quarantineBlobs=new Set(),unavailable=new Set(),retainedIds=new Set(),removedToDelete=[];let remaining=summaries.length+4+removedBackups.length,newUnknown=false;
     const archiveRefs=workspace.get(this.storageKey('task-archive-refs')),quarantineRefs=workspace.get(this.storageKey('task-quarantine-refs')),unavailableIds=workspace.get(this.storageKey('backup-unavailable-ids')),retained=workspace.get(this.storageKey('backup-retained-ids'));
     const collect=()=>{
      const prefix=this.storageKey('task:');
      const finish=keys=>{
       if(newUnknown)for(const key of keys)if(typeof key==='string'&&key.startsWith(prefix))quarantineBlobs.add(key.slice(prefix.length));
       for(const blob of quarantineBlobs)protectedBlobs.add(blob);
       for(const id of removedToDelete)if(!unavailable.has(id)&&!retainedIds.has(id))backups.delete(this.storageKey(id));workspace.put([...retainedIds],this.storageKey('backup-retained-ids'));
       workspace.put([...quarantineBlobs],this.storageKey('task-quarantine-refs'));workspace.put([...unavailable],this.storageKey('backup-unavailable-ids'));workspace.put([...protectedBlobs],this.storageKey('task-backup-refs'));
       const keep=new Set([...protectedBlobs,...compact.taskRefs.map(ref=>ref.blob)]);for(const key of keys)if(typeof key==='string'&&key.startsWith(prefix)&&!keep.has(key.slice(prefix.length)))workspace.delete(key);
      };
      const keys=workspace.getAllKeys();keys.onsuccess=()=>finish(keys.result);
     };
     archiveRefs.onsuccess=()=>{for(const blob of archiveRefs.result||[])protectedBlobs.add(blob);if(!--remaining)collect();};quarantineRefs.onsuccess=()=>{for(const blob of quarantineRefs.result||[])quarantineBlobs.add(blob);if(!--remaining)collect();};unavailableIds.onsuccess=()=>{for(const id of unavailableIds.result||[])unavailable.add(id);if(!--remaining)collect();};
     retained.onsuccess=()=>{for(const id of retained.result||[])retainedIds.add(id);const extra=[...retainedIds].filter(id=>!summaries.some(summary=>summary.id===id));remaining+=extra.length;for(const id of extra){const request=backups.get(this.storageKey(id));request.onsuccess=()=>{for(const ref of request.result?.snapshot?.taskRefs||[])protectedBlobs.add(ref.blob);if(!--remaining)collect();};}if(!--remaining)collect();};
     for(const summary of removedBackups){const request=backups.get(this.storageKey(summary.id));request.onsuccess=()=>{const record=request.result;if(record?.recoveryGroup)retainedIds.add(summary.id);else if(!record||(!Array.isArray(record.snapshot?.taskRefs)&&!Array.isArray(record.snapshot?.tasks))){if(!unavailable.has(summary.id))newUnknown=true;unavailable.add(summary.id);}else removedToDelete.push(summary.id);if(!--remaining)collect();};}
     for(const summary of summaries){const request=backups.get(this.storageKey(summary.id));request.onsuccess=()=>{if(!Array.isArray(request.result?.snapshot?.taskRefs)&&!request.result?.snapshot?.tasks){if(!unavailable.has(summary.id))newUnknown=true;unavailable.add(summary.id);}for(const ref of request.result?.snapshot?.taskRefs||[])protectedBlobs.add(ref.blob);if(!--remaining)collect();};}

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
