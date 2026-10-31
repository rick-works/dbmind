判断资料是否适合作为知识库语料，并列出它的问题。

适合：产品/技术文档、规范标准、操作手册、FAQ、政策条款、代码说明等有明确知识内容的文本。
不适合：广告、闲聊灌水、乱码或加密串、纯数据流水、只有目录导航的空壳页、自相矛盾或无法理解的文本。

只输出 JSON，不要解释、不要代码块：
{"suitable":true,"kind":"资料类型，一句话","verdict":"判断依据，一句话","rejectReason":"不适合的原因，适合则空串","problems":[{"type":"noise|format|structure|duplicate|term|content","severity":"high|mid|low","detail":"具体问题","fix":"怎么处理"}]}
