import { invoke } from '@tauri-apps/api/core'
import type { Config } from './types'

export const api = {
  getConfig: () => invoke<Config>('get_config'),
  applyConfig: (config: Config) => invoke<void>('apply_config', { config }),
  startCaddy: () => invoke<void>('start_caddy'),
  stopCaddy: () => invoke<void>('stop_caddy'),
  caddyStatus: () => invoke<boolean>('caddy_status'),
  refreshHealth: () => invoke<CaddyHealth>('refresh_health'),
  killForeignCaddy: (pids: number[]) => invoke<void>('kill_foreign_caddy', { pids }),
  generateCaddyfile: () => invoke<string>('generate_caddyfile'),
  repairCaddyPermissions: () => invoke<void>('repair_caddy_permissions'),
  getCertificateTrustStatus: () =>
    invoke<CertificateTrustStatus>('get_certificate_trust_status'),
  trustCaddyCertificate: () =>
    invoke<CertificateTrustStatus>('trust_caddy_certificate'),
  getNodeExtraCaCerts: () => invoke<boolean>('get_node_extra_ca_certs'),
  setNodeExtraCaCerts: (enabled: boolean) =>
    invoke<void>('set_node_extra_ca_certs', { enabled }),
  getDependencyStatus: () => invoke<DependencyStatus>('get_dependency_status'),
  installCaddy: () => invoke<DependencyStatus>('install_caddy'),
}

/** Caddy(엔진) / Homebrew 설치 여부. 미설치 배너 렌더에 사용. */
export type DependencyStatus = {
  caddy_installed: boolean
  caddy_path: string | null
  homebrew_installed: boolean
}

export type ProcessInfo = { pid: number; command: string }

export type CaddySighting =
  | { kind: 'none' }
  | { kind: 'ours_alive'; pid: number }
  | { kind: 'ours_dead' }
  | { kind: 'foreign'; bellboy_owned: ProcessInfo[]; external: ProcessInfo[] }

export type CaddyHealth = {
  is_running: boolean
  admin_api_reachable: boolean
  sighting: CaddySighting
}

/** UI traffic light derived from a health snapshot. */
export type HealthLevel = 'ok' | 'warning' | 'stopped'

export function healthLevel(health: CaddyHealth | null): HealthLevel {
  if (!health) return 'stopped'
  if (!health.is_running) return 'stopped'
  // Foreign caddies racing for the same ports, or an unresponsive admin API,
  // are the symptoms of the "TLS internal error" failure mode we're guarding
  // against — surface them as warning, not ok.
  if (health.sighting.kind === 'foreign') return 'warning'
  if (!health.admin_api_reachable) return 'warning'
  return 'ok'
}

export type CertificateTrustState = 'trusted' | 'untrusted' | 'root_missing'

export type CertificateTrustStatus = {
  state: CertificateTrustState
  rootPath: string | null
  message: string
  nodeHint: string
}

/** Structured error rejected by `start_caddy` when pre-flight permission check fails. */
export type PermissionRepairError = {
  kind: 'permission_repair_required'
  message: string
  path: string
}

export function isPermissionRepairError(e: unknown): e is PermissionRepairError {
  return (
    typeof e === 'object' &&
    e !== null &&
    'kind' in e &&
    (e as { kind: unknown }).kind === 'permission_repair_required'
  )
}

/** Rejected by `start_caddy` when a non-Bellboy caddy already holds the ports. */
export type ForeignCaddyError = {
  kind: 'foreign_caddy_detected'
  message: string
  bellboy_owned: ProcessInfo[]
  external: ProcessInfo[]
}

export function isForeignCaddyError(e: unknown): e is ForeignCaddyError {
  return (
    typeof e === 'object' &&
    e !== null &&
    'kind' in e &&
    (e as { kind: unknown }).kind === 'foreign_caddy_detected'
  )
}

export function formatError(e: unknown): string {
  if (typeof e === 'object' && e !== null && 'message' in e) {
    return String((e as { message: unknown }).message)
  }
  return String(e)
}
