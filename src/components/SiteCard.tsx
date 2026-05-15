import type { Site } from '../types'

type Props = {
  site: Site
  onEdit: () => void
  onDelete: () => void
  onToggle: (enabled: boolean) => void
}

export function SiteCard({ site, onEdit, onDelete, onToggle }: Props) {
  const ruleCount = site.rules.length

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
