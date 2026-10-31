// Generate a scoped color layer from the maintained base stylesheet.
// Original installations keep their default style; cola16 explicitly opts in.
const fs=require('fs'),postcss=require('postcss');
process.chdir(require('path').resolve(__dirname,'../..'));
// One editable manifest supplies both the immediate fallback and public consumers.
fs.mkdirSync('web/public/branding',{recursive:true});
fs.copyFileSync('web/src/cola16-navigation.json','web/public/branding/navigation.json');
// Shared header contract v1: one CSS source styles the shadow root (embedded in the
// script) and, scoped to the undefined element, the light-DOM fallback before/without
// script. Consumers may vendor an older fallback copy; :not(:defined) keeps it from
// overriding the shadow styles of a newer script.
const headerCSS=fs.readFileSync('web/src/cola-header.css','utf8');
const fallbackCSS=postcss.parse(headerCSS),host='cola-header:not(:defined)';
fallbackCSS.walkRules(r=>{r.selectors=r.selectors.map(s=>s.startsWith(':host')?s.replace(/^:host(?:\((.*)\))?/,(_,inner='')=>host+inner):s.startsWith('::slotted(')?host+'>'+s.slice(10,-1):host+' '+s)});
let headerJS=fs.readFileSync('web/src/cola-header.js','utf8');
for(const [token,value] of [["/*@css*/''",JSON.stringify(headerCSS)],['/*@navigation*/{ version: 1, items: [] }',JSON.stringify(JSON.parse(fs.readFileSync('web/src/cola16-navigation.json','utf8')))]]){
 if(!headerJS.includes(token))throw new Error('web/src/cola-header.js lost its build token '+token);
 headerJS=headerJS.replace(token,()=>value);
}
fs.mkdirSync('web/public/branding/v1',{recursive:true});
fs.writeFileSync('web/public/branding/v1/cola-header.js',headerJS);
fs.writeFileSync('web/public/branding/v1/cola-header.css',fallbackCSS.toString());
const base=postcss.parse(fs.readFileSync('web/src/style.css','utf8'));
function color(hex,prop){
 let s=hex.slice(1);if(s.length===3||s.length===4)s=s.split('').map(x=>x+x).join('');
 const [r,g,b]=[0,2,4].map(i=>parseInt(s.slice(i,i+2),16)),a=s.length===8?parseInt(s.slice(6),16)/255:1;
 let role;
 if(a<.95){role=prop.includes('shadow')?'divider':prop.includes('background')?'raised':'divider';}
 else if(prop.includes('background'))role=(r+g+b)/3<28?'bg':(r+g+b)/3<44?'surface':'raised';
 else if(prop.includes('border'))role='border';
 else if(r>g*1.3&&r>b*1.15)role=g>120?'accent':'error';
 else if(g>r*1.2&&g>b*.9)role='success';
 else role=(r+g+b)/3>198?'text':(r+g+b)/3>90?'muted':'text';
 return `var(--cola-${role})`;
}
base.walkAtRules('import',r=>r.remove());
base.walkRules(r=>{
 if(r.parent.type==='atrule'&&/keyframes$/.test(r.parent.name))return;
 r.selectors=r.selectors.map(s=>s.includes(':root')?s.replace(/:root/g,'html[data-brand="cola16"]'):'html[data-brand="cola16"] '+s);
});
base.walkDecls(d=>{
 const alias={'--red':'error','--green':'success','--accent':'accent','--surface':'surface','--border':'divider','--muted':'muted'}[d.prop];
 const selector=d.parent.selector || '';
 let semantic=/\.(?:error-box|danger|down|down-text|defaults-error)(?:[ .:[>]|$)/.test(selector)?'error':/\.(?:warning|partial)(?:[ .:[>]|$)/.test(selector)?'warning':/\.(?:healthy|success)(?:[ .:[>]|$)/.test(selector)?'success':null;
 if(alias)d.value=`var(--cola-${alias})`;
 else d.value=d.value.replace(/#[a-fA-F0-9]{3,8}\b/g,h=>{
  if(/\.(?:brand-mark|button\.primary)(?:[ .:[>]|$)/.test(selector))return `var(--cola-${d.prop==='color'?'on-accent':'accent'})`;
  if(semantic)return `var(--cola-${d.prop.includes('background')&&!/\.(down|partial|healthy)$/.test(selector)?'surface':semantic})`;
  return color(h,d.prop);
 });
 if(d.prop==='font-size'&&/^\d+(\.\d+)?px$/.test(d.value)&&parseFloat(d.value)<14)d.value='14px';
 if(d.prop.includes('border-radius'))d.value='4px';
 if(d.prop==='box-shadow')d.value='none';
});
const roles=JSON.parse(fs.readFileSync('web/src/plum.tokens.json','utf8')).themes;
const tokens=Object.entries(roles).map(([mode,colors])=>`html[data-brand="cola16"][data-cola-theme="${mode}"]{${Object.entries(colors).map(([k,v])=>`--cola-${k}:${v}`).join(';')};color-scheme:${mode}}`).join('\n');
const layout=postcss.parse(fs.readFileSync('web/src/plum-layout.css','utf8'));
layout.walkRules(r=>{r.selectors=r.selectors.map(s=>s===':root'?'html[data-brand="cola16"]':'html[data-brand="cola16"] '+s)});
fs.writeFileSync('web/src/plum.css',tokens+'\n'+base.toString()+'\n'+layout.toString());
