/* cleanErrText / connErrorRaw 的直测（node 直接跑，不依赖构建） */
import { cleanErrText } from './src/utils/errMsg.js'

const cases = [
  [
    "连接失败：Login failed for user 'not_exist_user'。ClientConnectionId:30ddcd6e-a29b-4977-bcb7-cb720fa6f153",
    "连接失败：Login failed for user 'not_exist_user'。",
  ],
  [
    "用户 'demo' 登录失败。ClientConnectionId:30ddcd6e-a29b-4977-bcb7-cb720fa6f153",
    "用户 'demo' 登录失败。",
  ],
  [
    "失败 ClientConnectionId=30ddcd6e-a29b-4977-bcb7-cb720fa6f153",
    '失败',
  ],
  [
    "执行失败（会话 11111111-2222-3333-4444-555555555555 已断开）",
    "执行失败（会话 已断开）",
  ],
  ["Incorrect syntax near 'selct'.", "Incorrect syntax near 'selct'."],
  ["SQLServerException, SQLState=S0001, errorCode=18456\nLogin failed for user 'x'", "Login failed for user 'x'"],
]

let bad = 0
for (const [input, want] of cases) {
  const got = cleanErrText(input)
  const ok = got.trim() === want
  if (!ok) bad++
  console.log((ok ? 'PASS' : 'FAIL'), JSON.stringify(got), ok ? '' : '期望 ' + JSON.stringify(want))
}
console.log(bad ? `${bad} 个失败` : '全部通过')
process.exit(bad ? 1 : 0)
