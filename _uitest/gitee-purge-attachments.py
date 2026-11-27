"""清理 Gitee 旧版本 Release 附件，腾出 1GB 附件配额。

Gitee 的附件配额是**仓库总量 1GB**，不是单文件上限；每次发版前若不清理，
新的安装包会全部报「文件大小已超出仓库附件配额：1 GB」（2026-11-11 实测）。

用法：
  python _uitest/gitee-purge-attachments.py            # 预览，不删
  python _uitest/gitee-purge-attachments.py --apply    # 真删（保留 KEEP 里的 tag）
"""
import json
import os
import sys
import urllib.request

API = "https://gitee.com/api/v5/repos/rick-works/dbmind"
# 保留最新版 + 上一版（够腾出配额即可，别把能下的历史版本都清了）
KEEP = {"v2.3.3", "v2.3.2"}
APPLY = "--apply" in sys.argv

tok = open(os.path.expanduser("~/.dbmind/gitee-token"), encoding="utf-8").read().strip()


def call(path, method="GET"):
    # Gitee 偶发连接被拒/超时（URLError），不能让整个清理中断 —— 重试 3 次，
    # 仍失败返回 None 交给调用方跳过（HTTPError 保留错误体给上传/删除判断）。
    last = None
    for attempt in range(3):
        req = urllib.request.Request(API + path, method=method,
                                     headers={"Authorization": "Bearer " + tok})
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                raw = r.read().decode("utf-8")
                return json.loads(raw) if raw.strip() else None
        except urllib.error.HTTPError as e:
            return {"__httpError": e.code, "body": e.read().decode("utf-8", "replace")[:200]}
        except urllib.error.URLError as e:
            last = e
            if attempt < 2:
                import time
                time.sleep(2 * (attempt + 1))
    print("      [warn] 请求失败 %s: %s" % (path, last))
    return None


# Gitee 的 /releases 是**升序**（v0.1.0 在前），单页 50 条只够翻到 v2.0.x，
# 新版根本扫不到 —— 会得出「可释放 0 MB」的假象（踩过）。循环翻页直到空页。
releases = []
page = 1
while True:
    chunk = call("/releases?per_page=100&page=%d" % page) or []
    if not isinstance(chunk, list) or not chunk:
        break
    releases.extend(chunk)
    page += 1
print("共 %d 个版本；保留：%s" % (len(releases), ",".join(sorted(KEEP))))
freed = 0
for rel in releases:
    tag = rel.get("tag_name") or ""
    if tag in KEEP:
        continue
    files = call("/releases/%s/attach_files" % rel["id"]) or []
    if not isinstance(files, list) or not files:
        continue
    for f in files:
        size = f.get("size") or 0
        print("  %-10s 删 %-42s %6.1f MB" % (tag, f["name"], size / 1048576))
        if APPLY:
            res = call("/releases/%s/attach_files/%s" % (rel["id"], f["id"]), "DELETE")
            if res and res.get("__httpError"):
                print("      失败：", res)
                continue
        freed += size

print("---")
print("%s 共释放 %.1f MB" % ("已删除" if APPLY else "预计释放", freed / 1048576))
if not APPLY:
    print("（预览模式，加 --apply 才真删）")
