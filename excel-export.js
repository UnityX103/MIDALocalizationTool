'use strict';

globalThis.LocalizationExcel = (() => {
 const mimeType='application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';
 const maxBytes=64*1024*1024;
 const columns=[['状态',16],['片段',20],['对话包',32],['对话下标',14],['词条 key',42],['原文',64],['译文',64]];
 const historicalColor='FFE7E6E6';
 const requireValue=(condition,message)=>{if(!condition)throw new Error('Excel 导出：'+message);};
 const pause=()=>new Promise(resolve=>setTimeout(resolve,0));
 let libraryLoading;
 async function loadLibrary(){
  if(globalThis.ExcelJS?.Workbook)return;
  if(!libraryLoading)libraryLoading=new Promise((resolve,reject)=>{
   const script=document.createElement('script');script.src='vendor/exceljs.min.js';
   script.onload=resolve;script.onerror=()=>{libraryLoading=null;script.remove();reject(new Error('Excel 组件加载失败，请运行 npm ci 并重新生成前端。'));};
   document.head.append(script);
  });
  await libraryLoading;
 }
 function rowHeight(values){
  const lines=values.map((value,index)=>String(value).split(/\r\n|\r|\n/).reduce((count,line)=>{
   // CJK characters occupy roughly two Latin character widths in Excel.
   const width=[...line].reduce((size,character)=>size+(character.codePointAt(0)>255?2:1),0);
   return count+Math.max(1,Math.ceil(width/(columns[index][1]-2)));
  },0));
  return Math.min(409,Math.max(24,Math.max(...lines)*16+8));
 }
 async function collectRows(tasks,history){
  const groups=new Map();let entryCount=0,historicalCount=0,processed=0;
  const countRow=()=>{entryCount++;requireValue(entryCount+tasks.length+1<=1048576,'含历史文本的行数超过 Excel 上限，请减少勾选任务。');};
  const taskKey=task=>JSON.stringify([task.partName,task.language]);
  for(const task of tasks){
   const entries=new Map();
   for(const entry of task.entries){
    entries.set(entry.key,{current:entry,rows:[{entry,status:'使用中'}],seen:new Map([[entry.currentSource,new Set([entry.translation])]])});
    countRow();
   }
   groups.set(taskKey(task),{task,entries,historicalCount:0});
  }
  const add=(task,entry)=>{
   const group=groups.get(taskKey(task));if(!group)return;
   requireValue(typeof entry.key==='string'&&typeof entry.currentSource==='string'&&typeof entry.translation==='string','历史文本缺少原文、译文或词条 key。');
   let item=group.entries.get(entry.key);
   if(!item){item={current:null,rows:[],seen:new Map()};group.entries.set(entry.key,item);}
   let translations=item.seen.get(entry.currentSource);
   if(!translations){translations=new Set();item.seen.set(entry.currentSource,translations);}
   if(translations.has(entry.translation))return;
   translations.add(entry.translation);countRow();historicalCount++;group.historicalCount++;
   if(typeof entry.dialoguePackage!=='string'){
    const separator=entry.key.lastIndexOf('_');
    entry={...entry,dialoguePackage:separator>0?entry.key.slice(0,separator):entry.key};
   }
   item.rows.push({entry,status:item.current?'已被覆盖':'已弃用'});
  };
  const addTask=async task=>{
   if(!groups.has(taskKey(task)))return;
   for(const entry of task.entries){
    add(task,entry);
    // These two fields are a paired source/target snapshot, not a draft translation.
    if(entry.localizationSourceAtExport)add(task,{...entry,currentSource:entry.localizationSourceAtExport,translation:entry.translationAtExport});
    if(++processed%500===0)await pause();
   }
  };
  for(const task of tasks)await addTask(task);
  for await(const batch of history){
   for(const task of batch.tasks||[])await addTask(task);
   for(const record of batch.records||[]){
    if(!groups.has(taskKey(record)))continue;
    const separator=record.key.lastIndexOf('_');
    add(record,{key:record.key,dialoguePackage:separator>0?record.key.slice(0,separator):record.key,currentSource:record.sourceText,translation:record.translation});
    if(++processed%500===0)await pause();
   }
  }
  return {groups,entryCount,historicalCount};
 }
 async function createWorkbook(tasks,history=[]){
  await loadLibrary();
  requireValue(globalThis.ExcelJS?.Workbook,'组件未加载，请安装依赖后重新生成前端。');
  requireValue(tasks.length&&tasks.some(task=>task.entries.length),'请先勾选包含对话的任务。');
  const language=tasks[0].language;
  requireValue(tasks.every(task=>task.language===language),'请选择同一目标语言的任务。');
  const {groups,entryCount,historicalCount}=await collectRows(tasks,history);
  const workbook=new ExcelJS.Workbook();
  workbook.creator='MIDA 本地化编辑器';workbook.created=new Date();
  const sheet=workbook.addWorksheet('对话翻译',{views:[{state:'frozen',ySplit:1}]});
  sheet.columns=columns.map(([,width])=>({width}));
  const header=sheet.addRow(columns.map(([title])=>title==='译文'?`${title}（${language}）`:title));
  header.height=28;
  header.eachCell(cell=>{
   cell.font={name:'Microsoft YaHei',size:11,bold:true,color:{argb:'FFFFFFFF'}};
   cell.fill={type:'pattern',pattern:'solid',fgColor:{argb:'FF244762'}};
   cell.alignment={vertical:'middle',wrapText:true};
  });
  let processed=0;
  for(const {task,entries,historicalCount:oldCount} of groups.values()){
   // Keep the status cell blank so fragment titles do not become filter statuses.
   const separator=sheet.addRow([null,`${task.partName} · ${task.language} · 当前 ${task.entries.length} 句 / 旧版 ${oldCount} 条`]);
   sheet.mergeCells(separator.number,2,separator.number,columns.length);
   separator.height=28;
   for(const index of [1,2]){
    const title=separator.getCell(index);
    title.font={name:'Microsoft YaHei',size:11,bold:true,color:{argb:'FF244762'}};
    title.fill={type:'pattern',pattern:'solid',fgColor:{argb:'FFE3EDF5'}};
    title.border={top:{style:'medium',color:{argb:'FF648BA8'}},bottom:{style:'thin',color:{argb:'FF648BA8'}}};
    title.alignment={vertical:'middle',wrapText:true};
   }
   for(const item of entries.values())for(const {entry,status} of item.rows){
    const suffix=entry.key.startsWith(entry.dialoguePackage+'_')?entry.key.slice(entry.dialoguePackage.length+1):null;
    const values=[status,task.partName,entry.dialoguePackage,suffix??entry.order??'',entry.key,entry.currentSource,entry.translation];
    for(const value of values)requireValue(typeof value!=='string'||value.length<=32767,`${task.partName} / ${entry.key} 的单元格超过 32767 字符上限。`);
    // Strings remain literal text, including keys with leading zeroes and dialogue beginning with '='.
    const row=sheet.addRow(values);row.height=rowHeight(values);
    row.eachCell({includeEmpty:true},cell=>{
     cell.font={name:'Microsoft YaHei',size:11,color:{argb:'FF202C36'}};
     cell.alignment={vertical:'top',wrapText:true};
     cell.border={bottom:{style:'hair',color:{argb:'FFDCE3E9'}}};
     if(status!=='使用中')cell.fill={type:'pattern',pattern:'solid',fgColor:{argb:historicalColor}};
     if(typeof cell.value==='string')cell.numFmt='@';
    });
    if(++processed%500===0)await pause();
   }
  }
  sheet.autoFilter={from:{row:1,column:1},to:{row:sheet.rowCount,column:columns.length}};
  const bytes=new Uint8Array(await workbook.xlsx.writeBuffer());
  requireValue(bytes.byteLength<=maxBytes,'文件超过 64 MiB，请减少勾选任务。');
  return {bytes,entryCount,historicalCount,taskCount:tasks.length};
 }
 function toBase64(bytes){
  const chunks=[];
  for(let index=0;index<bytes.length;index+=32768)chunks.push(String.fromCharCode(...bytes.subarray(index,index+32768)));
  return btoa(chunks.join(''));
 }
 return {mimeType,createWorkbook,toBase64};
})();
