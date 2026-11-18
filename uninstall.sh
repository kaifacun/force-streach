#!/bin/zsh
# Force Stretch 제거: 자동 실행 해제 + 앱 삭제 (설정 파일은 유지)
launchctl bootout "gui/$(id -u)/local.force-stretch" 2>/dev/null || true
pkill -f "Force Stretch.app/Contents/MacOS/force-stretch" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/local.force-stretch.plist"
rm -rf "/Applications/Force Stretch.app"
echo "제거 완료"
