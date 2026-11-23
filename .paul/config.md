# Project Config

**Project:** vtradingview
**Created:** 2026-11-23

## Mode

```yaml
mode: full   # lite (default) | standard | full; see paul-config "Mode"
```

## Project Settings

```yaml
project:
  name: vtradingview
  version: 0.0.0
```

## Tools

```yaml
tools:
  container: kiri       # default; docker is the fallback
  browser: obscura      # default; playwright is the fallback
```

## Integrations

### SonarQube

```yaml
sonarqube:
  enabled: false
```

### Enterprise Plan Audit

```yaml
enterprise_plan_audit:
  enabled: false
```

### Security Audit

```yaml
security_audit:
  enabled: true
  default_profile: standard
```

## Special Flows

```yaml
special_flows:
  pstack: enabled
  pstack_stage: plan
  uxui: enabled            # lite customization: design track D3 done for a rendered surface
  design_research: disabled # the spec does not yet specify motion (C17 open)
  row_overrides: {}
  disabled_rows: []
```

## Preferences

```yaml
preferences:
  auto_commit: false
  verbose_output: false
  debt_store: true
  learnings: true
  parallel_agents: true
  reflect: pstack
```

---
*Config created: 2026-11-23 by /seed launch (lite defaults; uxui on). Updated 2026-11-23 via /paul-config: mode full.*
