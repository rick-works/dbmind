/**
 * 轮询失败保护。
 *
 * <p>进度轮询的失败有两种极端写法，都不对：
 * 一是「一次失败就收摊」—— 后端重启、链路瞬断都能把已经跑了几十分钟的同步判死；
 * 二是「失败就 console 一句接着试」—— 后端真的没了就每 500ms 无限打请求，界面永远转圈，
 * 用户看不出是断了还是在跑。
 *
 * <p>这里给中间口径：连续 N 次没有任何响应才算真断（默认 3 次；按 500~800ms 的间隔约
 * 1.5~2.4 秒，覆盖得住重启/抖动），到达上限后由调用方停轮询并明确提示。
 *
 * <p>用法：
 * ```
 * const pollFails = createPollGuard(3)
 * try {
 *   const st = await taskStatus(id)
 *   pollFails.ok()                      // 本轮成功：计数归零
 *   ...
 * } catch (e) {
 *   if (!pollFails.fail()) return       // 还在容忍范围内：下一轮接着试
 *   stopPolling()                       // 连续失败到上限：收摊 + 提示
 * }
 * ```
 */
export const createPollGuard = (limit = 3) => {
  let fails = 0
  return {
    /** 本轮成功（或换了一个新任务）：计数归零 */
    ok() { fails = 0 },
    reset() { fails = 0 },
    /** 本轮失败：返回是否已到上限（true = 调用方应当停轮询并提示） */
    fail() { fails += 1; return fails >= limit },
    /** 当前连续失败次数（提示文案里用得到） */
    get fails() { return fails },
    get limit() { return limit }
  }
}

export default createPollGuard
