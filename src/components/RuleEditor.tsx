import type { Rule } from '../types'

type Props = {
  rule: Rule
  onChange: (rule: Rule) => void
  onRemove: () => void
}

export function RuleEditor({ rule, onChange, onRemove }: Props) {
  const setKind = (kind: Rule['kind']) => {
    if (kind === rule.kind) return
    if (kind === 'proxy') onChange({ kind, path: rule.path, target: 'localhost:8080' })
    else if (kind === 'static') onChange({ kind, path: rule.path, root: '' })
    else onChange({ kind, path: rule.path })
  }

  return (
    <div className="rule-row">
      <select
        value={rule.kind}
        onChange={(e) => setKind(e.target.value as Rule['kind'])}
        className="rule-kind"
      >
        <option value="proxy">프록시</option>
        <option value="static">정적 파일</option>
        <option value="bypass">제외(404)</option>
      </select>

      <input
        type="text"
        value={rule.path}
        placeholder="/api/*"
        onChange={(e) => onChange({ ...rule, path: e.target.value })}
        className="rule-path"
      />

      {rule.kind === 'proxy' && (
        <input
          type="text"
          value={rule.target}
          placeholder="localhost:8080 또는 https://api.example.com"
          onChange={(e) => onChange({ ...rule, target: e.target.value })}
          className="rule-value"
        />
      )}

      {rule.kind === 'static' && (
        <input
          type="text"
          value={rule.root}
          placeholder="/Users/me/project/public"
          onChange={(e) => onChange({ ...rule, root: e.target.value })}
          className="rule-value"
        />
      )}

      {rule.kind === 'bypass' && <div className="rule-value muted">(요청 차단)</div>}

      <button className="btn-ghost danger icon" onClick={onRemove} aria-label="규칙 삭제">
        ×
      </button>
    </div>
  )
}
