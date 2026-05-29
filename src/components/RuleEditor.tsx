import type { ProxyEnvPreset, Rule } from '../types'

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

  const addEnvPreset = () => {
    if (rule.kind !== 'proxy') return
    const preset: ProxyEnvPreset = { name: '', target: rule.target }
    onChange({ ...rule, envs: [...(rule.envs ?? []), preset] })
  }

  const updateEnvPreset = (i: number, preset: ProxyEnvPreset) => {
    if (rule.kind !== 'proxy') return
    const envs = (rule.envs ?? []).map((e, j) => (j === i ? preset : e))
    onChange({ ...rule, envs })
  }

  const removeEnvPreset = (i: number) => {
    if (rule.kind !== 'proxy') return
    onChange({ ...rule, envs: (rule.envs ?? []).filter((_, j) => j !== i) })
  }

  const activateEnvPreset = (target: string) => {
    if (rule.kind !== 'proxy') return
    onChange({ ...rule, target })
  }

  return (
    <div className="rule-entry">
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

      {rule.kind === 'proxy' && (
        <div className="rule-envs">
          <div className="rule-envs-header">
            <span>환경 프리셋</span>
            <button className="btn-ghost" style={{ fontSize: 11, padding: '2px 6px' }} onClick={addEnvPreset}>
              + 추가
            </button>
          </div>
          {(rule.envs ?? []).length === 0 ? (
            <span className="muted small">프리셋을 추가하면 SiteCard에서 원클릭으로 환경 전환할 수 있습니다.</span>
          ) : (
            (rule.envs ?? []).map((env, i) => (
              <div key={i} className="env-preset-row">
                <input
                  type="text"
                  value={env.name}
                  placeholder="로컬"
                  onChange={(e) => updateEnvPreset(i, { ...env, name: e.target.value })}
                  className="env-preset-name"
                />
                <input
                  type="text"
                  value={env.target}
                  placeholder="localhost:8080"
                  onChange={(e) => updateEnvPreset(i, { ...env, target: e.target.value })}
                  className="env-preset-target"
                />
                <button
                  className={`env-chip ${env.target === rule.target ? 'active' : ''}`}
                  onClick={() => activateEnvPreset(env.target)}
                  title="이 환경 활성화"
                >
                  {env.target === rule.target ? '적용 중' : '적용'}
                </button>
                <button className="btn-ghost danger icon" onClick={() => removeEnvPreset(i)} aria-label="프리셋 삭제">
                  ×
                </button>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  )
}
