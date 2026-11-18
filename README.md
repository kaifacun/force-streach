# Force Stretch

하루 종일 컴퓨터 앞에 앉아 있는 사람을 위한 **강제 스트레칭 알리미** (macOS, Rust).

정해진 주기마다 연결된 **모든 모니터를 전체 화면으로 가려서** 작업을 할 수 없게 만들고, 스트레칭 시간이 끝나야 화면을 돌려줍니다. 알림은 무시할 수 있지만, 화면이 가려지면 일어날 수밖에 없습니다.

![overlay](docs/overlay.png)

## 기능

- **메뉴바 상주 앱** — Dock 아이콘 없이 상단 메뉴바에 다음 스트레칭까지 남은 시간(`🧘 23:41`)을 표시
- **강제 화면 가림**
  - 연결된 모든 모니터, 전체화면 앱, 다른 데스크탑(Spaces) 위까지 덮음
  - Dock·메뉴바 숨김, `Cmd+Tab` 앱 전환 차단, `Cmd+Q` 등 단축키 무시
  - 큰 카운트다운과 스트레칭 동작 안내(20초마다 교체)
  - 스트레칭 도중 모니터를 연결/해제해도 다시 덮음
- **사전 경고** — 시작 30초 전부터 메뉴바 아이콘이 `⚠️`로 바뀜
- **똑똑한 주기 계산**
  - 5분 이상(또는 스트레칭 시간 이상) 입력이 없으면 이미 쉰 것으로 보고 주기를 초기화
  - 잠자기에서 깨어나면 주기를 초기화
- **설정** — 메뉴에서 주기·시간을 바로 고르거나, 설정 창에서 직접 입력
- **안전장치**
  - 비상 해제 문구: 가림 중에 정해 둔 문구를 그대로 타이핑하면 해제 (한글 입력 상태에서도 동작, 비워두면 비활성)
  - 가림 화면이 예정 종료 시각보다 30초 이상 남아 있으면 프로세스가 스스로 종료 (UI와 독립된 감시 스레드)
  - 강제 종료(`Cmd+Opt+Esc`)는 막지 않음

## 요구 사항

- macOS 12 이상 (Apple Silicon / Intel)
- [Rust](https://rustup.rs) (stable)

## 설치

```sh
git clone <this-repo-url> force-stretch
cd force-stretch
./install.sh
```

`install.sh`는 다음을 수행합니다.

1. 릴리스 빌드 후 `Force Stretch.app` 번들 생성 (`bundle.sh`)
2. `/Applications/Force Stretch.app`으로 복사
3. 로그인 시 자동 실행되도록 LaunchAgent(`~/Library/LaunchAgents/local.force-stretch.plist`) 등록 후 즉시 실행

메뉴에서 **종료**하면 다음 로그인 때까지 꺼진 상태로 유지되고, 비정상 종료 시에는 자동으로 다시 실행됩니다.

앱 번들만 만들고 싶다면:

```sh
./bundle.sh   # → target/Force Stretch.app
```

### 제거

```sh
./uninstall.sh
```

자동 실행 해제와 앱 삭제를 수행합니다. 설정 파일은 남겨 둡니다.

## 사용법

메뉴바의 `🧘` 아이콘을 클릭하면 다음 메뉴가 열립니다.

| 메뉴 | 설명 |
| --- | --- |
| 다음 스트레칭까지 MM:SS | 현재 상태 표시 |
| 지금 스트레칭 시작 | 즉시 화면 가림 시작 |
| 1시간 일시정지 / 일시정지 해제 | 회의·발표 중에 사용 |
| 스트레칭 주기 | 20 / 30 / 45 / 50 / 60 / 90분마다 |
| 스트레칭 시간 | 1 / 2 / 3 / 5 / 10분 |
| 설정… | 주기·시간 직접 입력, 경고 시간, 자리 비움 초기화, 비상 해제 문구 |
| 설정 파일 열기… | `config.toml`을 기본 편집기로 열기 |
| 종료 | 앱 종료 |

### 시험 실행

5초 뒤 약 8초간 화면을 가리고, 끝나면 자동 종료됩니다. 설정 파일은 바꾸지 않습니다.

```sh
open -W "target/Force Stretch.app" --args --test
```

> 이미 실행 중인 인스턴스가 있다면 먼저 메뉴에서 종료하세요.

## 설정 파일

`~/Library/Application Support/force-stretch/config.toml`

```toml
interval_secs = 3000      # 작업 주기 (초). 기본 50분
break_secs = 180          # 스트레칭(화면 가림) 시간 (초). 기본 3분
warn_secs = 30            # 시작 전 메뉴바 경고 시간 (초)
idle_reset = true         # 자리 비움 시 주기 초기화 여부
idle_reset_secs = 300     # 자리 비움으로 판단할 무입력 시간 (초)
emergency_phrase = "i promise i will stretch later"  # 비상 해제 문구. ""이면 비활성
```

설정 파일을 직접 고친 경우 앱을 다시 실행해야 반영됩니다. 메뉴나 설정 창에서 바꾼 값은 즉시 반영되고 자동 저장됩니다.

허용 범위: 주기 1분~24시간, 스트레칭 시간 10초~30분, 자리 비움 판단 2분~1시간.

## 구조

```
src/
├── main.rs      # 앱 델리게이트, 상태 머신(작업/일시정지/스트레칭), 메뉴바, 타이머, 안전장치
├── overlay.rs   # 모니터별 전체 화면 가림 창, 키 입력(비상 해제) 처리, 스트레칭 안내 문구
├── settings.rs  # 설정 창 UI
└── config.rs    # 설정 구조체 및 TOML 입출력
```

- AppKit 바인딩: [`objc2`](https://github.com/madsmtm/objc2) / `objc2-app-kit` / `objc2-foundation`
- 가림 창은 `CGShieldingWindowLevel` 레벨의 테두리 없는 `NSWindow`이며, `NSApplicationPresentationOptions`로 Dock·메뉴바 숨김과 앱 전환 차단을 적용합니다.
- 자리 비움 감지는 `CGEventSourceSecondsSinceLastEventType`을 사용합니다.

## 한계

- 의지가 있다면 우회할 수 있습니다 (강제 종료, 비상 해제 문구, 다른 기기에서 SSH로 종료 등). 이 앱의 목적은 "일어나는 것이 가장 편한 선택"이 되게 만드는 것입니다.
- 앱 아이콘은 아직 없습니다.
