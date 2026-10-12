# Tarefas pendentes

Backlog do que falta fazer. Prioridades: **alta**, **média**, **baixa**.
Itens concluídos são movidos para a seção _Concluídas_ (mantendo o histórico).

Roadmap do motor pt-BR: `docs/ULTRAPLAN.md` (ULTRAPLAN v2, 2026-09-26), por
enquanto só no branch `feat/corpus-f0b` (PR #8). As fases e prioridades abaixo
seguem o §3 (fases) e o §6 (decisões de 2026-09-27) do plano.

## A fazer

### Entrega e CI (alta)

- [ ] PR #12 (rascunho): remove a aprovação humana do pipeline (CI →
      `Pipeline complete`). Fazer merge.
- [ ] PR #8 (rascunho): corpus F0b + `docs/ULTRAPLAN.md`. Depois do merge do
      #12, atualizar o branch com o `main` e fazer merge.
- [ ] PR #1 (rascunho do Copilot, atrás do `main`): outra abordagem para
      builds sem credenciais de assinatura, problema já resolvido no `main`
      pelo #6/#7. Decidir se fecha.

### F0a — Harness mínimo (alta)

- [ ] CLI `--manifest m.jsonl`, `--language`, `--vad {off,silero,earshot}`
      (mesmo código de VAD do app); saída JSON por fala com texto cru, final,
      trace por estágio e tempos.
- [ ] Métricas: WER/CER normalizado e formatado; P/R/F1 por `, . ? ! :`; ITN
      exact-match; preservação de palavras.
- [ ] Trace de latência no app (tecla → 1º quadro → 1ª fala no VAD → 1º feed →
      1ª emissão; soltar → colar) para diagnosticar os 3–4 s do issue #4.
- [ ] Linha de base: config atual (small + auto + Earshot) × turbo + `pt` ×
      Silero vs Earshot, com FLEURS pt_br (200 falas), 20–30 falas próprias e
      20 sondas de silêncio.

### F0b — Corpus pessoal (alta; depende de você)

- [ ] Antes de gravar: mudar a retenção do histórico para "Nunca apagar" (o
      padrão guarda só os 5 últimos áudios).
- [ ] Gravar os 8 blocos de `docs/corpus-f0b/` (131 falas + 24 sondas de
      silêncio, ~60 min) e acompanhar com `scripts/corpus_f0b.py status`.
- [ ] Conjunto de concordância: ≥ 200 frases corretas + ≥ 200 com erros
      injetados e rotulados (concordância nominal/verbal, crase, homófono).
- [ ] CORAA v1.1 (500 falas estratificadas) como controle.
- [ ] Privacidade: dados fora do git, opt-in separado para áudio e texto,
      retenção e apagar, cifrado.

### F1 — Modelo e decode (alta)

- [ ] Ativar Whisper large-v3-turbo Q8_0 + idioma `pt`, com migração única
      ("Você dita em português?"), porque o `auto` já está gravado.
- [ ] Prompt do Whisper: 2–3 frases pt bem pontuadas + glossário, com probe
      de eco.
- [ ] Knobs de decode: `temperature_fallback`, `no_speech_thold`, `n_threads`
      nos P-cores; `pnc`/`itn` quando o modelo suportar.
- [ ] Stop do Preview: descartar o trecho em curso e ir direto ao passe final.
- [ ] Nemotron: `pt` forçado × auto e presets 0/3/6/13.
- [ ] Quantização pt: Q8_0 × Q5_K_M × Q4_K_M.
- [ ] Métrica e recomendação pt-BR num arquivo overlay separado do
      `catalog.json`, que o upstream regenera inteiro.

### F2a — Correções testáveis sem corpus (alta)

- [ ] Léxico de siglas e caixa exata ("Stf" → "STF").
- [ ] Muletas pt ("é", "hã", "ahn"); "né" e "tipo" removidos por padrão
      (decisão 5).
- [ ] Vocabulário Unicode sem acento + fonética pt; regras X→Y na UI; S2
      rodando também para Whisper.
- [ ] Meses e dias minúsculos.
- [ ] Núcleo aplicado ao overlay.
- [ ] Testes unitários e property test de preservação.

### F6 — Ao vivo e energia (alta; decisão 4)

- [ ] 2º slot de engine: Nemotron ao vivo + turbo final; ao soltar a tecla,
      cancelar o stream e rodar o final.
- [ ] Spike Apple SpeechTranscriber (ANE).
- [ ] Harness de replay a 1× para medir o 1º parcial.
- [ ] Camada C: troca ao desconectar da tomada, pré-carga em background e
      dispositivo por modelo (hoje o acelerador é global).

### F2b — S0 + ITN + comandos (média)

- [ ] S0 no recorder: relógio real por segmento de VAD e `gap_ms`.
- [ ] ITN pt-BR com `NumberStyle` (padrão `18h30` e `R$ 15.000`, decisão 6) e
      goldens (ITN pt do NeMo portado + 22 armadilhas).
- [ ] Comandos de voz (§2.5) com match mais longo, contexto numérico, veto por
      determinante, gate de pausa e dedupe.
- [ ] "apaga isso" opt-in.

### F3 — Estrutura e render (média)

- [ ] Parágrafos e listas (S10), com limiares ajustados no corpus F0b.
- [ ] Paste rico (`public.html` + texto) nos apps que aceitam (decisão 3);
      contar a leitura HTML como recibo na restauração da área de
      transferência.
- [ ] Tabela bundle ID → classe de app; matriz: Notes, Pages, Mail, Word,
      Docs, Slack, WhatsApp, Obsidian, VS Code, Terminal.

### F4 — Pontuação ONNX (média)

- [ ] A/B romance × MiniLM via `ort`, só em trechos fracos, pré-carregado no
      início da gravação.

### F5 — Concordância e polish (média)

- [ ] Guardas S9 primeiro, com a fonte de lemas definida.
- [ ] Medir os três candidatos da §2.6 (LanguageTool/LingoTweaker,
      Qwen3-1.7B via `llama-server`, Apple Foundation Models) no conjunto de
      concordância.
- [ ] Polish como opção do fluxo normal; local por padrão, nuvem opcional
      (decisão 2).
- [ ] Endurecer o LLM: timeout, prompts por idioma, sem retry, chaves no
      Keychain; nunca colar texto cru do LLM (`actions.rs:326-334`).

### F7 — Desafiantes (baixa)

- [ ] Parakeet v3 pt-BR FT → GGUF; medir pontuação.
- [ ] Soak de 10 min do Voxtral Realtime.
- [ ] Qwen3-ASR 1.7B e Voxtral Mini 3B como desafiantes da camada final.

### Fora do ULTRAPLAN (baixa)

- [ ] `release.yml`, `build-test.yml` e `pr-test-build.yml` (só
      `workflow_dispatch`) continuam com `sign-binaries: true`, caminho que
      exige credenciais que o fork não tem.
- [ ] `LiveMode::ChunksPaste` (T4): reservado, tratado como `Preview` no
      backend e desabilitado na UI (`settings.rs`, `LiveModeSetting.tsx`).

## Concluídas

- [x] 2026-09-15 — Fork e infra: rebrand Handy Live PT + pipeline de entrega
      (`990e6b0`).
- [x] 2026-09-16 — Validação ponta a ponta dos gates do pipeline com o teste da
      matriz `OverlayStyle` × `is_streaming` (#2).
- [x] 2026-09-16 — T2: `stream_latency_preset` para streaming nativo (#3).
- [x] 2026-09-16 — T0: Nemotron 3.5 streaming em pt-BR no Handy 0.9.6
      oficial; veredito "Whisper inevitável" → T3 (issue #4).
- [x] 2026-09-16 — T3: modo `Preview` (pseudo-streaming por segmento de VAD)
      para modelos Whisper (#5).
- [x] 2026-09-16 — Build local sem chave de assinatura
      (`createUpdaterArtifacts` desligado, `6de4a6b`).
- [x] 2026-09-26 — "Main Branch Build" sem assinatura e dylibs carregando no
      macOS sem assinatura (#6, #7).
- [x] 2026-09-27 — ULTRAPLAN v2 com as 6 decisões respondidas (PR #8).
- [x] 2026-10-12 — Gate de revisão Kilo removido (#11, `eb9bcd2`): o projeto
      não usa mais o KiloCode. PR #9 fechado e secret `KILO_API_KEY` apagado.
