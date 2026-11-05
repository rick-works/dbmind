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
gh_ release download "$TAG" --dir "$WORK/assets"
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

echo "==> 清理同名旧附件"
# 所有 curl 都带超时：CI 实测过上传大文件时连接挂死、job 悬 2h44m 无输出 ——
# 连接 20s 超时、单请求最多 30 分钟、速度低于 1KB/s 持续 90s 判定断流退出。
CURL_TO=(--connect-timeout 20 --max-time 1800 --speed-limit 1024 --speed-time 90)
OLD_IDS=$(curl -sf "${CURL_TO[@]}" "${GITEE_API}/releases/${RID}/attach_files?per_page=100" \
  -H "Authorization: Bearer ${GITEE_TOKEN}" \
  | jq -r '.[].id' || true)
for oid in $OLD_IDS; do
  curl -sf "${CURL_TO[@]}" -X DELETE "${GITEE_API}/releases/${RID}/attach_files/${oid}" \
    -H "Authorization: Bearer ${GITEE_TOKEN}" >/dev/null \
    && echo "    已删旧附件 id=${oid}" || echo "    删旧附件失败 id=${oid}（继续）"
done

echo "==> 上传附件"
FAILED=0
SKIPPED=0
GITEE_MAX=104857600   # Gitee 单附件上限 100MB（服务端明确返回超限拒绝，提前跳过不白传）
for f in "$WORK"/assets/*; do
  [ -e "$f" ] || continue
  fname=$(basename "$f")
  size=$(du -h "$f" | cut -f1)
  fsize=$(stat -c%s "$f" 2>/dev/null || wc -c < "$f")
  echo -n "    ${fname} (${size}) ... "
  if [ "${fsize:-0}" -gt "$GITEE_MAX" ]; then
    echo "跳过（超过 Gitee 单附件 100MB 上限，只能从 GitHub Release 下载）"
    SKIPPED=$((SKIPPED + 1))
    continue
  fi
  RESP=$(curl -s "${CURL_TO[@]}" -X POST "${GITEE_API}/releases/${RID}/attach_files" \
    -H "Authorization: Bearer ${GITEE_TOKEN}" \
    -F "file=@${f}" 2>&1) || RESP="curl failed"
  if echo "$RESP" | grep -q '"id"'; then
    echo "OK"
  else
    echo "失败"
    echo "      ${RESP}"
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
