import { useState } from 'react'
import type { Rule, Site } from '../types'
import { RuleEditor } from './RuleEditor'

type Props = {
  site: Site
  onSave: (site: Site) => void
  onCancel: () => void
}

export function SiteDialog({ site, onSave, onCancel }: Props) {
  const [draft, setDraft] = useState<Site>(site)

  const isValid = draft.domain.trim().length > 0 && draft.upstream.trim().length > 0

  const addRule = () => {
    const newRule: Rule = { kind: 'proxy', path: '/api/*', target: 'localhost:8080' }
    setDraft({ ...draft, rules: [...draft.rules, newRule] })
  }

  const updateRule = (index: number, rule: Rule) => {
    const rules = draft.rules.map((r, i) => (i === index ? rule : r))
    setDraft({ ...draft, rules })
  }

  const removeRule = (index: number) => {
    setDraft({ ...draft, rules: draft.rules.filter((_, i) => i !== index) })
  }

  return (
    <div className="dialog-backdrop" onClick={onCancel}>
      <div className="dialog" onClick={(e) => e.stopPropagation()}>
        <div className="dialog-header">
          <h2>{site.domain ? `${site.domain} 편집` : '새 사이트'}</h2>
        </div>

        <div className="dialog-body">
          <label className="field">
            <span className="field-label">도메인</span>
            <input
              type="text"
              value={draft.domain}
              placeholder="myapp.test"
              onChange={(e) => setDraft({ ...draft, domain: e.target.value.trim() })}
              autoFocus
            />
            <span className="field-hint">도메인은 /etc/hosts 에 자동 등록됩니다.</span>
          </label>

          <label className="field">
            <span className="field-label">기본 Upstream</span>
            <input
              type="text"
              value={draft.upstream}
              placeholder="localhost:3000"
              onChange={(e) => setDraft({ ...draft, upstream: e.target.value.trim() })}
            />
            <span className="field-hint">규칙에 매칭되지 않는 모든 요청이 이리로 갑니다.</span>
          </label>

          <div className="field">
            <div className="field-label-row">
              <span className="field-label">경로 규칙</span>
              <button className="btn-ghost" onClick={addRule}>+ 규칙 추가</button>
            </div>
            {draft.rules.length === 0 ? (
              <div className="muted small">
                경로별로 다른 동작이 필요할 때만 추가하세요. (예: /api/* 만 8080으로 분기)
              </div>
            ) : (
              <>
                <div className="rule-list">
                  {draft.rules.map((rule, i) => (
                    <RuleEditor
                      key={i}
                      rule={rule}
                      onChange={(r) => updateRule(i, r)}
                      onRemove={() => removeRule(i)}
                    />
                  ))}
                </div>
                <div className="muted small">
                  프록시 타겟에 관리 중인 도메인(자기 자신 포함)을 넣으면 /etc/hosts 루프를
                  피해 실제 원격 IP로 자동 연결됩니다. Host 헤더와 SNI는 원본 도메인으로 유지됩니다.
                </div>
              </>
            )}
          </div>
        </div>

        <div className="dialog-footer">
          <button className="btn-ghost" onClick={onCancel}>취소</button>
          <button
            className="btn-primary"
            disabled={!isValid}
            onClick={() => onSave(draft)}
          >
            저장
          </button>
        </div>
      </div>
    </div>
  )
}
