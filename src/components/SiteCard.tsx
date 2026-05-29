import type { Site } from '../types'

type Props = {
  site: Site
  onEdit: () => void
  onDelete: () => void
  onToggle: (enabled: boolean) => void
  onSwitchRuleEnv: (ruleIndex: number, target: string) => void
}

export function SiteCard({ site, onEdit, onDelete, onToggle, onSwitchRuleEnv }: Props) {
  const ruleCount = site.rules.length
  const proxyRulesWithEnvs = site.rules
    .map((rule, i) => ({ rule, i }))
    .filter(({ rule }) => rule.kind === 'proxy' && rule.envs && rule.envs.length > 0)

  return (
    <div className={`site-card ${site.enabled ? '' : 'disabled'}`}>
      <div className="site-card-main">
        <div className="site-domain">
          <a
            href={`https://${site.domain}`}
            target="_blank"
            rel="noreferrer"
            onClick={(e) => !site.enabled && e.preventDefault()}
          >
            {site.domain || '(도메인 없음)'}
          </a>
        </div>
        <div className="site-meta">
          <span className="upstream">→ {site.upstream}</span>
          {ruleCount > 0 && <span className="rule-badge">경로 규칙 {ruleCount}</span>}
        </div>
        {proxyRulesWithEnvs.length > 0 && (
          <div className="proxy-envs">
            {proxyRulesWithEnvs.map(({ rule, i }) => {
              if (rule.kind !== 'proxy') return null
              return (
                <div key={i} className="proxy-env-row">
                  <span className="proxy-env-path">{rule.path}</span>
                  <div className="env-chips">
                    {(rule.envs ?? []).map((env) => (
                      <button
                        key={env.name}
                        className={`env-chip ${env.target === rule.target ? 'active' : ''}`}
                        onClick={() => onSwitchRuleEnv(i, env.target)}
                        title={env.target}
                      >
                        {env.name || env.target}
                      </button>
                    ))}
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </div>
      <div className="site-card-actions">
        <label className="toggle">
          <input
            type="checkbox"
            checked={site.enabled}
            onChange={(e) => onToggle(e.target.checked)}
          />
          <span className="toggle-slider" />
        </label>
        <button className="btn-ghost" onClick={onEdit}>편집</button>
        <button className="btn-ghost danger" onClick={onDelete}>삭제</button>
      </div>
    </div>
  )
}
