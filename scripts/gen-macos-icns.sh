#!/usr/bin/env bash
# 从 assets/brand/memdock-icon-macos.svg 重新生成 src-tauri/icons/icon.icns。
# 只管 macOS：Dock、⌘Tab 切换器、访达都读这个文件。
# 托盘图标走 icons/32x32.png，Windows 走 icon.ico，都不在这里动。
#
# 需要 rsvg-convert（brew install librsvg）和 iconutil（系统自带）。
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$root/assets/brand/memdock-icon-macos.svg"
out="$root/src-tauri/icons/icon.icns"
set="$(mktemp -d)/MemDock.iconset"
mkdir -p "$set"

command -v rsvg-convert >/dev/null || { echo "缺 rsvg-convert：brew install librsvg" >&2; exit 1; }

# iconset 要求的 10 个representation：名字 => 实际像素
for pair in \
  16x16:16 16x16@2x:32 32x32:32 32x32@2x:64 \
  128x128:128 128x128@2x:256 256x256:256 256x256@2x:512 \
  512x512:512 512x512@2x:1024
do
  name="${pair%%:*}"; px="${pair##*:}"
  rsvg-convert -w "$px" -h "$px" "$src" -o "$set/icon_$name.png"
done

iconutil -c icns "$set" -o "$out"
rm -rf "$(dirname "$set")"
echo "已写入 $out"

# 图标是 tauri-codegen 在编译期嵌进二进制的，但 tauri-build 只对 capabilities/ 和
# tauri*.conf.json 发 rerun-if-changed —— 换了 icns 不会触发重编，OUT_DIR 里会一直是旧图。
# 碰一下被监听的文件，逼 build script 重跑（只改 mtime，内容不动，git 看不见）。
touch "$root/src-tauri/tauri.conf.json"
echo "已 touch tauri.conf.json 触发重编（否则下次构建仍用旧图标）"
