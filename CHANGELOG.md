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
- Harness de avaliação `--manifest` (F0a do `docs/ULTRAPLAN.md`, #13):
  `handy --manifest m.jsonl [--model ID] [--language CODE] [--vad off|silero|earshot]`
  transcreve cada WAV do manifest JSONL pelo mesmo caminho do app (VAD do
  recorder com o perfil offline, padding de clipe curto, transcrição batch com
  os estágios de texto, OpenCC) e imprime uma linha JSON por fala (texto cru,
  final, cada estágio, tempos) e uma linha de resumo.
- Métricas da F0a (#14): `scripts/f0a_metrics.py` junta a saída do harness
  com o manifest pelo `id` e reporta WER/CER normalizado, WER formatado, P/R/F1
  de `, . ? ! :`, ITN exact-match, preservação (`raw` → `final`) e palavras
  inseridas nas sondas de silêncio, no total e por faixa de duração.
- Preparação da linha de base da F0a (#15): `scripts/f0a_fleurs.py` baixa 200
  falas do FLEURS pt_br (uma por frase) no formato do harness;
  `scripts/corpus_f0b.py export` gera o manifest do corpus F0b a partir do
  histórico do Handy; `scripts/f0a_metrics.py --baseline` compara duas
  execuções com IC 95% por bootstrap pareado.
- Trace de latência por gravação (F0a, #16): uma linha `Latency trace` no log
  do app por sessão, com press→1º frame, →1ª fala no VAD, →1º feed do engine,
  →1º texto ao vivo e release→texto final, →colar, mais modelo, caminho ao vivo
  e desfecho. Só medição, para diagnosticar a demora da 1ª palavra (issue #4).
- Suporte ao Windows no harness e nos scripts da F0a (#17):
  `handy --manifest … --out r.jsonl` grava os resultados em UTF-8 num arquivo,
  porque o `handy.exe` é um programa gráfico e o PowerShell não espera nem
  captura o stdout dele (rodar com `Start-Process -Wait`);
  `scripts/corpus_f0b.py` lê o histórico do sistema em uso (`%APPDATA%` no
  Windows) em vez do caminho do macOS.
- Transcrição na nuvem (porta do `handy-live-pt-cloud#1`): seção "API na
  Nuvem" nas configurações, desligada por padrão, com OpenAI, OpenRouter, Groq
  ou endpoint compatível com OpenAI (`POST {base}/audio/transcriptions`, WAV
  16 kHz mono, divisão em partes acima de 5 min). As chaves são as mesmas do
  pós-processamento. Com a nuvem ligada, o modelo local não é carregado e não
  há texto ao vivo; o harness e o `--transcribe-file` continuam locais.

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

### Fixed

- Pós-processamento por LLM era pulado em silêncio quando nenhum prompt estava
  selecionado, o estado de uma instalação nova (#18): agora usa o prompt
  selecionado ou, sem seleção, o primeiro ("Improve Transcriptions"), e a tela
  de configurações mostra o prompt em uso.
- Execuções headless (`--manifest`, `--transcribe-file`, `--list-*`) não
  gravam mais no `handy.log` do app (#19): um benchmark enchia o arquivo, a
  rotação apagava o log da sessão do usuário (inclusive as linhas
  `Latency trace`). Os logs headless vão só para o stderr.

### Removed

- `bundle.windows.signCommand` (Azure Trusted Signing do upstream) do
  `tauri.conf.json` e a entrada de troubleshooting correspondente no
  `BUILD.md` (#6).
- Gate de revisão Kilo (#11): o projeto não usa mais o KiloCode. Saem o job
  `kilo-review` do `ai-pipeline.yml` e o `.ai-pipeline/kilo-review.sh`.
- Aprovação humana no pipeline (#12): sai o job `human-approval` do
  `ai-pipeline.yml`. O pipeline passa a ser CI → `Pipeline complete`; o merge
  manual continua sendo a decisão humana (`.ai-pipeline/POLICY.md`).
