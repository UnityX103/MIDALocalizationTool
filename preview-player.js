class LocalizationPreviewPlayer {
 constructor(panel,options){
  this.panel=panel;this.options=options;this.video=panel.querySelector('video');this.body=panel.querySelector('.preview-body');this.message=panel.querySelector('[data-preview-message]');this.timeline=panel.querySelector('[data-preview-timeline]');this.clock=panel.querySelector('[data-preview-clock]');this.nodes=panel.querySelector('[data-preview-nodes]');this.title=panel.querySelector('[data-preview-title]');this.toggle=panel.querySelector('[data-preview-toggle]');
  this.state={expanded:false,positions:{}};this.generation=0;this.identity=null;this.reference=null;this.desiredTime=null;this.pendingLoad=null;this.loading=false;this.available=false;this.lastSavedSecond=-1;
  this.toggle.addEventListener('click',()=>this.expand(!this.state.expanded));
  this.video.addEventListener('loadedmetadata',()=>this.applyTime());
  this.video.addEventListener('error',()=>{this.available=false;this.options.onAvailabilityChange?.();});
  this.video.addEventListener('timeupdate',()=>{this.timeline.value=this.video.currentTime;this.clock.textContent=this.format(this.video.currentTime)+' / '+this.format(this.video.duration);if(this.reference&&Number.isFinite(this.video.currentTime)){this.state.positions[this.reference.mediaId]=this.video.currentTime;const second=Math.floor(this.video.currentTime);if(second!==this.lastSavedSecond){this.lastSavedSecond=second;this.options.onChange();}}});
  this.video.addEventListener('error',()=>{if(this.video.getAttribute('src')){this.available=false;this.message.textContent='预览视频不可用：文件可能已清理或格式不受支持。请重新导入带视频的 ZIP。';this.timeline.disabled=true;}});
  this.timeline.addEventListener('input',()=>{this.video.pause();this.desiredTime=Number(this.timeline.value);this.applyTime();});
  this.nodes.addEventListener('change',()=>{const event=this.reference?.map?.events?.[Number(this.nodes.value)];if(event)this.seekEvent(event);});
  this.renderExpansion();
 }
 format(seconds){if(!Number.isFinite(seconds))return '00:00.0';return String(Math.floor(seconds/60)).padStart(2,'0')+':'+(seconds%60).toFixed(1).padStart(4,'0');}
 snapshot(){return structuredClone(this.state);}
 restore(value){this.state={expanded:value?.expanded===true,positions:{}};if(value?.positions&&typeof value.positions==='object')for(const [key,time] of Object.entries(value.positions).slice(-100)){if(Number.isFinite(time)&&time>=0)this.state.positions[key]=time;}this.reset();this.renderExpansion();}
 expand(expanded){this.state.expanded=expanded;if(!expanded)this.video.pause();this.renderExpansion();this.options.onChange();}
 renderExpansion(){this.body.hidden=!this.state.expanded;this.toggle.textContent=this.state.expanded?'收起视频':'展开视频';this.toggle.setAttribute('aria-expanded',String(this.state.expanded));}
 canSeekPackage(packageName){return this.available&&this.reference===this.options.context().preview&&this.reference.map.events.some(event=>event.packageName===packageName);}
 reset(){this.generation++;this.video.pause();this.video.removeAttribute('src');this.video.load();this.identity=null;this.reference=null;this.desiredTime=null;this.pendingLoad=null;this.loading=false;this.available=false;this.timeline.disabled=true;this.nodes.replaceChildren();this.clock.textContent='00:00.0 / 00:00.0';this.options.onAvailabilityChange?.();}
 refresh(){
  const context=this.options.context();const reference=context.preview;const identity=JSON.stringify([context.projectId,context.task?.partName,context.task?.language,context.task?.sourceHash,reference?.recordingId,reference?.mediaId]);
  if(identity===this.identity&&(this.available||this.loading))return this.pendingLoad||Promise.resolve();
  this.reset();this.identity=identity;this.reference=reference||null;const generation=this.generation;
  this.title.textContent='片段预览 · '+(context.task?.partName||'未选择片段');this.video.hidden=true;this.nodes.disabled=true;
  if(!reference){this.message.textContent='此片段未附预览视频；导入带视频的 ZIP 后可按对话包定位。';return Promise.resolve();}
  const names=new Set(context.task.entries.map(entry=>entry.dialoguePackage));const catalog=reference.map?.packageNames||[];this.stale=names.size!==catalog.length||catalog.some(name=>!names.has(name));
  this.message.textContent='正在读取本机预览视频…';this.loading=true;
  this.pendingLoad=(async()=>{
   try{
    const media=await this.options.resolve(context.projectId,context.task.partName,reference);if(generation!==this.generation)return;
    if(!media||typeof media.url!=='string')throw new Error('视频资源不存在');
    this.available=true;this.video.hidden=false;this.video.src=media.url;this.timeline.disabled=false;
    this.timeline.max=Math.max(0,(reference.map.durationMs||0)/1000);this.desiredTime=this.state.positions[reference.mediaId]||0;
    this.nodes.replaceChildren();const placeholder=document.createElement('option');placeholder.value='';placeholder.textContent='选择对话时间节点';this.nodes.append(placeholder);
    for(const [index,event] of reference.map.events.entries()){const option=document.createElement('option');option.value=String(index);option.textContent=this.format(event.timeMs/1000)+' · '+event.packageName+' · 第 '+event.occurrence+' 次';this.nodes.append(option);}
    this.nodes.disabled=!reference.map.events.length;this.message.textContent=this.stale?'录制版本已落后，旧画面仅供参考，请在 Unity 更新录制。':'点击对话包旁的“查看视频”定位；视频与映射只读。';this.applyTime();
   }catch(error){if(generation===this.generation){this.available=false;this.message.textContent='此版本的视频已清理或不可用。恢复旧备份不会恢复已删除的视频，请重新导入预览视频。';}}
   finally{if(generation===this.generation){this.loading=false;this.options.onAvailabilityChange?.();}}
  })();return this.pendingLoad;
 }
 async seekPackage(packageName){
  this.expand(true);await this.refresh();if(!this.available)return;
  const events=this.reference.map.events;const index=events.findIndex(event=>event.packageName===packageName);
  if(index<0){this.video.pause();this.message.textContent='该对话包在视频中没有对话。';return;}
  this.nodes.value=String(index);this.seekEvent(events[index]);
 }
 seekEvent(event){this.video.pause();this.desiredTime=event.timeMs/1000;this.applyTime();this.message.textContent=(this.stale?'旧版画面仅供参考 · ':'')+event.packageName+' · 第 '+event.occurrence+' 次出现 · '+this.format(event.timeMs/1000);}
 applyTime(){if(!this.available||this.video.readyState<1||this.desiredTime===null)return;const duration=this.video.duration;if(!Number.isFinite(duration)||this.desiredTime>duration+.1){this.message.textContent='映射时间超出视频长度，请重新录制此片段。';this.desiredTime=null;return;}this.video.currentTime=Math.min(duration,Math.max(0,this.desiredTime));this.timeline.value=this.video.currentTime;this.timeline.max=duration;this.clock.textContent=this.format(this.video.currentTime)+' / '+this.format(duration);this.desiredTime=null;}
}
