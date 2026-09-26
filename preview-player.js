class LocalizationPreviewPlayer {
 constructor(panel,options){
  this.panel=panel;this.options=options;this.video=panel.querySelector('video');this.body=panel.querySelector('.preview-body');this.message=panel.querySelector('[data-preview-message]');this.title=panel.querySelector('[data-preview-title]');
  this.targetSelect=document.createElement('select');this.targetSelect.setAttribute('aria-label','片段与录制出现位置');this.targetSelect.hidden=true;panel.querySelector('.preview-header').append(this.targetSelect);
  this.targetSelect.addEventListener('change',()=>{const target=this.targets[Number(this.targetSelect.value)];if(this.targetSelect.value!==''&&target)this.selectTarget(target);});
  this.state={expanded:false,positions:{}};this.generation=0;this.identity=null;this.reference=null;this.desiredTime=null;this.pendingLoad=null;this.loading=false;this.available=false;this.lastSavedSecond=-1;this.locatedEvent=null;this.sources=[];this.targets=[];
  this.video.addEventListener('loadedmetadata',()=>this.applyTime());
  this.video.addEventListener('error',()=>{this.available=false;for(const source of this.sources)if(source.reference===this.reference)delete source.media;this.options.onAvailabilityChange?.();});
  this.video.addEventListener('timeupdate',()=>{if(this.reference&&Number.isFinite(this.video.currentTime)){this.state.positions[this.reference.mediaId]=this.video.currentTime;const second=Math.floor(this.video.currentTime);if(second!==this.lastSavedSecond){this.lastSavedSecond=second;this.options.onChange();}}});
  this.video.addEventListener('error',()=>{if(this.video.getAttribute('src')){this.available=false;this.setMessage('预览视频不可用：文件可能已清理或格式不受支持。请重新导入带视频的 ZIP。');}});
  this.renderExpansion();
 }
 setMessage(text){this.message.textContent=text;this.message.hidden=!text;}
 snapshot(){return structuredClone({...this.state,expanded:false});}
 restore(value){this.state={expanded:false,positions:{}};if(value?.positions&&typeof value.positions==='object')for(const [key,time] of Object.entries(value.positions).slice(-100)){if(Number.isFinite(time)&&time>=0)this.state.positions[key]=time;}this.reset();this.renderExpansion();}
 expand(expanded){this.state.expanded=expanded;if(!expanded){this.video.pause();this.locatedEvent=null;}this.renderExpansion();this.options.onChange();}
 renderExpansion(){this.panel.hidden=!this.state.expanded;this.body.hidden=!this.state.expanded;}
 references(context){
  if(context.task?.unitKind!=='chapter')return context.preview?[{partName:context.task.partName,reference:context.preview}]:[];
  if(!context.task.partName?.trim())return [];
  return Object.entries(context.previews||{}).flatMap(([partName,reference])=>{
   const map=reference?.map;
   return map?.projectId===context.projectId&&map.chapterName===context.task.partName&&map.events.some(event=>event.sourceUnit===context.task.partName)
    ?[{partName,reference}]:[];
  });
 }
 packageTargets(packageName){
  const context=this.options.context();if(this.contextTask!==context.task)return [];
  return this.sources.flatMap(source=>source.media?(source.eventsByPackage.get(packageName)||[]).map(event=>({source,event})):[]);
 }
 canSeekPackage(packageName){return this.contextTask===this.options.context().task&&this.sources.some(source=>source.media&&source.eventsByPackage.has(packageName));}
 reset(){this.generation++;this.video.pause();this.video.removeAttribute('src');this.video.load();this.identity=null;this.reference=null;this.contextTask=null;this.sources=[];this.targets=[];this.targetSelect.replaceChildren();this.targetSelect.hidden=true;this.desiredTime=null;this.pendingLoad=null;this.loading=false;this.available=false;this.locatedEvent=null;this.state.expanded=false;this.renderExpansion();this.setMessage('');this.options.onAvailabilityChange?.();}
 refresh(){
  const context=this.options.context(),sources=this.references(context),chapter=context.task?.unitKind==='chapter';
  const identity=JSON.stringify([context.projectId,context.task?.partName,context.task?.language,context.task?.sourceHash,context.task?.unitKind,sources.map(source=>[source.partName,source.reference.recordingId,source.reference.mediaId])]);
  if(identity===this.identity&&this.contextTask===context.task&&(this.sources.some(source=>source.media)||this.loading))return this.pendingLoad||Promise.resolve();
  this.reset();this.identity=identity;this.contextTask=context.task;this.sources=sources;const generation=this.generation;
  for(const source of sources){
   source.eventsByPackage=new Map();
   for(const event of source.reference.map.events){
    if(chapter?event.sourceUnit!==context.task.partName:event.sourceUnit?.trim()&&event.sourceUnit!==context.task.partName)continue;
    if(!source.eventsByPackage.has(event.packageName))source.eventsByPackage.set(event.packageName,[]);
    source.eventsByPackage.get(event.packageName).push(event);
   }
  }
  this.title.textContent=(chapter?'章节预览 · ':'片段预览 · ')+(context.task?.partName||'未选择片段');this.video.hidden=true;
  if(!sources.length){this.setMessage(chapter?'此章节没有明确来源的录制事件。':'此片段未附预览视频；导入带视频的 ZIP 后可按对话包定位。');return Promise.resolve();}
  this.setMessage('正在读取本机预览视频…');this.loading=true;
  this.pendingLoad=(async()=>{
   let next=0;
   const resolveNext=async()=>{
    while(next<sources.length&&generation===this.generation){
     const source=sources[next++];
     try{const media=await this.options.resolve(context.projectId,source.partName,source.reference);if(generation!==this.generation)return;if(media&&typeof media.url==='string')source.media=media;}catch{}
    }
   };
   await Promise.all(Array.from({length:Math.min(4,sources.length)},resolveNext));
   if(generation!==this.generation)return;
   this.loading=false;
   if(!sources.some(source=>source.media))this.setMessage('此版本的视频已清理或不可用。恢复旧备份不会恢复已删除的视频，请重新导入预览视频。');
   else if(!chapter)this.loadSource(sources[0]);
   else this.setMessage('');
   this.options.onAvailabilityChange?.();
  })();return this.pendingLoad;
 }
 loadSource(source){
  const task=this.options.context().task,reference=source.reference;if(!source.media||this.contextTask!==task||!this.sources.includes(source))return false;
  this.available=true;this.video.hidden=false;
  const names=new Set(task.entries.map(entry=>entry.dialoguePackage)),catalog=new Set(reference.map.packageNames||[]);
  this.stale=[...names].some(name=>!catalog.has(name))||(!reference.map.chapterName&&task.unitKind!=='chapter'&&names.size!==catalog.size);
  if(this.reference!==reference){
   this.video.pause();this.reference=reference;this.available=true;this.video.hidden=false;this.locatedEvent=null;this.video.src=source.media.url;this.lastSavedSecond=-1;
   this.desiredTime=this.state.positions[reference.mediaId]||0;
  }
  this.title.textContent='片段预览 · '+source.partName+(task.unitKind==='chapter'?' · 章节 '+task.partName:'');
  this.setMessage(this.stale?'录制版本已落后，旧画面仅供参考，请在 Unity 更新录制。':'');this.applyTime();return true;
 }
 async seekPackage(packageName){
  const pending=this.refresh();const generation=this.generation;await pending;if(generation!==this.generation)return;
  const targets=this.packageTargets(packageName);if(!targets.length)return;
  this.expand(true);
  this.targets=targets;this.targetSelect.replaceChildren();this.targetSelect.hidden=true;
  if(this.options.context().task.unitKind==='chapter'&&targets.length>1){
   this.video.pause();this.locatedEvent=null;this.desiredTime=null;this.video.hidden=true;
   this.title.textContent='章节预览 · '+this.options.context().task.partName+' · '+packageName;
   const placeholder=document.createElement('option');placeholder.value='';placeholder.textContent='选择录制位置';placeholder.disabled=true;placeholder.selected=true;this.targetSelect.append(placeholder);
   targets.forEach((target,index)=>{
    const option=document.createElement('option');option.value=String(index);
    const time=target.event.timeMs,minutes=Math.floor(time/60000),seconds=Math.floor(time/1000)%60,milliseconds=time%1000;
    option.textContent=target.source.partName+' · 第 '+target.event.occurrence+' 次 · '+minutes+':'+String(seconds).padStart(2,'0')+'.'+String(milliseconds).padStart(3,'0');
    this.targetSelect.append(option);
   });
   this.targetSelect.hidden=false;this.setMessage('');this.targetSelect.focus();return;
  }
  const {source,event}=targets[0];this.loadSource(source);
  if(this.locatedEvent===event&&this.desiredTime===null&&this.video.readyState>=1&&!this.video.ended&&Math.abs(this.video.currentTime-event.timeMs/1000)<0.15){
   try{await this.video.play();}catch(error){if(generation===this.generation&&error.name!=='AbortError')this.setMessage('视频未能开始播放，请点击视频自带的播放按钮重试。');}
   return;
  }
  this.seekEvent(event);
 }
 selectTarget({source,event}){if(this.loadSource(source))this.seekEvent(event);else this.setMessage('所选录像已不可用，请重新导入预览视频。');}
 seekEvent(event){this.video.pause();this.locatedEvent=event;this.desiredTime=event.timeMs/1000;this.setMessage(this.stale?'录制版本已落后，旧画面仅供参考，请在 Unity 更新录制。':'');this.applyTime();}
 applyTime(){if(!this.available||this.video.readyState<1||this.desiredTime===null)return;const duration=this.video.duration;if(!Number.isFinite(duration)||this.desiredTime>duration+.1){this.setMessage('映射时间超出视频长度，请重新录制此片段。');this.desiredTime=null;return;}this.video.currentTime=Math.min(duration,Math.max(0,this.desiredTime));this.desiredTime=null;}
}
