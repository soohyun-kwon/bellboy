# Perch

로컬 개발 서버를 `myapp.test` 같은 도메인 위에 **얹어주는(perch)** 맥용 미니 앱.
Caddy를 엔진으로 쓰고, `/etc/hosts` 편집·Caddy 프로세스 관리·경로별 라우팅 규칙을
GUI에서 한 번에 다룹니다. Ophiuchi(nginx 기반)의 Caddy 버전이라고 보면 됩니다.

## 주요 기능

- 도메인 ↔ `localhost:port` 매핑을 추가·편집·삭제
- 경로별 규칙: **프록시 / 정적 파일 / 제외(404)**
- 프록시 규칙별 **환경 프리셋** — SiteCard에서 원클릭으로 업스트림 전환 (로컬 ↔ 스테이징 등)
- `/etc/hosts` 자동 동기화 (관리자 권한 프롬프트 1회)
- Caddy start / stop / 핫 리로드
- HTTPS(`tls internal`) + 로컬 루트 CA **키체인 신뢰 등록 / 상태 표시**
- **Node.js TLS 자동 설정**(`NODE_EXTRA_CA_CERTS`) — Next.js SSR 등에서 로컬 HTTPS 인증서 오류 방지
- 외부 / 잔여 Caddy 프로세스 **감지·정리** (포트 80·443·2019 충돌 방지)
- 첫 권한 사용 시 sudoers + Touch ID 자동 설치 (이후 무프롬프트)
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
        ┌────────────────────────────┬──────────┼───────────┬────────────────────┐
        ▼                            ▼          ▼           ▼                    ▼
┌──────────────┐            ┌────────────────┐  │  ┌────────────────┐   ┌────────────────┐
│ config_store │            │   caddyfile    │  │  │     hosts      │   │  system_trust  │
│ JSON persist │            │   generator    │  │  │  /etc/hosts    │   │  키체인 CA 신뢰 │
└──────────────┘            └────────┬───────┘  │  └───────┬────────┘   └────────────────┘
                                     │ dns       │          │
                                     ▼ (루프 방지) ▼          ▼ via auth_helper (sudoers/Touch ID)
                          ┌────────────────┐ ┌────────────────┐
                          │ caddy_process  │ │ caddy_supervisor│
                          │ spawn / reload │ │ 외부 caddy 감지 │
                          └────────────────┘ └────────────────┘
                                     │
                                     ▼ (start 직후)
                          ┌─────────────────────────────┐
                          │ node_env  NODE_EXTRA_CA_CERTS│
                          │ caddy_permissions  권한 점검  │
                          └─────────────────────────────┘
```

Rust 쪽 모듈(`src-tauri/src/`)은 각자 책임이 좁고, 대부분 단위 테스트가 붙어 있습니다.

| 모듈 | 책임 |
| --- | --- |
| `lib.rs` | Tauri command 정의·등록, 부팅 시 상태 정리 |
| `model.rs` | 도메인 모델 (`Config` → `Site` → `Rule`) |
| `config_store.rs` | `config.json` · Caddyfile 경로 + load/save |
| `caddyfile.rs` | 사이트 목록 → Caddyfile 생성 (`tls internal`, 루프백 방지) |
| `caddy_process.rs` | `caddy run` 자식 프로세스 spawn / stop / reload |
| `caddy_supervisor.rs` | 프로세스 테이블 스캔, 외부·잔여 caddy 분류·정리, admin API 헬스체크 |
| `caddy_permissions.rs` | Caddy 데이터 디렉터리 권한 사전 점검·복구 |
| `hosts.rs` | `/etc/hosts` Perch 블록 동기화 |
| `auth_helper.rs` | 권한 작업을 osascript 1회로 처리 (sudoers + Touch ID 설치) |
| `dns.rs` | 공개 DNS 조회 (caddyfile 루프 방지용 외부 IP 해석) |
| `node_env.rs` | `NODE_EXTRA_CA_CERTS` 관리 (system CA + Caddy 로컬 CA 합본) |
| `system_trust.rs` | Caddy 루트 CA의 macOS 키체인 신뢰 상태 조회·등록 |

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

- [x] 권한 헬퍼 (sudoers + Touch ID — 매번 admin 프롬프트 제거)
- [x] 인증서 신뢰 상태 표시 / 버튼
- [x] 프록시 환경 프리셋 (로컬 ↔ 스테이징 원클릭 전환)
- [ ] 메뉴바 트레이 UI (백그라운드 실행)
- [ ] Caddy 바이너리 번들링 (별도 설치 불필요)
- [ ] Ophiuchi 설정 import
- [ ] 상태 로그 뷰어 (Caddy access log)
- [ ] Linux 지원

## 라이선스

MIT
