class LocalizationPreviewPlayer {
 constructor(panel,options){
  this.panel=panel;this.options=options;this.video=panel.querySelector('video');this.body=panel.querySelector('.preview-body');this.message=panel.querySelector('[data-preview-message]');this.title=panel.querySelector('[data-preview-title]');
  this.state={expanded:false,positions:{}};this.generation=0;this.identity=null;this.reference=null;this.desiredTime=null;this.pendingLoad=null;this.loading=false;this.available=false;this.lastSavedSecond=-1;this.locatedEvent=null;
  this.video.addEventListener('loadedmetadata',()=>this.applyTime());
  this.video.addEventListener('error',()=>{this.available=false;this.options.onAvailabilityChange?.();});
  this.video.addEventListener('timeupdate',()=>{if(this.reference&&Number.isFinite(this.video.currentTime)){this.state.positions[this.reference.mediaId]=this.video.currentTime;const second=Math.floor(this.video.currentTime);if(second!==this.lastSavedSecond){this.lastSavedSecond=second;this.options.onChange();}}});
  this.video.addEventListener('error',()=>{if(this.video.getAttribute('src')){this.available=false;this.setMessage('预览视频不可用：文件可能已清理或格式不受支持。请重新导入带视频的 ZIP。');}});
  this.renderExpansion();
 }
 setMessage(text){this.message.textContent=text;this.message.hidden=!text;}
 snapshot(){return structuredClone({...this.state,expanded:false});}
 restore(value){this.state={expanded:false,positions:{}};if(value?.positions&&typeof value.positions==='object')for(const [key,time] of Object.entries(value.positions).slice(-100)){if(Number.isFinite(time)&&time>=0)this.state.positions[key]=time;}this.reset();this.renderExpansion();}
 expand(expanded){this.state.expanded=expanded;if(!expanded){this.video.pause();this.locatedEvent=null;}this.renderExpansion();this.options.onChange();}
 renderExpansion(){this.panel.hidden=!this.state.expanded;this.body.hidden=!this.state.expanded;}
 canSeekPackage(packageName){return this.available&&this.reference===this.options.context().preview&&this.reference.map.events.some(event=>event.packageName===packageName);}
 reset(){this.generation++;this.video.pause();this.video.removeAttribute('src');this.video.load();this.identity=null;this.reference=null;this.desiredTime=null;this.pendingLoad=null;this.loading=false;this.available=false;this.locatedEvent=null;this.state.expanded=false;this.renderExpansion();this.setMessage('');this.options.onAvailabilityChange?.();}
 refresh(){
  const context=this.options.context();const reference=context.preview;const identity=JSON.stringify([context.projectId,context.task?.partName,context.task?.language,context.task?.sourceHash,reference?.recordingId,reference?.mediaId]);
  if(identity===this.identity&&(this.available||this.loading))return this.pendingLoad||Promise.resolve();
  this.reset();this.identity=identity;this.reference=reference||null;const generation=this.generation;
  this.title.textContent='片段预览 · '+(context.task?.partName||'未选择片段');this.video.hidden=true;
  if(!reference){this.setMessage('此片段未附预览视频；导入带视频的 ZIP 后可按对话包定位。');return Promise.resolve();}
  const names=new Set(context.task.entries.map(entry=>entry.dialoguePackage));const catalog=reference.map?.packageNames||[];this.stale=names.size!==catalog.length||catalog.some(name=>!names.has(name));
  this.setMessage('正在读取本机预览视频…');this.loading=true;
  this.pendingLoad=(async()=>{
   try{
    const media=await this.options.resolve(context.projectId,context.task.partName,reference);if(generation!==this.generation)return;
    if(!media||typeof media.url!=='string')throw new Error('视频资源不存在');
    this.available=true;this.video.hidden=false;this.video.src=media.url;
    this.desiredTime=this.state.positions[reference.mediaId]||0;
    this.setMessage(this.stale?'录制版本已落后，旧画面仅供参考，请在 Unity 更新录制。':'');this.applyTime();
   }catch(error){if(generation===this.generation){this.available=false;this.setMessage('此版本的视频已清理或不可用。恢复旧备份不会恢复已删除的视频，请重新导入预览视频。');}}
   finally{if(generation===this.generation){this.loading=false;this.options.onAvailabilityChange?.();}}
  })();return this.pendingLoad;
 }
 async seekPackage(packageName){
  const pending=this.refresh();const generation=this.generation;await pending;if(generation!==this.generation||!this.available)return;
  this.expand(true);
  const events=this.reference.map.events;const index=events.findIndex(event=>event.packageName===packageName);
  if(index<0){this.video.pause();this.setMessage('该对话包在视频中没有对话。');return;}
  const event=events[index];
  if(this.locatedEvent===event&&this.desiredTime===null&&this.video.readyState>=1&&!this.video.ended&&Math.abs(this.video.currentTime-event.timeMs/1000)<0.15){
   try{await this.video.play();}catch(error){if(generation===this.generation&&error.name!=='AbortError')this.setMessage('视频未能开始播放，请点击视频自带的播放按钮重试。');}
   return;
  }
  this.seekEvent(event);
 }
 seekEvent(event){this.video.pause();this.locatedEvent=event;this.desiredTime=event.timeMs/1000;this.setMessage(this.stale?'录制版本已落后，旧画面仅供参考，请在 Unity 更新录制。':'');this.applyTime();}
 applyTime(){if(!this.available||this.video.readyState<1||this.desiredTime===null)return;const duration=this.video.duration;if(!Number.isFinite(duration)||this.desiredTime>duration+.1){this.setMessage('映射时间超出视频长度，请重新录制此片段。');this.desiredTime=null;return;}this.video.currentTime=Math.min(duration,Math.max(0,this.desiredTime));this.desiredTime=null;}
}
