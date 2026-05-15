import type { CertificateTrustStatus, HealthLevel } from '../api'

type Props = {
  isRunning: boolean
  busy: boolean
  healthLevel: HealthLevel
  healthHint?: string
  onToggle: () => void
  onRefresh: () => void
  refreshing: boolean
  certificateTrust: CertificateTrustStatus | null
  onTrustCertificate: () => void
}

export function StatusBar({
  isRunning,
  busy,
  healthLevel,
  healthHint,
  onToggle,
  onRefresh,
  refreshing,
  certificateTrust,
  onTrustCertificate,
}: Props) {
  const certificateLabel = certificateTrust
    ? certificateTrustLabel(certificateTrust.state)
    : '인증서 확인 중'
  const shouldShowTrustButton = certificateTrust?.state === 'untrusted'

  return (
    <header className="statusbar">
      <div className="brand">
        <span className="brand-dot" />
        <span className="brand-name">Perch</span>
      </div>
      <div className="status">
        <span
          className={`certificate-status ${certificateTrust?.state ?? 'checking'}`}
          title={certificateTrust ? `${certificateTrust.message}\n\n${certificateTrust.nodeHint}` : undefined}
        >
          {certificateLabel}
        </span>
        {shouldShowTrustButton && (
          <button className="btn-ghost" onClick={onTrustCertificate} disabled={busy}>
            신뢰 등록
          </button>
        )}
        <span
          className={`status-dot ${healthLevel}`}
          title={healthHint}
        />
        <span className="status-text">{statusLabel(healthLevel, isRunning)}</span>
        <button
          className="btn-ghost btn-icon"
          onClick={onRefresh}
          disabled={refreshing}
          title="상태 새로고침"
          aria-label="상태 새로고침"
        >
          {refreshing ? '⟳' : '↻'}
        </button>
        <button
          className={isRunning ? 'btn-danger' : 'btn-primary'}
          onClick={onToggle}
          disabled={busy}
        >
          {busy ? '...' : isRunning ? '중지' : '시작'}
        </button>
      </div>
    </header>
  )
}

function statusLabel(level: HealthLevel, isRunning: boolean): string {
  switch (level) {
    case 'ok':
      return 'Caddy 실행 중'
    case 'warning':
      return isRunning ? 'Caddy 상태 이상' : 'Caddy 상태 이상'
    case 'stopped':
      return 'Caddy 중지됨'
  }
}

function certificateTrustLabel(state: CertificateTrustStatus['state']): string {
  switch (state) {
    case 'trusted':
      return '인증서 신뢰됨'
    case 'untrusted':
      return '인증서 미신뢰'
    case 'root_missing':
      return '인증서 없음'
  }
}
