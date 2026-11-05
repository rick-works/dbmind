#!/usr/bin/env bash
# GitHub Release → Gitee Release 镜像同步
#
# 用法：
#   CI：   GITEE_TOKEN=... GH_TOKEN=... bash scripts/release/gitee-release.sh <tag>
#   本地： GITEE_TOKEN=... GH_TOKEN=... bash scripts/release/gitee-release.sh v2.0.6
#
# 行为（可重复执行，幂等）：
#   1. 用 gh 下载该 Release 的全部资产
#   2. 把 tag 推到 Gitee（已存在则强制覆盖为 GitHub 上的指向）
#   3. 在 Gitee 创建同名 Release（已存在则更新标题/正文）
#   4. 逐个上传附件（先删除同名旧附件避免重跑产生副本）
#
# 环境变量：
#   GITEE_TOKEN  Gitee 私人令牌（必填，scopes 需含 projects）
#   GH_TOKEN     GitHub 令牌（读 Release 用；CI 里自动有）
#   GITEE_REPO   默认 rick-works/dbmind
#   GITHUB_REPO  默认 rick-works/dbmind（或 CI 注入的 GITHUB_REPOSITORY）
set -euo pipefail

TAG="${1:-}"
[ -n "$TAG" ] || { echo "错误：缺少 tag 参数（例：v2.0.6）"; exit 1; }
: "${GITEE_TOKEN:?错误：缺少 GITEE_TOKEN 环境变量（Gitee 私人令牌）}"
: "${GH_TOKEN:?错误：缺少 GH_TOKEN 环境变量（GitHub 令牌）}"

GITHUB_REPO="${GITHUB_REPOSITORY:-rick-works/dbmind}"
GITEE_REPO="${GITEE_REPO:-rick-works/dbmind}"
GITEE_API="https://gitee.com/api/v5/repos/${GITEE_REPO}"
gh_() { gh --repo "$GITHUB_REPO" "$@"; }
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "==> 读取 GitHub Release ${TAG}"
gh_ release view "$TAG" --json name,body,isPrerelease > "$WORK/release.json" \
  || { echo "错误：GitHub 上找不到 Release ${TAG}"; exit 1; }
NAME=$(jq -r '.name // empty' "$WORK/release.json")
[ -n "$NAME" ] || NAME="$TAG"
BODY=$(jq -r '.body // ""' "$WORK/release.json")
PRERELEASE=$(jq -r '.isPrerelease // false' "$WORK/release.json")

echo "==> 下载资产到 $WORK/assets"
mkdir -p "$WORK/assets"
# 不走 `gh release download`：它拼的是 github.com 上的 browser_download_url，而 github.com
# 一旦被墙/抽风就很坑 —— 2026-11-06 实测 api.github.com 200、objects.githubusercontent.com 通，
# 但 github.com 及其所有 IP 全部超时，结果 0 字节死等半小时也不报错。
# 资产接口 api.github.com/repos/<repo>/releases/assets/<id> 会 302 到
# objects.githubusercontent.com，正好绕开 github.com；只需 GH_TOKEN，与浏览器地址无关。
API_TSV="$WORK/assets.tsv"
# 注意：endpoint 不能带前导斜杠 —— Git Bash（MSYS）会把它当 POSIX 绝对路径改写成
# D:/app/git/repos/...，gh 报 invalid API endpoint
if gh api "repos/${GITHUB_REPO}/releases/tags/${TAG}" \
     --jq '.assets[] | [.id,.name,(.size|tostring)] | @tsv' > "$API_TSV" 2>"$WORK/assets.err" \
   && [ -s "$API_TSV" ]; then
  export DL_BASE="https://api.github.com/repos/${GITHUB_REPO}/releases/assets"
  export DL_DIR="$WORK/assets" DL_ERR="$WORK/dl.err"
  : > "$DL_ERR"
  cat > "$WORK/dl.sh" <<'EOS'
#!/usr/bin/env bash
set -u
id="$1"; name="$2"
curl -sSL --connect-timeout 20 --max-time 1800 --speed-limit 1024 --speed-time 120 \
  -H "Authorization: Bearer ${GH_TOKEN}" -H "Accept: application/octet-stream" \
  -o "${DL_DIR}/${name}" "${DL_BASE}/${id}" \
  || echo "${name}" >> "${DL_ERR}"
EOS
  chmod +x "$WORK/dl.sh"
  # 3 路并发：单连接实测只有 ~170KB/s，390MB 串行要半小时以上
  awk -F'\t' '{print $1" "$2}' "$API_TSV" | xargs -P 3 -n 2 "$WORK/dl.sh"
  if [ -s "$DL_ERR" ]; then
    echo "错误：以下资产下载失败"
    sed 's/^/      /' "$DL_ERR"
    exit 1
  fi
  # 大小逐个校验：宁可这里失败，也不要把半截文件传上去 —— Gitee 附件按「同名同大小」判增量，
  # 一旦传上残缺文件，后续重跑会一直跳过它
  while IFS=$'\t' read -r _aid aname asize; do
    [ -n "${asize:-}" ] || continue
    got=$(stat -c%s "$DL_DIR/$aname" 2>/dev/null || echo 0)
    if [ "$got" != "$asize" ]; then
      echo "错误：${aname} 大小不符（期望 ${asize}，实际 ${got}）"
      exit 1
    fi
  done < "$API_TSV"
else
  echo "    （资产列表读不到，回退 gh release download）"
  sed 's/^/      /' "$WORK/assets.err" 2>/dev/null || true
  gh_ release download "$TAG" --dir "$WORK/assets"
fi
ls -l "$WORK/assets" || true

echo "==> 推送 tag ${TAG} 到 Gitee"
git rev-parse -q --verify "refs/tags/${TAG}^{commit}" >/dev/null 2>&1 \
  || git fetch origin "refs/tags/${TAG}:refs/tags/${TAG}" --no-tags
git push "https://oauth2:${GITEE_TOKEN}@gitee.com/${GITEE_REPO}.git" \
  "+refs/tags/${TAG}:refs/tags/${TAG}"

echo "==> 查找/创建 Gitee Release"
EXIST=$(curl -sf -H "Authorization: Bearer ${GITEE_TOKEN}" \
  "${GITEE_API}/releases/tags/${TAG}" || true)
RID=$(jq -r '.id // empty' <<<"$EXIST")
# 注意：Gitee 的 WAF 会 400 掉「-d 行内 JSON + 中文」的请求，必须 --data-binary @文件
if [ -n "$RID" ]; then
  echo "    Gitee Release 已存在（id=${RID}），更新标题/正文"
  jq -n --arg tag "$TAG" --arg name "$NAME" --arg body "$BODY" \
    '{tag_name:$tag, name:$name, body:$body}' > "$WORK/patch.json"
  curl -sf -X PATCH "${GITEE_API}/releases/${RID}" \
    -H "Authorization: Bearer ${GITEE_TOKEN}" \
    -H "Content-Type: application/json" \
    --data-binary @"$WORK/patch.json" >/dev/null
else
  SHA=$(git rev-parse "refs/tags/${TAG}^{commit}")
  jq -n --arg tag "$TAG" --arg name "$NAME" --arg body "$BODY" \
    --arg sha "$SHA" --argjson pre "$PRERELEASE" \
    '{tag_name:$tag, name:$name, body:$body, prerelease:$pre, target_commitish:$sha}' > "$WORK/create.json"
  RID=$(curl -sf -X POST "${GITEE_API}/releases" \
    -H "Authorization: Bearer ${GITEE_TOKEN}" \
    -H "Content-Type: application/json" \
    --data-binary @"$WORK/create.json" \
    | jq -r '.id')
  echo "    已创建 Gitee Release（id=${RID}）"
fi
[ -n "$RID" ] || { echo "错误：拿不到 Gitee Release id"; exit 1; }

echo "==> 清理旧附件（只删与本次资产对不上的）"
# 所有 curl 都带超时：CI 实测过上传大文件时连接挂死、job 悬 2h44m 无输出 ——
# 连接 20s 超时、单请求最多 30 分钟、速度低于 1KB/s 持续 90s 判定断流退出。
CURL_TO=(--connect-timeout 20 --max-time 1800 --speed-limit 1024 --speed-time 90)

# 本次 Release 的资产清单：名字 -> 字节数（用于「同名同大小就保留」的增量判断）
declare -A WANT=()
for f in "$WORK"/assets/*; do
  [ -e "$f" ] || continue
  WANT["$(basename "$f")"]=$(stat -c%s "$f" 2>/dev/null || wc -c < "$f")
done

EXIST=$(curl -sf "${CURL_TO[@]}" "${GITEE_API}/releases/${RID}/attach_files?per_page=100" \
  -H "Authorization: Bearer ${GITEE_TOKEN}" \
  | jq -r '.[] | [.id, .name, (.size // 0)] | @tsv' || true)
while IFS=$'\t' read -r aid aname asize; do
  [ -n "${aid:-}" ] || continue
  if [ "${WANT[$aname]:-x}" = "$asize" ]; then
    echo "    保留 ${aname}（同名同大小，不重传）"
  else
    curl -sf "${CURL_TO[@]}" -X DELETE "${GITEE_API}/releases/${RID}/attach_files/${aid}" \
      -H "Authorization: Bearer ${GITEE_TOKEN}" >/dev/null \
      && echo "    已删旧附件 id=${aid} ${aname}" || echo "    删旧附件失败 id=${aid}（继续）"
  fi
done <<<"$EXIST"

echo "==> 上传附件（3 路并发：CI 到 Gitee 单连接仅 ~430KB/s，串行 330MB 要 13 分钟）"
FAILED=0
SKIPPED=0
GITEE_MAX=104857600   # Gitee 单附件上限 100MB（服务端明确返回超限拒绝，提前跳过不白传）
PAR=3
RES="$WORK/results"
mkdir -p "$RES"
i=0
for f in "$WORK"/assets/*; do
  [ -e "$f" ] || continue
  fname=$(basename "$f")
  size=$(du -h "$f" | cut -f1)
  fsize="${WANT[$fname]:-0}"
  if [ "${fsize:-0}" -gt "$GITEE_MAX" ]; then
    echo "    ${fname} (${size}) 跳过（超过 Gitee 单附件 100MB 上限，只能从 GitHub Release 下载）"
    SKIPPED=$((SKIPPED + 1))
    continue
  fi
  if grep -Fq "$(printf '%s\t%s' "$fname" "$fsize")" <<<"$EXIST"; then
    echo "    ${fname} (${size}) 已存在（同名同大小，跳过）"
    continue
  fi
  echo "    ${fname} (${size}) 上传中 ..."
  (
    RESP=$(curl -s "${CURL_TO[@]}" -X POST "${GITEE_API}/releases/${RID}/attach_files" \
      -H "Authorization: Bearer ${GITEE_TOKEN}" \
      -F "file=@${f}" 2>&1) || RESP="curl failed"
    if grep -q '"id"' <<<"$RESP"; then
      echo "OK" > "$RES/$fname"
    else
      { echo "失败"; echo "      ${RESP}"; } > "$RES/$fname"
    fi
  ) &
  i=$((i + 1))
  if [ $((i % PAR)) -eq 0 ]; then wait; fi
done
wait
for r in "$RES"/*; do
  [ -e "$r" ] || continue
  rname=$(basename "$r")
  first=$(head -n1 "$r")
  if [ "$first" = "OK" ]; then
    echo "    ${rname} ... OK"
  else
    echo "    ${rname} ... $first"
    tail -n +2 "$r" | sed 's/^/      /'
    FAILED=$((FAILED + 1))
  fi
done

if [ "$SKIPPED" -gt 0 ]; then
  echo "==> 提示：${SKIPPED} 个附件因超过 100MB 未上传到 Gitee"
fi
if [ "$FAILED" -gt 0 ]; then
  echo "==> 完成，但有 ${FAILED} 个附件上传失败（多为超过 Gitee 单附件大小限制）"
  exit 2
fi
echo "==> 完成：Gitee Release ${TAG} 与附件已同步"
