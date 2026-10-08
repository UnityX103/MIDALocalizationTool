'use strict';

// The wire format is shared with Unity; see docs/workload-protocol.md.
globalThis.LocalizationWorkload = (() => {
 const rule='han-v1',limit=100000;
 const archives=new Map(),indexes=new WeakMap();
 const identity=(task,entry)=>canonical([task.partName,task.language,entry.key,entry.currentSource]);
 const recordIdentity=record=>canonical([record.partName,record.language,record.key,record.sourceText]);
 const requireValue=(condition,message)=>{if(!condition)throw new Error('工作量清单：'+message);};
 const text=value=>LocalizationWorkspaceStore.hasText(value);
 const integer=(value,min=0)=>Number.isSafeInteger(value)&&value>=min;
 const copy=value=>structuredClone(value);
 const summary=workload=>{const {records,...result}=workload;return copy(result);};
 const canonical=value=>JSON.stringify(normalize(value));
 function normalize(value){
  if(Array.isArray(value))return value.map(normalize);
  if(value&&typeof value==='object')return Object.fromEntries(Object.keys(value).sort().map(key=>[key,normalize(value[key])]));
  return value;
 }
 async function hash(value){
  return [...new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(canonical(value))))].map(byte=>byte.toString(16).padStart(2,'0')).join('');
 }
 async function hashTasks(tasks){return Promise.all(tasks.map(({previewPartNames,...task})=>hash(task)));}
 async function fingerprint(document,taskHashes){
  const {newRecordIds,previousDeliveryDeltaChars,...stableWorkload}=document.workload;
  return hash({projectId:document.projectId,parentPackageId:document.parentPackageId,lineageId:document.fileVersion.lineageId,deliveryState:document.deliveryState,taskHashes:taskHashes||await hashTasks(document.tasks),workload:stableWorkload});
 }
 const counts=new Map();
 function count(source){
  if(counts.has(source))return counts.get(source);let result=0;
  for(const char of source){
   const point=char.codePointAt(0);
   if(point===0x3007||(point>=0x3400&&point<=0x4dbf)||(point>=0x4e00&&point<=0x9fff)||
      (point>=0xf900&&point<=0xfaff)||(point>=0x20000&&point<=0x2ebef)||
      (point>=0x2f800&&point<=0x2fa1f)||(point>=0x30000&&point<=0x323af))result++;
  }
  if(counts.size>=100000)counts.clear();counts.set(source,result);return result;
 }
 function strings(value,name,max=limit){
  requireValue(Array.isArray(value)&&value.length<=max&&value.every(text)&&new Set(value).size===value.length,name+'无效或重复');
  return value;
 }
 const recordValidation=new WeakMap(),deliveryValidation=new WeakSet();
 const freeze=value=>{if(value&&typeof value==='object'&&!Object.isFrozen(value)){for(const child of Object.values(value))freeze(child);Object.freeze(value);}return value;};
 function validateRecords(records){
  requireValue(Array.isArray(records)&&records.length<=limit,'工作记录超出上限');
  let cache=recordValidation.get(records);if(!cache||cache.length>records.length){cache={ids:new Set(),identities:new Set(),length:0};recordValidation.set(records,cache);}const {ids,identities}=cache;
  for(const record of records.slice(cache.length)){
   requireValue(record&&['id','partName','language','key','confirmedAt'].every(field=>text(record[field])),'工作记录身份缺失');
   requireValue(typeof record.sourceText==='string'&&typeof record.translation==='string','工作记录文本无效');
   const provenance=(record.sourcePackageId===null&&record.sourceRevision===null)||(text(record.sourcePackageId)&&integer(record.sourceRevision,1));
   requireValue(integer(record.chars)&&record.chars===count(record.sourceText)&&provenance&&integer(record.taskVersion,1),'工作记录字数或版本错误');
   requireValue(['translation','source_revision'].includes(record.kind),'未知工作类型');
   const identity=canonical([record.partName,record.language,record.key,record.sourceText]);
   requireValue(!ids.has(record.id)&&!identities.has(identity),'工作记录身份重复');
   ids.add(record.id);identities.add(identity);freeze(record);cache.length++;
  }
  return ids;
 }
 function validateReceipts(receipts,manifest){
  requireValue(Array.isArray(receipts)&&receipts.length<=10000,'交接回执列表无效');
  const ids=new Set();
  for(const receipt of receipts){
   requireValue(receipt&&receipt.version===1&&['id','projectId','lineageId','language','ledgerId','deliveryId','receivedAt'].every(field=>text(receipt[field])),'回执身份无效');
   requireValue(!ids.has(receipt.id),'回执 ID 重复');ids.add(receipt.id);
   strings(receipt.recordIds,'回执记录');
   if(manifest)requireValue(receipt.projectId===manifest.projectId&&receipt.lineageId===manifest.fileVersion.lineageId,'回执项目或谱系不匹配');
  }
 }
 function progress(tasks,isPending=entry=>entry.review.state!=='confirmed'){
  let progressChars=0,totalChars=0;
  for(const task of tasks)for(const entry of task.entries){
   const chars=count(entry.currentSource);totalChars+=chars;
   if(!isPending(entry)&&LocalizationWorkspaceStore.acceptsTranslation(task,entry))progressChars+=chars;
  }
  return {progressChars,totalChars};
 }
 function validatePackage(manifest,tasks,wireProgress=null){
  if(Object.hasOwn(manifest,'workReceipts'))validateReceipts(manifest.workReceipts,manifest);
  if(!Object.hasOwn(manifest,'delivery')&&!Object.hasOwn(manifest,'workload'))return;
  const {delivery,workload:w}=manifest;
  requireValue(delivery?.version===1&&text(delivery.id)&&delivery.id===manifest.packageId&&text(delivery.ledgerId)&&integer(delivery.revision,1)&&(delivery.previousId===null||text(delivery.previousId))&&delivery.previousId!==delivery.id,'交付身份无效');
  requireValue(w?.version===1&&w.countingRule===rule&&w.scope?.projectId===manifest.projectId&&w.scope.lineageId===manifest.fileVersion.lineageId&&text(w.scope.language),'统计范围或计数规则无效');
  const parts=new Set(strings(w.scope.partNames,'统计片段',200));requireValue(parts.size>0,'统计范围为空');
  const ids=validateRecords(w.records),byId=new Map(w.records.map(record=>[record.id,record]));
  for(const record of w.records)requireValue(parts.has(record.partName)&&record.language===w.scope.language,'记录不属于本次交付范围');
  for(const field of ['acknowledgedRecordIds','newRecordIds','sourceUpdateRecordIds'])requireValue(strings(w[field],field).every(id=>ids.has(id)),field+'包含未知记录');
  const sum=list=>list.reduce((total,id)=>total+byId.get(id).chars,0);
  const cumulative=sum([...ids]),ack=new Set(w.acknowledgedRecordIds);
  for(const [field,value] of Object.entries({
   cumulativeChars:cumulative,handoverChars:sum([...ids].filter(id=>!ack.has(id))),
   previousDeliveryDeltaChars:sum(w.newRecordIds),sinceSourceUpdateChars:sum(w.sourceUpdateRecordIds)
  }))requireValue(integer(w[field])&&w[field]===value,field+'与明细不一致');
  requireValue(integer(w.progressChars)&&integer(w.totalChars)&&w.progressChars<=w.totalChars,'当前进度无效');
  if(tasks){
   requireValue(tasks.every(task=>task.language===w.scope.language)&&canonical([...new Set(tasks.map(task=>task.partName))].sort())===canonical([...parts].sort()),'统计范围与任务不一致');
   const actual=wireProgress||progress(tasks);
   requireValue(actual.progressChars===w.progressChars&&actual.totalChars===w.totalChars,'当前进度与任务不一致');
  }
 }
 function create(){return {version:1,id:localizationUuid(),records:[],acknowledgedIds:[],receipts:[],deliveries:[],sources:[]};}
 function alignSources(ledger,tasks,manifest,historicalSources=ledger.sources){
  for(const task of tasks){
   let source=historicalSources.find(item=>item.partName===task.partName&&item.language===task.language&&item.taskVersion===task.taskVersion);
   if(!source){
    const known=manifest?.assets?.some(asset=>asset.type==='localization-dialogues'&&asset.partName===task.partName);
    source={partName:task.partName,language:task.language,packageId:known?manifest.packageId:null,revision:known?manifest.fileVersion.revision:null,taskVersion:task.taskVersion,startIndex:0};
   }
   const index=ledger.sources.findIndex(item=>item.partName===task.partName&&item.language===task.language);
   if(index<0)ledger.sources.push(copy(source));else ledger.sources[index]=copy(source);
  }
 }
 function validateLedger(ledger,archived=false){
  requireValue(ledger?.version===1&&text(ledger.id),'本地台账身份无效');
  const ids=validateRecords(ledger.records);
  requireValue(strings(ledger.acknowledgedIds,'已交接记录').every(id=>ids.has(id)),'已交接记录缺失');
  validateReceipts(ledger.receipts);
  requireValue(Array.isArray(ledger.deliveries)&&ledger.deliveries.length<=(archived?10010:10000),'交付历史过多或无效');
  const deliveries=new Set(),revisions=new Set(),byId=new Map(ledger.records.map(record=>[record.id,record]));
  for(const delivery of ledger.deliveries){
   requireValue(text(delivery.id)&&!deliveries.has(delivery.id)&&integer(delivery.revision,1)&&!revisions.has(delivery.revision)&&text(delivery.exportedAt)&&typeof delivery.fingerprint==='string'&&integer(delivery.fileRevision,1),'交付历史无效或序号冲突');
   if(!deliveryValidation.has(delivery)){requireValue(strings(delivery.recordIds,'交付历史记录').every(id=>ids.has(id)),'交付历史记录缺失');validatePackage({packageId:delivery.id,projectId:delivery.workload?.scope?.projectId,fileVersion:{lineageId:delivery.workload?.scope?.lineageId},
    delivery:{version:1,id:delivery.id,ledgerId:ledger.id,revision:delivery.revision,previousId:delivery.previousId},
    workload:{...delivery.workload,records:delivery.recordIds.map(id=>byId.get(id))}});freeze(delivery);deliveryValidation.add(delivery);}
   deliveries.add(delivery.id);revisions.add(delivery.revision);
  }
  requireValue(Array.isArray(ledger.sources)&&ledger.sources.length<=1000,'来源基线无效');
  requireValue(ledger.archives===undefined||(Array.isArray(ledger.archives)&&ledger.archives.every(item=>text(item.id)&&text(item.ledgerId)&&integer(item.chars)&&integer(item.recordCount)&&text(item.archivedAt))),'归档索引无效');
  const sources=new Set();
  for(const source of ledger.sources){
   const identity=canonical([source.partName,source.language]);
   const provenance=(source.packageId===null&&source.revision===null)||(text(source.packageId)&&integer(source.revision,1));
   requireValue(text(source.partName)&&text(source.language)&&provenance&&integer(source.taskVersion,1)&&integer(source.startIndex)&&source.startIndex<=ledger.records.length&&!sources.has(identity),'来源基线无效');
   sources.add(identity);
  }
 }
 function confirmed(ledger,task,entry,before,manifest,commit=true){
  // Existing confirmed material and unchanged reviews are not new translation labor.
  if(before.state==='confirmed'||before.translation===entry.translation)return false;
  let index=indexes.get(ledger);if(!index||index.size!==ledger.records.length){index=new Set(ledger.records.map(recordIdentity));indexes.set(ledger,index);}
  const key=identity(task,entry);if(index.has(key))return false;
  for(const item of ledger.archives||[]){const archive=archives.get(item.id);requireValue(archive,'归档尚未载入，请重新打开工作区');if(archive.identities.has(key))return false;}
  requireValue(ledger.records.length<limit,'工作记录已达上限，请先归档交接');
  let keys=index.entryKeys;if(!keys){keys=new Set(ledger.records.map(record=>canonical([record.partName,record.language,record.key])));index.entryKeys=keys;}const entryKey=canonical([task.partName,task.language,entry.key]);const previous=keys.has(entryKey)||(ledger.archives||[]).some(item=>archives.get(item.id).keys.has(entryKey));
  const source=ledger.sources.find(source=>source.partName===task.partName&&source.language===task.language&&source.taskVersion===task.taskVersion);
  requireValue(source,'缺少当前任务的来源基线');
  const record={
   id:localizationUuid(),partName:task.partName,language:task.language,key:entry.key,
   sourceText:entry.currentSource,translation:entry.translation,
   sourcePackageId:source.packageId,sourceRevision:source.revision,
   taskVersion:task.taskVersion,kind:previous?'source_revision':'translation',chars:count(entry.currentSource),confirmedAt:new Date().toISOString()
  };
  if(commit){ledger.records.push(record);index.add(key);keys.add(entryKey);}return record;
 }
 async function mergeIncoming(ledger,manifest,tasks,wireFingerprint=null){
  // Task normalization can mark imported confirmed lines pending again. The wire
  // snapshot was validated before normalization at the import boundary.
  validatePackage(manifest);
  let next=copy(ledger);
  for(const item of next.archives||[]){
   const archive=archives.get(item.id);requireValue(archive,'归档尚未载入，请重新打开工作区');
   const acknowledgedBits=item.acknowledgedBits?atob(item.acknowledgedBits):null;const acknowledged=new Set(acknowledgedBits?archive.ledger.records.filter((_,index)=>acknowledgedBits.charCodeAt(index>>3)&(1<<(index&7))).map(record=>record.id):archive.ledger.acknowledgedIds);item.receiptHashes||=[];
   for(const receipt of manifest.workReceipts||[]){if(receipt.ledgerId!==item.ledgerId)continue;
    const fingerprint=await hash(receipt),known=item.receiptHashes.find(value=>value.id===receipt.id),original=archive.ledger.receipts.find(value=>value.id===receipt.id);
    if(known){requireValue(known.hash===fingerprint,'归档回执 ID 内容冲突');continue;}if(original){requireValue(canonical(original)===canonical(receipt),'归档回执 ID 内容冲突');continue;}
    const delivery=archive.ledger.deliveries.find(value=>value.id===receipt.deliveryId);if(!delivery){item.receiptHashes.push({id:receipt.id,hash:fingerprint});continue;}
    requireValue(receipt.projectId===delivery.workload.scope.projectId&&receipt.lineageId===delivery.workload.scope.lineageId&&receipt.language===delivery.workload.scope.language,'归档回执范围不匹配');
    const eligible=new Set(delivery.recordIds);requireValue(receipt.recordIds.every(id=>eligible.has(id)),'归档回执包含未交付记录');
    receipt.recordIds.forEach(id=>acknowledged.add(id));item.receiptHashes.push({id:receipt.id,hash:fingerprint});
   }
   const bits=new Uint8Array(Math.ceil(archive.ledger.records.length/8));archive.ledger.records.forEach((record,index)=>{if(acknowledged.has(record.id))bits[index>>3]|=1<<(index&7);});item.acknowledgedBits=btoa(String.fromCharCode(...bits));item.pendingChars=archive.ledger.records.reduce((sum,record)=>sum+(acknowledged.has(record.id)?0:record.chars),0);
  }
  const archivedIncoming=manifest.workload&&(next.archives||[]).find(item=>item.ledgerId===manifest.delivery.ledgerId);
  if(archivedIncoming){const archive=archives.get(archivedIncoming.id),known=new Map(archive.ledger.records.map(record=>[record.id,record]));requireValue(manifest.workload.records.every(record=>known.has(record.id)&&canonical(known.get(record.id))===canonical(record)),'归档交付包含未知或冲突记录');}
  if(manifest.workload&&!archivedIncoming){
   const incoming=manifest.workload;
   requireValue(next.id===manifest.delivery.ledgerId||(!next.records.length&&!next.deliveries.length),'不同台账不能合并到已有工作区，请分别保留交付');
   next.id=manifest.delivery.ledgerId;
   const existing=new Map(next.records.map(record=>[record.id,record]));
   for(const record of incoming.records){
    if(existing.has(record.id))requireValue(canonical(existing.get(record.id))===canonical(record),'相同工作 ID 内容冲突');
    else{next.records.push(copy(record));existing.set(record.id,record);}
   }
   const same=next.deliveries.find(delivery=>delivery.id===manifest.delivery.id);
   if(same)requireValue(canonical(same.recordIds)===canonical(incoming.records.map(record=>record.id))&&same.revision===manifest.delivery.revision&&same.previousId===manifest.delivery.previousId&&same.exportedAt===manifest.exportedAt&&same.fileRevision===manifest.fileVersion.revision&&canonical(same.workload)===canonical(summary(incoming))&&(!same.fingerprint||same.fingerprint===wireFingerprint),'相同交付 ID 内容冲突');
   else next.deliveries.push({...copy(manifest.delivery),fingerprint:wireFingerprint||'',exportedAt:manifest.exportedAt,fileRevision:manifest.fileVersion.revision,recordIds:incoming.records.map(record=>record.id),workload:summary(incoming)});
   next.acknowledgedIds=[...new Set([...next.acknowledgedIds,...incoming.acknowledgedRecordIds])];
   next.deliveries.sort((left,right)=>left.revision-right.revision);
  }
  const acknowledged=new Set(next.acknowledgedIds);
  for(const receipt of manifest.workReceipts||[]){
   const same=next.receipts.find(item=>item.id===receipt.id);
   if(same){requireValue(canonical(same)===canonical(receipt),'同 ID 回执内容冲突');continue;}
   if(receipt.ledgerId!==next.id||receipt.language!==tasks[0].language)continue;
   next.receipts.push(copy(receipt));
  }
  for(const receipt of next.receipts){
   if(receipt.ledgerId!==next.id||receipt.language!==tasks[0].language)continue;
   const delivery=next.deliveries.find(item=>item.id===receipt.deliveryId);
   if(!delivery)continue;
   const eligible=new Set(delivery.recordIds);
   requireValue(receipt.recordIds.every(id=>eligible.has(id)),'回执包含该交付未提交的记录');
   receipt.recordIds.forEach(id=>acknowledged.add(id));
  }
  next.acknowledgedIds=[...acknowledged];
  for(const task of tasks){
   const index=next.sources.findIndex(source=>source.partName===task.partName&&source.language===task.language);
   if(index>=0&&next.sources[index].taskVersion>=task.taskVersion){
    if(next.sources[index].taskVersion===task.taskVersion&&next.sources[index].packageId===null){
     next.sources[index].packageId=manifest.packageId;next.sources[index].revision=manifest.fileVersion.revision;
    }
    continue;
   }
   const source={partName:task.partName,language:task.language,packageId:manifest.packageId,revision:manifest.fileVersion.revision,taskVersion:task.taskVersion,startIndex:next.records.length};
   if(index<0)next.sources.push(source);else next.sources[index]=source;
  }
  validateLedger(next);return next;
 }
 async function makePacket(ledger,document){
  validateLedger(ledger);
  const selected=new Set(document.tasks.map(task=>task.partName)),language=document.tasks[0].language;
  const records=ledger.records.filter(record=>selected.has(record.partName)&&record.language===language);
  const previous=ledger.deliveries.at(-1),previousIds=new Set(previous?.recordIds||[]);
  const acknowledged=new Set(ledger.acknowledgedIds);
  const sourceStarts=new Map(ledger.sources.map(source=>[canonical([source.partName,source.language]),source.startIndex]));
  const since=new Set(ledger.records.filter((record,index)=>index>=(sourceStarts.get(canonical([record.partName,record.language]))??0)).map(record=>record.id));
  const subset=predicate=>records.filter(predicate).map(record=>record.id);
  const sum=predicate=>records.reduce((total,record)=>total+(predicate(record)?record.chars:0),0);
  const workload={
   version:1,countingRule:rule,
   scope:{projectId:document.projectId,lineageId:document.fileVersion.lineageId,language,partNames:[...selected].sort()},
   ...progress(document.tasks),cumulativeChars:sum(()=>true),handoverChars:sum(record=>!acknowledged.has(record.id)),
   previousDeliveryDeltaChars:sum(record=>!previousIds.has(record.id)),sinceSourceUpdateChars:sum(record=>since.has(record.id)),
   records:copy(records),acknowledgedRecordIds:subset(record=>acknowledged.has(record.id)),
   newRecordIds:subset(record=>!previousIds.has(record.id)),sourceUpdateRecordIds:subset(record=>since.has(record.id))
  };
  // Hash each task before import normalization, without retaining another task copy.
  const contentFingerprint=await fingerprint({...document,workload});
  let delivery=previous,isNew=false;
  if(!delivery||delivery.fingerprint!==contentFingerprint){
   requireValue(ledger.deliveries.length<10000,'交付历史已达上限，请先归档');
   const revision=Math.max(0,...ledger.deliveries.map(item=>item.revision))+1;
   delivery={id:localizationUuid(),ledgerId:ledger.id,revision,previousId:previous?.id||null,exportedAt:new Date().toISOString(),fileRevision:Math.max(document.fileVersion.revision,...ledger.deliveries.map(item=>item.fileRevision))+1,fingerprint:contentFingerprint,recordIds:records.map(record=>record.id),workload:summary(workload)};
   isNew=true;
  }
  const packet={...document,packageId:delivery.id,exportedAt:delivery.exportedAt,fileVersion:{...document.fileVersion,revision:delivery.fileRevision},
   delivery:{version:1,id:delivery.id,ledgerId:ledger.id,revision:delivery.revision,previousId:delivery.previousId},
   workload:{...copy(delivery.workload),records:copy(records)}};
  validatePackage(packet,packet.tasks);
  if(isNew)ledger.deliveries.push(delivery);
  return packet;
 }
 async function prepareArchive(ledger,document){
  validateLedger(ledger);const frozen=copy(ledger);delete frozen.archives;const fork=copy(frozen);fork.deliveries=fork.deliveries.slice(-1);const packets=[],generatedDeliveries=[];
  for(let start=0;start<document.tasks.length;start+=200){const packet=await makePacket(fork,{...document,tasks:document.tasks.slice(start,start+200)});packets.push(packet);const delivery=fork.deliveries.at(-1);if(!frozen.deliveries.some(item=>item.id===delivery.id)&&!generatedDeliveries.some(item=>item.id===delivery.id))generatedDeliveries.push(copy(delivery));fork.deliveries=fork.deliveries.slice(-1);}
  validateLedger({...frozen,deliveries:[...frozen.deliveries,...generatedDeliveries]},true);const acknowledged=new Set(frozen.acknowledgedIds);
  const id=localizationUuid(),archive={format:'mida-localization-ledger-archive',version:1,id,ledger:frozen,packets,generatedDeliveries};
  const item={id,ledgerId:frozen.id,archivedAt:new Date().toISOString(),chars:frozen.records.reduce((sum,record)=>sum+record.chars,0),recordCount:frozen.records.length,packetCount:packets.length,pendingChars:frozen.records.reduce((sum,record)=>sum+(acknowledged.has(record.id)?0:record.chars),0)};
  const next=create();next.archives=[...(ledger.archives||[]),item];alignSources(next,document.tasks,null,ledger.sources.map(source=>({...source,startIndex:0})));return {archive,item,next};
 }
 function registerArchive(archive){requireValue(archive?.format==='mida-localization-ledger-archive'&&archive.version===1&&text(archive.id),'归档格式无效');if(archive.generatedDeliveries){const ids=new Set(archive.ledger.deliveries.map(item=>item.id));for(const delivery of archive.generatedDeliveries)if(!ids.has(delivery.id)){archive.ledger.deliveries.push(delivery);ids.add(delivery.id);}delete archive.generatedDeliveries;}validateLedger(archive.ledger,true);archive.identities=new Set(archive.ledger.records.map(recordIdentity));archive.keys=new Set(archive.ledger.records.map(record=>canonical([record.partName,record.language,record.key])));archives.set(archive.id,archive);return archive;}
 return {prepareArchive,registerArchive,count,progress,create,alignSources,validateLedger,validatePackage,confirmed,mergeIncoming,makePacket,canonical,hashTasks,fingerprint};
})();
