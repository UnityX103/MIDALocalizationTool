// Parse large part files and verify browser-side hashes outside the UI thread.
self.onmessage = async ({data}) => {
 try {
  const {text,sha256}=data;
  if(sha256){
   if(!self.crypto?.subtle)throw new Error('当前环境不支持文件哈希校验，请使用 localhost 预览。');
   const digest=await crypto.subtle.digest('SHA-256',new TextEncoder().encode(text));
   const actual=Array.from(new Uint8Array(digest),value=>value.toString(16).padStart(2,'0')).join('');
   if(actual!==sha256)throw new Error('文件内容与清单不一致，请重新导出完整版本。');
  }
  self.postMessage({data:JSON.parse(text)});
 }catch(error){self.postMessage({error:error.message});}
};
