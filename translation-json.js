'use strict';

globalThis.LocalizationTranslationJson = (() => {
 const format='mida-localization-translations',maxBytes=32*1024*1024,maxEntries=100000;
 const taskFields=['unitKind','chapterName','sourceKind','assetProtocolVersion','moduleId','tableId','tableGuid'];
 const entryFields=['assetGuid','allowEmpty'];
 const requireValue=(condition,message)=>{if(!condition)throw new Error('译文 JSON：'+message);};
 const text=value=>typeof value==='string';
 const nonempty=value=>text(value)&&Boolean(value.trim());
 const context=(value,fields)=>Object.fromEntries(fields.filter(field=>Object.hasOwn(value,field)).map(field=>[field,value[field]]));
 const taskKey=task=>JSON.stringify([task.partName,task.language]);
 const key=(task,entry)=>JSON.stringify([task.partName,task.language,entry.key]);
 const same=(left,right)=>LocalizationWorkload.canonical(left)===LocalizationWorkload.canonical(right);
 const validContext=value=>value&&typeof value==='object'&&!Array.isArray(value)&&Object.values(value).every(item=>item===null||['string','boolean','number'].includes(typeof item));
 function aiDescription(language){
  return `这是 MIDA 本地化编辑器的翻译交换 JSON。请把每条 currentSource 中文原文翻译成目标语言 ${language}，将结果填写到对应的 translation 字段中。\n只填写或修改 translation，保留 description、所有身份、版本、来源、context 字段及 tasks / entries 结构，不增加、删除或重排词条。speakerKey、speakerChineseName 和 context 供理解发言人与语境，不要翻译这些字段。\ntranslation: null 表示尚未填写，请改为译文字符串，例如 "translation": "Translated text"；已有字符串可校对。无法翻译时保留 null。只有词条 context.allowEmpty 为 true 时才允许空字符串。保留原文中的占位符、标签及换行含义，并正确转义 JSON 字符串。\n返回完整、可解析的纯 JSON，不要使用 Markdown 代码围栏，也不要在 JSON 前后添加解释文字。`;
 }
 function createDocument(projectId,fileVersion,tasks,includeTranslated=false,description=aiDescription(tasks[0]?.language||'')){
  const selected=tasks.map(task=>({
   partName:task.partName,language:task.language,taskVersion:task.taskVersion,context:context(task,taskFields),
   entries:task.entries.filter(entry=>includeTranslated||entry.review.state!=='confirmed').map(entry=>({
    key:entry.key,currentSource:entry.currentSource,speakerKey:entry.speakerKey,speakerChineseName:entry.speakerChineseName,
    context:context(entry,entryFields),translation:entry.review.state==='confirmed'?entry.translation:null
   }))
  })).filter(task=>task.entries.length);
  requireValue(selected.length,'当前导出范围没有词条。');
  return {description,format,formatVersion:1,projectId,lineageId:fileVersion.lineageId,language:selected[0].language,
   exportedAt:new Date().toISOString(),tasks:selected};
 }
 function validateRow(row){
  requireValue(row&&nonempty(row.key)&&['currentSource','speakerKey','speakerChineseName'].every(field=>text(row[field])),'词条身份或中文原文无效。');
  requireValue(validContext(row.context),'词条来源信息无效。');
  requireValue(row.translation===null||text(row.translation),'translation 必须是译文字符串或 null。');
 }
 function validateDocument(document){
  requireValue(document?.format===format&&document.formatVersion===1,'请选择本工具导出的原文 JSON，并仅填写 translation 字段。');
  requireValue(['projectId','lineageId','language'].every(field=>nonempty(document[field])),'项目、版本谱系或目标语言缺失。');
  requireValue(Array.isArray(document.tasks)&&document.tasks.length>0&&document.tasks.length<=1000,'任务列表为空或超出上限。');
  const identities=new Set();let count=0;
  for(const task of document.tasks){
   requireValue(task&&nonempty(task.partName)&&task.language===document.language&&Number.isSafeInteger(task.taskVersion)&&task.taskVersion>0&&validContext(task.context),'任务身份、来源或版本无效。');
   const identity=taskKey(task);requireValue(!identities.has(identity),'任务重复。');identities.add(identity);
   requireValue(Array.isArray(task.entries)&&task.entries.length>0,'词条列表为空。');
   count+=task.entries.length;requireValue(count<=maxEntries,'词条数量超出上限。');
   const keys=new Set();
   for(const entry of task.entries){validateRow(entry);requireValue(!keys.has(entry.key),'同一任务内存在重复 key。');keys.add(entry.key);}
  }
 }
 function parse(input){
  requireValue(text(input)&&new TextEncoder().encode(input).byteLength<=maxBytes,'文件超过 32 MiB 上限。');
  let document;try{document=JSON.parse(input.replace(/^\uFEFF/,''));}catch{throw new Error('译文 JSON：无法解析文件，请检查 JSON 语法。');}
  validateDocument(document);return document;
 }
 function matches(candidate,task,entry){
  return candidate.partName===task.partName&&candidate.language===task.language&&candidate.key===entry.key&&
   candidate.taskVersion<=task.taskVersion&&same(candidate.taskContext,context(task,taskFields))&&
   candidate.currentSource===entry.currentSource&&candidate.speakerKey===entry.speakerKey&&
   candidate.speakerChineseName===entry.speakerChineseName&&same(candidate.context,context(entry,entryFields));
 }
 function available(candidate,task,entry){
  return Boolean(candidate)&&matches(candidate,task,entry)&&
   LocalizationWorkspaceStore.acceptsTranslation(task,entry,candidate.translation);
 }
 function planImport(document,projectId,fileVersion,tasks,fileName){
  validateDocument(document);
  requireValue(document.projectId===projectId&&document.lineageId===fileVersion?.lineageId,'项目或版本谱系不匹配，未导入。');
  requireValue(tasks.length&&tasks.every(task=>task.language===document.language),'目标语言不匹配，请先切换到对应语言空间。');
  const byTask=new Map(tasks.map(task=>[taskKey(task),{task,entries:new Map(task.entries.map(entry=>[entry.key,entry]))}]));
  const candidates=[],stats={empty:0,stale:0};
  const importedAt=new Date().toISOString();
  for(const incoming of document.tasks){
   const current=byTask.get(taskKey(incoming));
   for(const row of incoming.entries){
    if(row.translation===null){stats.empty++;continue;}
    const entry=current?.entries.get(row.key);
    const candidate={key:row.key,currentSource:row.currentSource,speakerKey:row.speakerKey,speakerChineseName:row.speakerChineseName,
     context:row.context,translation:row.translation,partName:incoming.partName,language:incoming.language,taskVersion:incoming.taskVersion,
     taskContext:incoming.context,fileName:fileName.slice(0,512),importedAt};
    if(!entry||!matches(candidate,current.task,entry)){stats.stale++;continue;}
    if(!LocalizationWorkspaceStore.acceptsTranslation(current.task,entry,row.translation)){stats.empty++;continue;}
    candidates.push(candidate);
   }
  }
  return {candidates,stats};
 }
 function validateCandidates(candidates,tasks){
  requireValue(Array.isArray(candidates)&&candidates.length<=maxEntries,'保存的候选译文数量无效。');
  const byTask=new Map(tasks.map(task=>[taskKey(task),{task,entries:new Map(task.entries.map(entry=>[entry.key,entry]))}]));
  const identities=new Set();
  for(const candidate of candidates){
   validateRow(candidate);
   requireValue(text(candidate.translation)&&validContext(candidate.taskContext)&&Number.isSafeInteger(candidate.taskVersion)&&candidate.taskVersion>0&&
    text(candidate.fileName)&&nonempty(candidate.importedAt),'保存的候选译文无效。');
   requireValue(candidate.editedTranslation===undefined||text(candidate.editedTranslation),'待校验译文的编辑内容无效。');
   const identity=key(candidate,candidate),current=byTask.get(taskKey(candidate)),entry=current?.entries.get(candidate.key);
   requireValue(!identities.has(identity)&&entry&&available(candidate,current.task,entry),'保存的候选译文与任务不匹配。');identities.add(identity);
  }
 }
 function prune(candidates,tasks){
  const byTask=new Map(tasks.map(task=>[taskKey(task),{task,entries:new Map(task.entries.map(entry=>[entry.key,entry]))}]));
  return candidates.filter(candidate=>{const current=byTask.get(taskKey(candidate)),entry=current?.entries.get(candidate.key);return entry&&available(candidate,current.task,entry);});
 }
 return {maxBytes,key,aiDescription,createDocument,parse,planImport,available,validateCandidates,prune};
})();
