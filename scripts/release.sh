#!/bin/sh
# 打包 wtools 并安装到 /Applications，然后重启
set -eu

cd "$(dirname "$0")/.."
APP_NAME="wtools.app"
BUILT="src-tauri/target/release/bundle/macos/$APP_NAME"
TARGET="/Applications/$APP_NAME"

# 有本机签名证书时用它签名：签名固定，重新打包后系统权限不会失效（证书由 pnpm setup-signing 创建）
IDENTITY="wtools Local Signing"
if security find-certificate -c "$IDENTITY" >/dev/null 2>&1; then
  export APPLE_SIGNING_IDENTITY="$IDENTITY"
  SIGNED=1
  echo "==> 使用证书「$IDENTITY」签名"
else
  SIGNED=0
  echo "==> 未找到签名证书，使用临时签名（每次打包后需重新授权；运行 pnpm setup-signing 可解决）"
fi

echo "==> 打包"
pnpm tauri build --bundles app

echo "==> 退出正在运行的 wtools"
pkill -x wtools 2>/dev/null || true
# 等进程完全退出，避免替换文件时仍被占用
for _ in 1 2 3 4 5 6 7 8 9 10; do
  pgrep -x wtools >/dev/null || break
  sleep 0.3
done

echo "==> 安装到 $TARGET"
rm -rf "$TARGET"
ditto "$BUILT" "$TARGET"

echo "==> 启动"
open "$TARGET"

if [ "$SIGNED" = 1 ]; then
  echo "完成。"
else
  echo "完成。注意：临时签名每次都会变，「屏幕录制」和「辅助功能」权限需要在系统设置里重新允许。"
fi
