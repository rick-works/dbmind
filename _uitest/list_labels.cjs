let zh = require('d:/develop/workspace/product/DBMind/frontend/src/locales/zh-CN.js');
if (zh.default) zh = zh.default;
console.log('keys total =', Object.keys(zh).length);
Object.keys(zh).filter(k => k.startsWith('upv.')).sort().forEach(k => {
  const v = zh[k];
  if (typeof v === 'string' && /[\u4e00-\u9fa5]/.test(v)) console.log(k + ' = ' + v);
});

