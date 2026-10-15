'use strict';

globalThis.LocalizationExcel = (() => {
 const mimeType='application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';
 const maxBytes=64*1024*1024;
 const columns=[['片段',20],['对话包',32],['对话下标',14],['词条 key',42],['原文',64],['译文',64]];
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
 async function createWorkbook(tasks){
  await loadLibrary();
  requireValue(globalThis.ExcelJS?.Workbook,'组件未加载，请安装依赖后重新生成前端。');
  requireValue(tasks.length&&tasks.some(task=>task.entries.length),'请先勾选包含对话的任务。');
  const language=tasks[0].language;
  requireValue(tasks.every(task=>task.language===language),'请选择同一目标语言的任务。');
  const entryCount=tasks.reduce((count,task)=>count+task.entries.length,0);
  requireValue(entryCount+tasks.length+1<=1048576,'行数超过 Excel 上限，请减少勾选任务。');
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
  for(const task of tasks){
   const separator=sheet.addRow([`${task.partName} · ${task.language} · ${task.entries.length} 句`]);
   sheet.mergeCells(separator.number,1,separator.number,columns.length);
   separator.height=28;
   const title=separator.getCell(1);
   title.font={name:'Microsoft YaHei',size:11,bold:true,color:{argb:'FF244762'}};
   title.fill={type:'pattern',pattern:'solid',fgColor:{argb:'FFE3EDF5'}};
   title.border={top:{style:'medium',color:{argb:'FF648BA8'}},bottom:{style:'thin',color:{argb:'FF648BA8'}}};
   title.alignment={vertical:'middle',wrapText:true};
   for(const entry of task.entries){
    const suffix=entry.key.startsWith(entry.dialoguePackage+'_')?entry.key.slice(entry.dialoguePackage.length+1):null;
    const values=[task.partName,entry.dialoguePackage,suffix??entry.order,entry.key,entry.currentSource,entry.translation];
    for(const value of values)requireValue(typeof value!=='string'||value.length<=32767,`${task.partName} / ${entry.key} 的单元格超过 32767 字符上限。`);
    // Strings remain literal text, including keys with leading zeroes and dialogue beginning with '='.
    const row=sheet.addRow(values);row.height=rowHeight(values);
    row.eachCell({includeEmpty:true},cell=>{
     cell.font={name:'Microsoft YaHei',size:11,color:{argb:'FF202C36'}};
     cell.alignment={vertical:'top',wrapText:true};
     cell.border={bottom:{style:'hair',color:{argb:'FFDCE3E9'}}};
     if(typeof cell.value==='string')cell.numFmt='@';
    });
    if(++processed%500===0)await pause();
   }
  }
  const bytes=new Uint8Array(await workbook.xlsx.writeBuffer());
  requireValue(bytes.byteLength<=maxBytes,'文件超过 64 MiB，请减少勾选任务。');
  return {bytes,entryCount,taskCount:tasks.length};
 }
 function toBase64(bytes){
  const chunks=[];
  for(let index=0;index<bytes.length;index+=32768)chunks.push(String.fromCharCode(...bytes.subarray(index,index+32768)));
  return btoa(chunks.join(''));
 }
 return {mimeType,createWorkbook,toBase64};
})();
