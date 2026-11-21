# Changelog

이 프로젝트는 [Semantic Versioning](https://semver.org/lang/ko/)을 따릅니다. 버전은 `Cargo.toml` 한 곳에서 관리하며, `v<버전>` 태그를 push하면 GitHub Actions가 Releases에 빌드를 올립니다.

## [Unreleased]

## [0.3.0] - 2026-11-21

### 추가
- 자세 귀띔: 작업 중 10~20분에 한 번(무작위) 거북목·허리·어깨 등 앉은 자세 안내를 2초간 전체 화면으로 표시. 초점·키 입력·마우스를 빼앗지 않으며, 자리 비움 중이거나 스트레칭 직전에는 건너뜀 (macOS, Windows)
- 메뉴 "자세 귀띔 (가끔 2초)" 켜고 끄기, 설정 파일 `posture_nudge`
- 명령줄 `--test-nudge`: 자세 귀띔을 한 번 띄우고 종료

## [0.2.0] - 2026-11-19

### 추가
- 자동 업데이트: 6시간마다 새 버전을 확인하고, 메뉴에서 누르면 받아서 교체한 뒤 다시 실행 (macOS, Windows). 명령줄 `--update-now`
- macOS: 메뉴에서 "로그인 시 자동 실행" 켜고 끄기
- 중복 실행 방지를 macOS에도 적용 (파일 잠금)

### 수정
- macOS 설정 창에서 자리 비움 체크박스 문구가 잘리던 문제
- README: macOS 15 이상에서 받은 앱을 여는 방법(개인정보 보호 및 보안 → Open Anyway)

## [0.1.0] - 2026-11-19

첫 공개 릴리스입니다.

### 추가
- 정해진 주기마다 모든 모니터를 전체 화면으로 가리는 강제 스트레칭 알림 (macOS, Windows)
- macOS: 메뉴바 상주(남은 시간 표시), 설정 창, 전체화면 앱·Spaces 위 가림, Dock·메뉴바 숨김, 앱 전환 차단
- Windows: 트레이 상주(전용 아이콘), 저수준 키보드 훅으로 키 입력 차단, 로그인 시 자동 실행 옵션
- 비상 해제 문구(한글 입력 상태에서도 동작), 가림이 예정보다 오래 남으면 스스로 종료하는 감시 스레드
- 자리 비움·잠자기 감지 시 주기 초기화, 1시간 일시정지
- `--test` 시험 실행 모드

### 검증
- macOS: 모니터 3대, 전체화면 앱, 비상 해제 실사용 확인
- Windows 11 (VMware, ARM64에서 x64 에뮬레이션): 가림 화면·작업 표시줄 가림, 주기·스트레칭 시간 정확도 확인

[Unreleased]: https://github.com/kaifacun/force-streach/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/kaifacun/force-streach/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/kaifacun/force-streach/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kaifacun/force-streach/releases/tag/v0.1.0
