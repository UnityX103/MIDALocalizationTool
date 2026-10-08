'use strict';

// Practice data always lives outside production language spaces.
globalThis.MidaTutorial = (() => {
 const projectId='mida-editor-tutorial-v1',lineageId='mida-editor-tutorial-lineage-v1';
 const metaKey='mida-localization-editor-tutorial-guide-v1',guideVersion=2;
 const lessons=['基本编辑','查看视频','导出交付'];
 let hooks,active=false,busy=false,startupReady=false,offerPending=false,step=0,mediaUrl=null,editStarted=false,deliveryPracticed=false,manualNavigation=false,lastScrollTarget=null;
 const panel=document.createElement('aside');panel.id='mida-tutorial-panel';panel.hidden=true;panel.setAttribute('aria-label','使用教程');
 const focusLayer=document.createElement('div');focusLayer.id='mida-tutorial-focus';focusLayer.hidden=true;focusLayer.setAttribute('aria-hidden','true');
 focusLayer.innerHTML='<svg width="100%" height="100%"><defs><mask id="mida-tutorial-spotlight"><rect width="100%" height="100%" fill="white"/><rect class="focus-hole" rx="10" fill="black"/></mask></defs><rect width="100%" height="100%" fill="rgba(4,13,18,.62)" mask="url(#mida-tutorial-spotlight)"/></svg><div class="tutorial-focus-ring"></div>';
 function el(tag,text,className){const x=document.createElement(tag);if(text!==undefined)x.textContent=text;if(className)x.className=className;return x;}
 function meta(){try{return JSON.parse(localStorage.getItem(metaKey)||'null')||{};}catch{return {};}}
 function remember(status){try{localStorage.setItem(metaKey,JSON.stringify({version:guideVersion,status,step,delivered:deliveryPracticed,updatedAt:Date.now()}));}catch(error){hooks.notice('教程进度未能保存：'+error.message);}}
 function button(text,handler,primary=false){const b=el('button',text,primary?'primary':'');b.type='button';b.addEventListener('click',()=>{Promise.resolve().then(handler).catch(error=>hooks.notice('教程操作失败：'+error.message));});return b;}
 function visible(selector){return [...document.querySelectorAll(selector)].find(x=>!x.disabled&&x.getClientRects().length&&!x.closest('[hidden]'));}
 function namedButton(scope,name){return [...scope.querySelectorAll('button')].find(x=>x.textContent.trim()===name&&!x.disabled&&x.getClientRects().length);}
 function focusTarget(){
  const dialog=visible('#loc-overlay .dialog');if(dialog){
   if(document.querySelector('#loc-dialog-title').textContent==='仍有待处理问题，是否继续导出？')return [namedButton(dialog,'是'),'尚有待处理内容，点击“是”可导出草稿。'];
   return [visible('#loc-close'),'关闭弹窗，继续教程。'];
  }
  if(visible('#loc-history-return'))return [visible('#loc-history-return'),'返回当前版本，继续教程。'];
  if(step===0){const input=visible('#loc-text-0');if(input)return input.value.trim()?[visible('#loc-edit-0'),'点击“确定”，保存这句译文。']:[input,'输入 Welcome to the tutorial workspace.'];return [visible('#loc-edit-0')||visible('#loc-sentences'),'点击译文旁的“编辑”。'];}
  if(step===1){const targets=visible('.preview-targets');if(!targets)return [visible('.package-video'),'点击“查看视频”。'];const video=visible('video'),selected=targets.querySelector('[aria-pressed=true]');return video&&selected?.textContent.includes('教程镜头B')&&video.currentTime>=4?[video,'查看画面后，点击“下一步”。']:[namedButton(targets,'教程镜头B · 第 2 次 · 0:04.000'),'连续点两下镜头 B，直接打开并播放。'];}
  if(deliveryPracticed)return [visible('#loc-export'),'导出演练完成。正式交付时，把完整 ZIP 交给 Unity 回收，无需解压。'];
  const draft=visible('#loc-sentences textarea[aria-label$=" 译文"]');if(draft)return [namedButton(draft.closest('article'),'确定'),'先确定译文，再导出交付。'];
  if(!hooks.state().selected.length)return [visible('.taskfoot .checklabel')||visible('#loc-collapse'),'勾选要交付的任务。'];
  return [visible('#loc-export'),'点击右下角“导出”交付 ZIP。教程仅作演练。'];
 }
 function advance(){if(step>=lessons.length-1)return;step++;manualNavigation=false;remember('active');render();}
 function advanceIfComplete(){
  if(!active||busy||manualNavigation||step!==0||!editStarted)return;
  const current=hooks.state();if(!current.saving&&!current.editing&&current.tasks[0]?.entries[0].review.state==='confirmed')advance();
 }
 function setHighlights(){
  document.querySelectorAll('.tutorial-highlight').forEach(x=>x.classList.remove('tutorial-highlight'));if(!active){focusLayer.hidden=true;return;}
  advanceIfComplete();const [target,text]=focusTarget();panel.querySelector('.tutorial-instruction').textContent=text;if(!target){focusLayer.hidden=true;return;}let r=target.getBoundingClientRect();
  if(!r.width||!r.height){focusLayer.hidden=true;return;}if((r.top<8||r.bottom>innerHeight-8)&&lastScrollTarget!==target){lastScrollTarget=target;target.scrollIntoView({block:'center',inline:'nearest'});r=target.getBoundingClientRect();}
  const left=Math.max(4,r.left-7),top=Math.max(4,r.top-7),width=Math.max(1,Math.min(innerWidth-left-4,r.width+14)),height=Math.max(1,Math.min(innerHeight-top-4,r.height+14));
  target.classList.add('tutorial-highlight');focusLayer.hidden=false;
  for(const [key,value] of Object.entries({x:left,y:top,width,height}))focusLayer.querySelector('.focus-hole').setAttribute(key,value);
  Object.assign(focusLayer.querySelector('.tutorial-focus-ring').style,{left:left+'px',top:top+'px',width:width+'px',height:height+'px'});
  const p=panel.getBoundingClientRect(),gap=18;let x,y;
  if(innerWidth-r.right>=p.width+gap+12){x=r.right+gap;y=Math.max(12,Math.min(r.top,innerHeight-p.height-12));}
  else if(r.left>=p.width+gap+12){x=r.left-p.width-gap;y=Math.max(12,Math.min(r.top,innerHeight-p.height-12));}
  else{x=Math.max(12,Math.min(r.left,innerWidth-p.width-12));y=r.bottom+p.height+gap<innerHeight?r.bottom+gap:Math.max(12,r.top-p.height-gap);}
  panel.style.left=x+'px';panel.style.top=y+'px';
 }
 function render(){
  if(!active)return;panel.replaceChildren();panel.append(el('p','教程 · '+(step+1)+' / '+lessons.length,'tutorial-kicker'),el('h2',lessons[step]),el('p','','tutorial-instruction'));
  const controls=el('div',undefined,'tutorial-controls');if(step>0)controls.append(button('上一步',()=>{step--;editStarted=false;manualNavigation=true;remember('active');render();}));
  if(step<lessons.length-1)controls.append(button(step===0?'跳过':'下一步',advance,step===1));
  else if(deliveryPracticed)controls.append(button('完成教程并返回',()=>exit(true),true));
  controls.append(button('退出',()=>exit(false)));panel.append(controls);setHighlights();
 }
 function configure(value){hooks=value;document.body.append(focusLayer,panel);let scheduled=false;const refresh=event=>{if(!active)return;if(['click','input','change'].includes(event?.type)&&!panel.contains(event.target)){manualNavigation=false;if(step===0&&event.target?.id==='loc-edit-0'){editStarted=true;deliveryPracticed=false;}}if(scheduled)return;scheduled=true;requestAnimationFrame(()=>{scheduled=false;setHighlights();});};document.addEventListener('click',refresh);document.addEventListener('input',refresh);document.addEventListener('change',refresh);document.addEventListener('scroll',refresh,true);window.addEventListener('resize',refresh);}
 function startupFinished(){startupReady=true;const m=meta();offerPending=m.version!==guideVersion||!['completed','skipped'].includes(m.status);maybeOffer();}
 function maybeOffer(){if(!startupReady||!offerPending||active||busy||hooks.blocked())return;offerPending=false;welcome();}
 function welcome(){const content=hooks.dialog('使用教程');content.classList.add('tutorial-welcome');content.append(el('h3','三步学会基本操作'),el('p','编辑一句 → 查看视频 → 导出交付。'),el('p','全程静音，练习数据保存在独立教程空间。'));const actions=el('div',undefined,'dialogbottom'),m=meta();actions.append(button(m.version===guideVersion&&['active','paused'].includes(m.status)?'继续教程':'开始教程',()=>enter(),true),button('稍后',()=>{remember('skipped');hooks.close();}));content.append(actions);}
 async function enter(){
  if(active){render();return;}if(busy)throw new Error('请等待当前操作完成');hooks.close();if(hooks.blocked())throw new Error('请先完成当前操作或更新提示');busy=true;let entered=false;
  try{const m=meta(),fresh=m.version!==guideVersion||m.status==='completed';await hooks.enter();entered=true;if(fresh)await hooks.reset();await hooks.load();step=fresh?0:Math.max(0,Math.min(lessons.length-1,m.step||0));editStarted=false;deliveryPracticed=!fresh&&Boolean(m.delivered);manualNavigation=false;lastScrollTarget=null;active=true;document.body.classList.add('mida-tutorial-active');panel.hidden=false;remember('active');render();}
  catch(error){if(entered)await hooks.exit();throw error;}finally{busy=false;}
 }
 async function exit(completed=false){if(!active||busy)return;busy=true;try{await hooks.exit();hooks.close();remember(completed?'completed':'paused');active=false;document.body.classList.remove('mida-tutorial-active');panel.hidden=true;setHighlights();hooks.notice(completed?'教程已完成，已返回真实工作区。':'已退出教程，练习进度已保留。');}finally{busy=false;}}
 async function reset(){if(!active||busy)return;const c=hooks.dialog('重新练习教程');c.append(el('p','重新开始三步教程，仅重置教程练习数据。'));const actions=el('div',undefined,'dialogbottom');actions.append(button('取消',hooks.close),button('重新开始',async()=>{busy=true;try{await hooks.reset();await hooks.load();editStarted=false;deliveryPracticed=false;manualNavigation=false;lastScrollTarget=null;step=0;remember('active');hooks.close();render();}finally{busy=false;}},true));c.append(actions);}
 function openFeedback(){const c=hooks.dialog('问题反馈');c.append(el('p','请退出教程后，在真实工作区提交反馈。'));c.append(button('返回教程',hooks.close,true));}
 function delivery(){deliveryPracticed=true;remember('active');render();}
 async function hash(value){const bytes=await crypto.subtle.digest('SHA-256',new TextEncoder().encode(JSON.stringify(value)));return [...new Uint8Array(bytes)].map(x=>x.toString(16).padStart(2,'0')).join('');}
 async function snapshot(initialState){
 const rows=[['欢迎来到教程工作空间。','',false]];
 const entries=rows.map(([source,translation,confirmed],i)=>({key:'TutorialIntro_'+String(i+1).padStart(2,'0'),order:i,dialoguePackage:'TutorialIntro',speakerKey:'tutorial-guide',speakerChineseName:'教程向导',currentSource:source,localizationSourceAtExport:source,translationAtExport:translation,englishTranslationAtExport:translation,translation,issuesAtExport:confirmed?[]:['missing_translation'],review:{state:confirmed?'confirmed':'pending',reason:confirmed?'':'教程练习：请填写并确认译文'}}));
 const sourceHash=await hash(entries.map(e=>e.currentSource)),targetHash=await hash(entries.map(e=>e.translation)),packageHash=await hash(['TutorialIntro']);
 const fileVersion={lineageId,revision:1};const task={partName:'TutorialChapter',unitKind:'chapter',chapterName:'TutorialChapter',previewPartNames:['TutorialCameraA','TutorialCameraB'],language:'en',taskVersion:1,sourceHash,targetLocalizationHashAtExport:targetHash,versionStatusAtExport:'outdated',reviewAllRequired:false,entries};
 const manifest={format:'mida-localization-manifest',formatVersion:3,projectId,packageId:'mida-tutorial-source-v1',fileVersion,exportedAt:'2026-10-08T00:00:00Z',assets:[{id:'TutorialChapter',partName:'TutorialChapter',type:'localization-dialogues',path:'parts/TutorialChapter.json'}]};
 const previews={};for(const [partName,occurrence,timeMs] of [['TutorialCameraA',1,1000],['TutorialCameraB',2,4000]])previews[partName]={recordingId:'mida-tutorial-video-v1',mediaId:'tutorial-'+partName,map:{format:'mida-localization-preview',formatVersion:1,projectId,partName,chapterName:task.partName,recordingId:'mida-tutorial-video-v1',fps:24,packageNames:['TutorialIntro'],packageNamesHash:packageHash,events:[{packageName:'TutorialIntro',sourceUnit:task.partName,occurrence,frame:timeMs/1000*24,timeMs}]}};
 const ledger=LocalizationWorkload.create();LocalizationWorkload.alignSources(ledger,[task],manifest);
 return {format:'mida-localization-workspace',version:1,tasks:[task],drafts:[],importedTranslations:[],currentProjectId:projectId,currentManifest:manifest,currentPackageId:manifest.packageId,fileVersion,exportSequence:0,workLedger:ledger,previews,previewPlayer:{expanded:false,positions:{}},state:{...initialState,active:0,selected:[0]}};
 }
 function media(reference){if(!reference?.mediaId?.startsWith('tutorial-'))throw new Error('不是教程媒体');if(!mediaUrl){const bytes=Uint8Array.from(atob(MidaTutorialMediaBase64),x=>x.charCodeAt(0));mediaUrl=URL.createObjectURL(new Blob([bytes],{type:'video/mp4'}));}return {url:mediaUrl};}
 return {projectId,configure,startupFinished,maybeOffer,welcome,enter,exit,reset,openFeedback,delivery,snapshot,media,get active(){return active;},refreshHighlights:setHighlights};
})();
