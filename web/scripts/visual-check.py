"""移动端与桌面端布局视觉检查。"""
from playwright.sync_api import sync_playwright

DESKTOP = {"width": 1280, "height": 720}
MOBILE = {"width": 375, "height": 667}
BASE = "http://localhost:4173"

def has_horizontal_overflow(page):
    return page.evaluate("""
        () => {
            const doc = document.documentElement;
            return doc.scrollWidth > doc.clientWidth;
        }
    """)

def inject_chat_samples(page):
    """在 ChatView 的消息区域注入长文本与代码块样本，验证渲染。"""
    page.evaluate("""
        () => {
            const messages = document.querySelector('.messages');
            if (!messages) return false;
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
            `;
            return true;
        }
    """)

def screenshot(page, name, viewport):
    page.set_viewport_size(viewport)
    page.goto(f"{BASE}/login")
    page.wait_for_load_state("networkidle")
    page.screenshot(path=f"scripts/screenshots/{name}_login.png", full_page=True)

    # 通过 localStorage 写入登录态绕过后端，访问 /chat
    page.evaluate("""
        () => {
            localStorage.setItem('forgeclaw.token', 'demo-token');
            localStorage.setItem('forgeclaw.user', JSON.stringify({ id: '1', name: ' Tester' }));
        }
    """)
    page.goto(f"{BASE}/chat")
    page.wait_for_load_state("networkidle")
    inject_chat_samples(page)
    page.wait_for_timeout(300)
    page.screenshot(path=f"scripts/screenshots/{name}_chat.png", full_page=True)

    return {
        "login_overflow": has_horizontal_overflow(page),
        "chat_overflow": has_horizontal_overflow(page),
    }

def main():
    import os
    os.makedirs("scripts/screenshots", exist_ok=True)
    results = {}
    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        page = browser.new_page()
        results["desktop"] = screenshot(page, "desktop", DESKTOP)
        results["mobile"] = screenshot(page, "mobile", MOBILE)
        browser.close()
    print(results)
    if any(v for r in results.values() for v in r.values()):
        raise SystemExit("存在水平溢出")

if __name__ == "__main__":
    main()
