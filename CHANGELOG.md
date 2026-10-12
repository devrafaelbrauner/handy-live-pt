# Changelog

Todas as mudanças notáveis deste fork serão documentadas neste arquivo.

Formato baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/),
e este projeto segue [Versionamento Semântico](https://semver.org/lang/pt-BR/).

Base: Handy [v0.9.6](https://github.com/cjpais/Handy/releases/tag/v0.9.6)
(`cjpais/Handy`, commit `ba10ce1`, 2026-09-15). O histórico anterior ao fork
está nas releases do upstream. A versão do pacote continua `0.9.6`: o fork
ainda não publicou release.

## [Não publicado]

### Added

- Fork "Handy Live PT" (`990e6b0`): `productName` "Handy Live PT",
  identifier `com.rafael.handylive`, ícones próprios e aviso de fork no README.
- Pipeline de entrega com IA (`990e6b0`): `.github/workflows/ai-pipeline.yml`
  (CI → revisão Kilo → aprovação humana) e `.ai-pipeline/` (`ci.sh`,
  `kilo-review.sh`, `POLICY.md`). `ci.sh` usa bun e roda `cargo test` via
  `src-tauri/Cargo.toml`.
- Teste da matriz completa `OverlayStyle` × `is_streaming` no gate do overlay
  de streaming, usado para validar os gates do pipeline ponta a ponta (#2).
- `stream_latency_preset` (#3): troca latência por precisão no streaming nativo
  (`LowLatency`, `Balanced`, `HighLatency`, `Maximum`). O padrão `Maximum` não
  sobrescreve a família e mantém o comportamento anterior. Seletor nas
  configurações experimentais, textos em todos os idiomas, 5 testes novos.
- Modo ao vivo `Preview` (#5): legenda ao vivo para modelos batch (família
  Whisper), transcrevendo cada segmento de VAD quando a fala pausa. O texto
  colado continua vindo da transcrição batch completa no stop. O padrão
  `Standard` mantém o comportamento anterior; `chunks_paste` fica reservado e
  desabilitado na UI.

### Changed

- Updater aponta para as releases do fork, que não têm `latest.json`
  publicado; na prática é no-op (`990e6b0`).
- `createUpdaterArtifacts` desligado para compilar sem
  `TAURI_SIGNING_PRIVATE_KEY` (`6de4a6b`).
- "Main Branch Build" sem assinatura (#6): `main-build.yml` com
  `sign-binaries: false` e novo passo "Build with Tauri (unsigned)" em
  `build.yml`. Windows sai sem assinatura; macOS sai assinado ad hoc (`-`).
- Builds macOS sem assinatura carregam as dylibs empacotadas (#7): uma cópia
  temporária do `Entitlements.plist` recebe
  `com.apple.security.cs.disable-library-validation`; o hardened runtime
  continua ligado e uma guarda de inicialização roda `handy --help`.

### Removed

- `bundle.windows.signCommand` (Azure Trusted Signing do upstream) do
  `tauri.conf.json` e a entrada de troubleshooting correspondente no
  `BUILD.md` (#6).
- Gate de revisão Kilo (#11): o projeto não usa mais o KiloCode. Saem o job
  `kilo-review` do `ai-pipeline.yml` e o `.ai-pipeline/kilo-review.sh`; o
  pipeline passa a ser CI → aprovação humana → `Pipeline complete`.
