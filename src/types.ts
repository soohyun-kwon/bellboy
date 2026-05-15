export type Rule =
  | { kind: 'proxy'; path: string; target: string }
  | { kind: 'static'; path: string; root: string }
  | { kind: 'bypass'; path: string }

export type Site = {
  id: string
  domain: string
  upstream: string
  enabled: boolean
  rules: Rule[]
}

export type Config = {
  sites: Site[]
}

export const emptySite = (): Site => ({
  id: crypto.randomUUID(),
  domain: '',
  upstream: 'localhost:3000',
  enabled: true,
  rules: [],
})
