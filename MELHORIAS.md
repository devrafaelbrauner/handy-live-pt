# Melhorias futuras

Ideias de aprimoramento e funcionalidades fora do escopo atual.
Nada aqui é implementado sem combinar antes — é um estacionamento de ideias.

## Design

- Trocar os usos ad hoc de `red-500`/`yellow-500` do Tailwind
  (`ui/Alert.tsx`, `ui/Button.tsx`, `settings/debug/KeyboardDiagnostic.tsx`)
  pelos tokens `--color-error`/`--color-warning`, como `src/styles/theme.css`
  já prevê.
- Documentar a paleta num `DESIGN.md` que aponte `src/styles/theme.css` como
  fonte única. Sem ele, o baseline do NEXUS classifica o projeto como
  GREENFIELD e uma fase de design partiria do zero.

## Distribuição

- Publicar releases do fork. Hoje o updater aponta para releases sem
  `latest.json` (no-op) e `createUpdaterArtifacts` está desligado por falta de
  chave de assinatura.

## Engenharia

- Versionar em `docs/` as pesquisas que o ULTRAPLAN cita como evidência
  (`research-models.md`, `research-formatting.md`, `research-codebase.md`,
  `ULTRAPLAN-critique.md`). O PR #8 versiona só o plano; as pesquisas ficaram
  em `.nexus/runs`, fora do git.
