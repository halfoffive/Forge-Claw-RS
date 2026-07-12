/** 移动端与桌面端布局视觉检查。 */
import { chromium } from '@playwright/test'
import { mkdir } from 'node:fs/promises'

const DESKTOP = { width: 1280, height: 720 }
const MOBILE = { width: 375, height: 667 }
const BASE = 'http://localhost:4173'
const EDGE = 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe'

async function hasHorizontalOverflow(page) {
  return page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth)
}

async function injectChatSamples(page) {
  await page.evaluate(() => {
    const messages = document.querySelector('.messages')
    if (!messages) return false
    messages.innerHTML = `
      <div class="msg user">
        <div class="bubble"><p class="text-segment">请解释这段代码并给出很长的说明，确保换行和可读性正常。</p></div>
      </div>
      <div class="msg assistant">
        <div class="bubble">
          <p class="text-segment">这是第一行。\n这是第二行。\n\n长文本长文本长文本长文本长文本长文本长文本长文本长文本长文本长文本长文本长文本长文本长文本长文本长文本长文本长文本。</p>
          <div class="code-wrap">
            <div class="code-lang">rust</div>
            <pre class="code-block"><code>fn main() {\n    let message = "Hello, ForgeClaw!";\n    println!("{}", message);\n}</code></pre>
          </div>
          <p class="text-segment">代码已展示完毕。</p>
        </div>
      </div>
    `
    return true
  })
}

async function screenshot(page, name, viewport) {
  await page.setViewportSize(viewport)
  await page.evaluate(() => localStorage.clear())

  await page.goto(`${BASE}/login`)
  await page.waitForLoadState('networkidle')
  await page.screenshot({ path: `scripts/screenshots/${name}_login.png`, fullPage: true })

  await page.evaluate(() => {
    localStorage.setItem('forgeclaw.token', 'demo-token')
    localStorage.setItem('forgeclaw.user', JSON.stringify({ id: '1', name: 'Tester' }))
  })

  await page.goto(`${BASE}/chat`)
  await page.waitForLoadState('networkidle')
  await injectChatSamples(page)
  await page.waitForTimeout(300)
  await page.screenshot({ path: `scripts/screenshots/${name}_chat.png`, fullPage: true })

  return {
    login_overflow: await hasHorizontalOverflow(page),
    chat_overflow: await hasHorizontalOverflow(page),
  }
}

async function main() {
  await mkdir('scripts/screenshots', { recursive: true })

  const browser = await chromium.launch({ executablePath: EDGE, headless: true })
  const page = await browser.newPage()
  const results = {
    desktop: await screenshot(page, 'desktop', DESKTOP),
    mobile: await screenshot(page, 'mobile', MOBILE),
  }
  await browser.close()

  console.log(JSON.stringify(results, null, 2))
  const overflow = Object.values(results).some((r) => r.login_overflow || r.chat_overflow)
  if (overflow) {
    console.error('存在水平溢出')
    process.exit(1)
  }
}

main().catch((e) => {
  console.error(e)
  process.exit(1)
})
