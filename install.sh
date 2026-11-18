#!/bin/zsh
# Force Stretch 설치: /Applications 복사 + 로그인 시 자동 실행(LaunchAgent) 등록
set -euo pipefail
cd "$(dirname "$0")"
APP_SRC="target/Force Stretch.app"
APP_DST="/Applications/Force Stretch.app"
PL="$HOME/Library/LaunchAgents/local.force-stretch.plist"
UID_NUM=$(id -u)

[[ -d "$APP_SRC" ]] || ./bundle.sh

# 실행 중인 인스턴스/기존 등록 정리
launchctl bootout "gui/$UID_NUM/local.force-stretch" 2>/dev/null || true
pkill -f "Force Stretch.app/Contents/MacOS/force-stretch" 2>/dev/null || true
sleep 1

rm -rf "$APP_DST"
cp -R "$APP_SRC" "$APP_DST"

mkdir -p "$HOME/Library/LaunchAgents"
cat > "$PL" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>local.force-stretch</string>
    <key>ProgramArguments</key>
    <array><string>/Applications/Force Stretch.app/Contents/MacOS/force-stretch</string></array>
    <key>RunAtLoad</key><true/>
    <!-- 메뉴에서 정상 종료하면 그대로 두고, 비정상 종료(안전장치 등)면 다시 실행 -->
    <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
    <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
PLIST

launchctl bootstrap "gui/$UID_NUM" "$PL"
echo "설치 완료: $APP_DST (로그인 시 자동 실행 등록됨)"
