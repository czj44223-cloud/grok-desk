---
name: computer-use
description: 操作这台电脑的屏幕。用户要求点击、输入、切换窗口，或看着屏幕完成一步操作时使用。
user-invocable: true
---

# 电脑操作

用插件里的工具看屏幕并操作。桌面端每次调用仍会先询问，不要假设已经允许。

1. 先调用 `computer_screenshot`。它会给出图片路径、屏幕原点 `originX` / `originY`，以及 `scaleX` / `scaleY`。用读取工具看那张图片，不要把整张图贴回对话。
2. 图片上的点要换成屏幕坐标再点击：`screenX = originX + imageX * scaleX`，Y 同样计算。
3. 然后只做一小步：`computer_click`、`computer_type` 或 `computer_key`。做完再截一张屏，确认结果。
4. `computer_windows` 只列出当前窗口标题，用来确认焦点在哪。
5. 不要输入密码、验证码或密钥，除非用户在这一条消息里明确写了要输入的内容。
6. 不要连续截屏空转。屏幕没有按预期变化时，停下来说明你看到了什么。
