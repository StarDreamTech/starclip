#!/bin/zsh
cd -- "${0:A:h}" || exit 1
if ! command -v npm >/dev/null 2>&1; then
  echo "未找到 npm。请先安装 Node.js 22.12 或更新版本，然后重新打开。"
  read "?按回车关闭…"
  exit 1
fi
npm run app
if [[ $? -ne 0 ]]; then
  echo "预览未正常启动，请查看上方错误。首次启动需要 Rust 与 Xcode Command Line Tools。"
  read "?按回车关闭…"
fi
