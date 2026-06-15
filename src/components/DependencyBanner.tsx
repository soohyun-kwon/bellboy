import type { DependencyStatus } from '../api'

type Props = {
  status: DependencyStatus
  installing: boolean
  onInstall: () => void
}

/**
 * Caddy 미설치 시 StatusBar 아래에 뜨는 안내 배너.
 * - Homebrew 있음: `brew install caddy` 원클릭 설치 버튼
 * - Homebrew 없음: 자동 설치 불가 — Homebrew 선설치 안내
 * Caddy가 이미 설치돼 있으면 아무것도 렌더하지 않는다.
 */
export function DependencyBanner({ status, installing, onInstall }: Props) {
  if (status.caddy_installed) return null

  if (!status.homebrew_installed) {
    return (
      <div className="dependency-banner">
        <div className="dependency-text">
          <strong>Caddy가 설치되어 있지 않습니다.</strong>
          <span className="muted small">
            자동 설치에는 Homebrew가 필요합니다. <code>brew.sh</code>에서 Homebrew를
            먼저 설치한 뒤 상태를 새로고침하세요.
          </span>
        </div>
      </div>
    )
  }

  return (
    <div className="dependency-banner">
      <div className="dependency-text">
        <strong>Caddy가 설치되어 있지 않습니다.</strong>
        <span className="muted small">
          Bellboy의 엔진인 Caddy를 Homebrew로 설치합니다 (<code>brew install caddy</code>).
        </span>
      </div>
      <button className="btn-primary" onClick={onInstall} disabled={installing}>
        {installing ? '설치 중…' : 'Caddy 설치'}
      </button>
    </div>
  )
}
