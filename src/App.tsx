import { useCallback, useEffect, useState } from 'react'
import {
  api,
  formatError,
  healthLevel,
  isForeignCaddyError,
  isPermissionRepairError,
} from './api'
import type {
  AuthHelperStatus,
  CaddyHealth,
  CertificateTrustStatus,
  ForeignCaddyError,
  ProcessInfo,
} from './api'
import type { Site, Config } from './types'
import { emptySite } from './types'
import { StatusBar } from './components/StatusBar'
import { SiteCard } from './components/SiteCard'
import { SiteDialog } from './components/SiteDialog'

type RepairPrompt = { message: string; path: string }
type TrustPrompt = CertificateTrustStatus
type ForeignPrompt = {
  message: string
  perchOwned: ProcessInfo[]
  external: ProcessInfo[]
  /** Whether confirming should retry `start_caddy` (true) or just kill (false). */
  retryAfterKill: boolean
}

export default function App() {
  const [config, setConfig] = useState<Config>({ sites: [] })
  const [health, setHealth] = useState<CaddyHealth | null>(null)
  const [authHelper, setAuthHelper] = useState<AuthHelperStatus | null>(null)
  const [editing, setEditing] = useState<Site | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [repairPrompt, setRepairPrompt] = useState<RepairPrompt | null>(null)
  const [trustPrompt, setTrustPrompt] = useState<TrustPrompt | null>(null)
  const [foreignPrompt, setForeignPrompt] = useState<ForeignPrompt | null>(null)
  const [showAuthSetup, setShowAuthSetup] = useState(false)
  const [authTouchId, setAuthTouchId] = useState(true)
  const [certificateTrust, setCertificateTrust] = useState<CertificateTrustStatus | null>(null)

  const isRunning = health?.is_running ?? false
  const level = healthLevel(health)

  const refreshCertificateTrustStatus = useCallback(async () => {
    setCertificateTrust(await api.getCertificateTrustStatus())
  }, [])

  const refreshHealth = useCallback(async () => {
    setRefreshing(true)
    try {
      const [h, trust] = await Promise.all([
        api.refreshHealth(),
        api.getCertificateTrustStatus(),
      ])
      setHealth(h)
      setCertificateTrust(trust)
    } catch (e) {
      setError(formatError(e))
    } finally {
      setRefreshing(false)
    }
  }, [])

  const loadAll = useCallback(async () => {
    try {
      const [c, h, trust, auth] = await Promise.all([
        api.getConfig(),
        api.refreshHealth(),
        api.getCertificateTrustStatus(),
        api.getAuthHelperStatus(),
      ])
      setConfig(c)
      setHealth(h)
      setCertificateTrust(trust)
      setAuthHelper(auth)
    } catch (e) {
      setError(String(e))
    }
  }, [])

  useEffect(() => {
    loadAll()
    // Lightweight poll: just the running flag. Full health (PID scan + admin
    // API probe) runs on focus and manual refresh instead — it's heavier and
    // the user-driven triggers cover the cases the poll misses.
    const poll = setInterval(async () => {
      try {
        const running = await api.caddyStatus()
        setHealth((prev) => (prev ? { ...prev, is_running: running } : prev))
      } catch {
        /* ignore */
      }
    }, 2000)
    return () => clearInterval(poll)
  }, [loadAll])

  useEffect(() => {
    // External tooling (terminal kills, brew services, system sleep) can mutate
    // caddy state while Perch is in the background. Resync the moment the user
    // brings the window back.
    const onFocus = () => {
      refreshHealth()
    }
    window.addEventListener('focus', onFocus)
    return () => window.removeEventListener('focus', onFocus)
  }, [refreshHealth])

  const runStart = async () => {
    await api.startCaddy()
    await refreshHealth()
    await refreshCertificateTrustStatus()
  }

  const handleStartStop = async () => {
    setBusy(true)
    setError(null)
    try {
      if (isRunning) {
        await api.stopCaddy()
        await refreshHealth()
      } else {
        await runStart()
      }
    } catch (e) {
      if (isPermissionRepairError(e)) {
        setRepairPrompt({ message: e.message, path: e.path })
      } else if (isForeignCaddyError(e)) {
        setForeignPrompt(foreignPromptFromError(e, true))
      } else {
        setError(formatError(e))
      }
    } finally {
      setBusy(false)
    }
  }

  const handleConfirmRepair = async () => {
    if (!repairPrompt) return
    setRepairPrompt(null)
    setBusy(true)
    setError(null)
    try {
      await api.repairCaddyPermissions()
      await runStart()
    } catch (e) {
      setError(formatError(e))
    } finally {
      setBusy(false)
    }
  }

  const handleConfirmKillForeign = async () => {
    if (!foreignPrompt) return
    const { perchOwned, external, retryAfterKill } = foreignPrompt
    setForeignPrompt(null)
    setBusy(true)
    setError(null)
    try {
      const pids = [...perchOwned, ...external].map((p) => p.pid)
      await api.killForeignCaddy(pids)
      if (retryAfterKill) {
        await runStart()
      } else {
        await refreshHealth()
      }
    } catch (e) {
      setError(formatError(e))
    } finally {
      setBusy(false)
    }
  }

  const handleRequestTrustCertificate = () => {
    if (!certificateTrust) return
    setTrustPrompt(certificateTrust)
  }

  const handleConfirmTrustCertificate = async () => {
    if (!trustPrompt) return
    setTrustPrompt(null)
    setBusy(true)
    setError(null)
    try {
      setCertificateTrust(await api.trustCaddyCertificate())
    } catch (e) {
      setError(formatError(e))
    } finally {
      setBusy(false)
    }
  }

  const handleInstallAuthHelper = async () => {
    setShowAuthSetup(false)
    setBusy(true)
    setError(null)
    try {
      setAuthHelper(await api.installAuthHelper(authTouchId))
    } catch (e) {
      setError(formatError(e))
    } finally {
      setBusy(false)
    }
  }

  const handleUninstallAuthHelper = async () => {
    setBusy(true)
    setError(null)
    try {
      setAuthHelper(await api.uninstallAuthHelper(authHelper?.touchid_enabled ?? false))
    } catch (e) {
      setError(formatError(e))
    } finally {
      setBusy(false)
    }
  }

  const handleSaveSite = async (site: Site) => {
    const sites = config.sites.some((s) => s.id === site.id)
      ? config.sites.map((s) => (s.id === site.id ? site : s))
      : [...config.sites, site]
    await applyConfig({ ...config, sites })
    setEditing(null)
  }

  const handleDeleteSite = async (id: string) => {
    const sites = config.sites.filter((s) => s.id !== id)
    await applyConfig({ ...config, sites })
  }

  const handleToggleSite = async (id: string, enabled: boolean) => {
    const sites = config.sites.map((s) => (s.id === id ? { ...s, enabled } : s))
    await applyConfig({ ...config, sites })
  }

  const applyConfig = async (next: Config) => {
    setBusy(true)
    setError(null)
    try {
      await api.applyConfig(next)
      setConfig(next)
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  const handleWarningClick = () => {
    if (level !== 'warning' || !health) return
    if (health.sighting.kind === 'foreign') {
      setForeignPrompt({
        message: '다른 Caddy 프로세스가 감지되었습니다. 종료한 뒤 상태가 정상화되는지 확인합니다.',
        perchOwned: health.sighting.perch_owned,
        external: health.sighting.external,
        retryAfterKill: false,
      })
    }
  }

  return (
    <div className="app">
      <StatusBar
        isRunning={isRunning}
        busy={busy}
        healthLevel={level}
        healthHint={healthHint(health)}
        onToggle={handleStartStop}
        onRefresh={refreshHealth}
        refreshing={refreshing}
        certificateTrust={certificateTrust}
        onTrustCertificate={handleRequestTrustCertificate}
      />

      {level === 'warning' && (
        <div className="warning" onClick={handleWarningClick}>
          {warningMessage(health)}
        </div>
      )}

      {error && (
        <div className="error" onClick={() => setError(null)}>
          {error}
        </div>
      )}

      <main className="main">
        {config.sites.length === 0 ? (
          <div className="empty">
            <p>아직 등록된 사이트가 없어요.</p>
            <button className="btn-primary" onClick={() => setEditing(emptySite())}>
              첫 사이트 추가하기
            </button>
          </div>
        ) : (
          <div className="site-list">
            {config.sites.map((site) => (
              <SiteCard
                key={site.id}
                site={site}
                onEdit={() => setEditing(site)}
                onDelete={() => handleDeleteSite(site.id)}
                onToggle={(enabled) => handleToggleSite(site.id, enabled)}
              />
            ))}
          </div>
        )}
      </main>

      {config.sites.length > 0 && (
        <button className="fab" onClick={() => setEditing(emptySite())} aria-label="사이트 추가">
          +
        </button>
      )}

      {editing && (
        <SiteDialog
          site={editing}
          onSave={handleSaveSite}
          onCancel={() => setEditing(null)}
        />
      )}

      {repairPrompt && (
        <div className="dialog-backdrop" onClick={() => setRepairPrompt(null)}>
          <div className="dialog" onClick={(e) => e.stopPropagation()}>
            <div className="dialog-header">
              <h2>Caddy 권한 복구</h2>
            </div>
            <div className="dialog-body">
              <p>{repairPrompt.message}</p>
              <p className="muted small">대상 경로: {repairPrompt.path}</p>
              <p className="muted small">
                [복구]를 누르면 macOS 관리자 비밀번호 창이 뜹니다. 해당 폴더의 소유권이 현재 계정으로
                복구되고, 완료되면 Caddy를 자동으로 다시 시작합니다.
              </p>
            </div>
            <div className="dialog-footer">
              <button className="btn-ghost" onClick={() => setRepairPrompt(null)}>취소</button>
              <button className="btn-primary" onClick={handleConfirmRepair}>복구</button>
            </div>
          </div>
        </div>
      )}

      {foreignPrompt && (
        <div className="dialog-backdrop" onClick={() => setForeignPrompt(null)}>
          <div className="dialog" onClick={(e) => e.stopPropagation()}>
            <div className="dialog-header">
              <h2>다른 Caddy 프로세스 감지</h2>
            </div>
            <div className="dialog-body">
              <p>{foreignPrompt.message}</p>
              {foreignPrompt.perchOwned.length > 0 && (
                <>
                  <p className="muted small">Perch가 이전에 띄워둔 Caddy:</p>
                  <ul className="process-list">
                    {foreignPrompt.perchOwned.map((p) => (
                      <li key={p.pid}>
                        <code>PID {p.pid}</code> — {p.command}
                      </li>
                    ))}
                  </ul>
                </>
              )}
              {foreignPrompt.external.length > 0 && (
                <>
                  <p className="muted small">Perch 외부에서 실행 중인 Caddy:</p>
                  <ul className="process-list">
                    {foreignPrompt.external.map((p) => (
                      <li key={p.pid}>
                        <code>PID {p.pid}</code> — {p.command}
                      </li>
                    ))}
                  </ul>
                  <p className="muted small">
                    종료를 누르면 위 프로세스가 모두 SIGTERM으로 종료됩니다. 다른 도구가 띄운
                    Caddy라면 그 도구의 동작에 영향이 갈 수 있습니다.
                  </p>
                </>
              )}
            </div>
            <div className="dialog-footer">
              <button className="btn-ghost" onClick={() => setForeignPrompt(null)}>취소</button>
              <button className="btn-danger" onClick={handleConfirmKillForeign}>
                {foreignPrompt.retryAfterKill ? '종료 후 시작' : '종료'}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 관리자 권한 설정 footer */}
      <footer className="auth-footer">
        <div className="auth-footer-row">
          <span className="auth-status">
            <span className={`auth-dot ${authHelper?.is_installed ? 'installed' : 'not-installed'}`} />
            {authHelper === null
              ? '권한 설정 확인 중'
              : authHelper.is_installed
                ? `비밀번호 저장됨${authHelper.touchid_enabled ? ' · Touch ID 활성' : ''}`
                : '매 시작마다 비밀번호 입력 필요'}
          </span>
          {authHelper?.is_installed ? (
            <button className="btn-ghost small" onClick={handleUninstallAuthHelper} disabled={busy}>
              해제
            </button>
          ) : (
            <button className="btn-ghost small" onClick={() => setShowAuthSetup(true)} disabled={busy}>
              한 번만 인증 설정
            </button>
          )}
        </div>
      </footer>

      {/* Auth Helper 설치 다이얼로그 */}
      {showAuthSetup && (
        <div className="dialog-backdrop" onClick={() => setShowAuthSetup(false)}>
          <div className="dialog" onClick={(e) => e.stopPropagation()}>
            <div className="dialog-header">
              <h2>관리자 권한 저장</h2>
            </div>
            <div className="dialog-body">
              <p>
                지금 한 번만 비밀번호를 입력하면 이후 Perch를 껐다 켜도 다시 묻지 않습니다.
              </p>
              <p className="muted small">
                <code>/etc/sudoers.d/perch</code>에 두 가지 명령만 NOPASSWD로 등록합니다:
                hosts 동기화(<code>/bin/cp</code>)와 Caddy 소유권 복구(<code>/usr/sbin/chown</code>).
                다른 sudo 명령에는 영향을 주지 않습니다.
              </p>
              <label className="auth-option">
                <input
                  type="checkbox"
                  checked={authTouchId}
                  onChange={(e) => setAuthTouchId(e.target.checked)}
                />
                <span>
                  Touch ID로 sudo 인증 활성화
                  <span className="muted small"> (macOS 13.3+, /etc/pam.d/sudo_local)</span>
                </span>
              </label>
            </div>
            <div className="dialog-footer">
              <button className="btn-ghost" onClick={() => setShowAuthSetup(false)}>취소</button>
              <button className="btn-primary" onClick={handleInstallAuthHelper}>
                설정 (비밀번호 1회 입력)
              </button>
            </div>
          </div>
        </div>
      )}

      {trustPrompt && (
        <div className="dialog-backdrop" onClick={() => setTrustPrompt(null)}>
          <div className="dialog" onClick={(e) => e.stopPropagation()}>
            <div className="dialog-header">
              <h2>Caddy 인증서 신뢰 등록</h2>
            </div>
            <div className="dialog-body">
              <p>{trustPrompt.message}</p>
              {trustPrompt.rootPath && (
                <p className="muted small">대상 인증서: {trustPrompt.rootPath}</p>
              )}
              <p className="muted small">
                [등록]을 누르면 macOS 관리자 비밀번호 창이 뜹니다. Caddy 로컬 CA를 시스템
                Keychain에 신뢰 루트로 추가합니다.
              </p>
              <p className="muted small">{trustPrompt.nodeHint}</p>
            </div>
            <div className="dialog-footer">
              <button className="btn-ghost" onClick={() => setTrustPrompt(null)}>취소</button>
              <button className="btn-primary" onClick={handleConfirmTrustCertificate}>등록</button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

function foreignPromptFromError(e: ForeignCaddyError, retryAfterKill: boolean): ForeignPrompt {
  return {
    message: e.message,
    perchOwned: e.perch_owned,
    external: e.external,
    retryAfterKill,
  }
}

function healthHint(health: CaddyHealth | null): string | undefined {
  if (!health) return undefined
  const bits: string[] = []
  bits.push(health.is_running ? 'Caddy 프로세스: 실행 중' : 'Caddy 프로세스: 중지')
  bits.push(`Admin API(:2019): ${health.admin_api_reachable ? '응답 OK' : '응답 없음'}`)
  if (health.sighting.kind === 'foreign') {
    const count =
      health.sighting.perch_owned.length + health.sighting.external.length
    bits.push(`외부 Caddy ${count}개 감지`)
  }
  return bits.join('\n')
}

function warningMessage(health: CaddyHealth | null): string {
  if (!health) return ''
  if (health.sighting.kind === 'foreign') {
    return '다른 Caddy 프로세스가 감지되었습니다. 클릭해 정리하세요.'
  }
  if (!health.admin_api_reachable && health.is_running) {
    return 'Caddy는 실행 중인데 admin API(:2019)에 응답이 없어요. 재시작이 필요할 수 있습니다.'
  }
  return ''
}
