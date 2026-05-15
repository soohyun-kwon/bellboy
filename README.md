# Perch

로컬 개발 서버를 `myapp.test` 같은 도메인 위에 **얹어주는(perch)** 맥용 미니 앱.
Caddy를 엔진으로 쓰고, `/etc/hosts` 편집·Caddy 프로세스 관리·경로별 라우팅 규칙을
GUI에서 한 번에 다룹니다. Ophiuchi(nginx 기반)의 Caddy 버전이라고 보면 됩니다.

## 주요 기능

- 도메인 ↔ `localhost:port` 매핑을 추가·편집·삭제
- 경로별 규칙: **프록시 / 정적 파일 / 제외(404)**
- `/etc/hosts` 자동 동기화 (관리자 권한 프롬프트 1회)
- Caddy start / stop / 핫 리로드
- 설정은 `~/Library/Application Support/perch/config.json`
- 생성되는 Caddyfile: `~/Library/Application Support/perch/Caddyfile`

## 요구 사항

- macOS (Apple Silicon / Intel)
- [Caddy](https://caddyserver.com/) — `brew install caddy`
- [Node.js](https://nodejs.org/) 20 이상
- [Rust](https://www.rust-lang.org/tools/install) — `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`

## 개발 실행

```bash
npm install
npm run tauri dev
```

첫 실행에서 Rust 의존성을 빌드하느라 몇 분 걸립니다. 이후엔 빠릅니다.

## 빌드 (.app / .dmg)

```bash
npm run tauri build
```

결과물은 `src-tauri/target/release/bundle/` 아래에 생성됩니다.

## 아키텍처

```
┌─────────────────┐      invoke       ┌─────────────────────────┐
│  React (Vite)   │ ────────────────▶ │  Rust (Tauri commands)  │
│  src/App.tsx    │                   │  src-tauri/src/lib.rs   │
└─────────────────┘                   └──────────┬──────────────┘
                                                 │
           ┌─────────────────────────────────────┼─────────────────────────┐
           ▼                                     ▼                         ▼
   ┌──────────────┐                     ┌────────────────┐        ┌────────────────┐
   │ config_store │                     │   caddyfile    │        │     hosts      │
   │ JSON persist │                     │   generator    │        │  /etc/hosts    │
   └──────────────┘                     └────────┬───────┘        │  via osascript │
                                                 │                └────────────────┘
                                                 ▼
                                        ┌────────────────┐
                                        │ caddy_process  │
                                        │ spawn / reload │
                                        └────────────────┘
```

Rust 쪽 모듈은 각자 책임이 좁고, 대부분 단위 테스트가 붙어 있습니다.

```bash
cd src-tauri
cargo test
```

## 경로 규칙 예시

`myapp.test` → 기본 `localhost:3000`인데 `/api/*`만 `localhost:8080`으로 보내고
`/admin/*`은 아예 막고 싶다면:

| 종류 | 경로 | 값 |
| --- | --- | --- |
| 프록시 | `/api/*` | `localhost:8080` |
| 제외(404) | `/admin/*` | — |

생성되는 Caddyfile:

```caddy
myapp.test {
    handle /api/* {
        reverse_proxy localhost:8080
    }
    handle /admin/* {
        respond 404
    }
    handle {
        reverse_proxy localhost:3000
    }
}
```

## 로드맵

- [ ] 메뉴바 트레이 UI (백그라운드 실행)
- [ ] Caddy 바이너리 번들링 (별도 설치 불필요)
- [ ] SMAppService 권한 헬퍼 (매번 admin 프롬프트 제거)
- [ ] 인증서 신뢰 상태 표시 / 버튼
- [ ] Ophiuchi 설정 import
- [ ] 템플릿: Next.js, Vite, Rails 개발 서버 프리셋
- [ ] 상태 로그 뷰어 (Caddy access log)
- [ ] Linux 지원

## 라이선스

MIT
