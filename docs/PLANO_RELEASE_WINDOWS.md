# Plano de release para Windows

> Estado: plano, nada publicado. Base verificada em 13/10/2026: `main` em `e9d5885` mais o #23. Veio do `devrafaelbrauner/handy-live-pt-cloud#2` (repositório arquivado), que planejava o mesmo release no fork derivado antes de a transcrição na nuvem chegar aqui (#21).

## Objetivo

Publicar no GitHub Releases de `devrafaelbrauner/handy-live-pt` os instaladores Windows do Handy Live PT. Eles devem instalar num PC com Windows 10 ou 11 que não tenha nenhuma ferramenta de desenvolvimento.

## Onde estamos

- **O CI já gera os instaladores Windows sem assinatura.** O "Main Branch Build" do `e9d5885` ([run 38405745641](https://github.com/devrafaelbrauner/handy-live-pt/actions/runs/38405745641)) passou nas 7 plataformas. Os jobs Windows:

  | Job                      | Duração | Artefato (expira em 12/11/2026)                                                            |
  | ------------------------ | ------- | ------------------------------------------------------------------------------------------ |
  | `windows-latest` (x64)   | 13 min  | `handy-x86_64-pc-windows-msvc`, 56 MB: instalador NSIS `.exe` (20 MB) e MSI `.msi` (39 MB) |
  | `windows-11-arm` (ARM64) | 14 min  | `handy-aarch64-pc-windows-msvc`, 41 MB                                                     |

- **Os pacotes já são auditados no CI** (`.github/workflows/build.yml`, passo "Audit Windows package runtime contents"):
  - o NSIS é instalado em modo portátil silencioso e o MSI é extraído;
  - confere-se o runtime VC++ junto do app e, no x64, o `onnxruntime.dll` e as DLLs de transcribe/ggml;
  - roda-se `handy-live-pt.exe --list-devices`, e o ARM64 falha se expuser GPU.
- **Uso real:** os instaladores NSIS do "Main Branch Build" são instalados por cima num PC com Windows e GTX 1060 desde 12/10/2026, nos testes da F0a e da F6.
- **Arquiteturas:** no x64 há aceleração por GPU (Vulkan) e ONNX Runtime; no ARM64 só CPU.
- **Identidade própria:** `productName` "Handy Live PT", `identifier` `com.rafael.handylive`, executável `handy-live-pt.exe` e versão `0.9.6`. Pasta, atalhos, dados, executável e desinstalação são separados dos do Handy oficial. A única colisão é o atalho padrão `Ctrl+Space`, se os dois estiverem abertos.
- **Sem assinatura de código:** o fork não tem certificado, e o "Main Branch Build" usa o caminho sem assinatura desde o #6/#7.

## O que impede o release hoje

1. **Versão e tag.**
   - O `release.yml` cria a tag `v<versão>` com a versão do `tauri.conf.json`, hoje `0.9.6`.
   - O fork herdou do upstream a tag `v0.9.6` (`af48dd6`, código do Handy oficial), então o rascunho ficaria preso ao commit errado. É preciso uma versão própria.
   - O MSI só aceita sufixo numérico até 65535 (`-N` ou `+N`), regra do `convert_version` do `tauri-bundler`. `0.9.6-1` vira `0.9.6.1` no MSI; algo como `0.9.6-pt.1` quebra o build MSI.
2. **O `release.yml` monta as 7 plataformas.** Com `sign-binaries: false`, os DMGs macOS saem com assinatura ad-hoc, e os pacotes Linux também vão para o rascunho. Num release só do Windows, apague esses anexos antes de publicar. O macOS tem plano próprio: [`PLANO_RELEASE_MACOS.md`](PLANO_RELEASE_MACOS.md).
3. **O atualizador não funciona.** `plugins.updater.endpoints` (em `src-tauri/tauri.conf.json`) já busca `devrafaelbrauner/handy-live-pt/releases/latest/download/latest.json`, mas a chave pública é a do upstream e `createUpdaterArtifacts` é `false`, então nenhum release publica `latest.json`. A verificação falha em silêncio; o log registra "update endpoint did not respond with a successful status code".
4. **Nenhum teste num PC Windows limpo.** Os testes até aqui foram num PC com ferramentas de desenvolvimento e o app já configurado.

## Fases

### F1 — PR "release Windows" (feito, #23)

Porta do `handy-live-pt-cloud#3`, sem a troca de versão e sem cortar a matriz do release:

- `release.yml`, `pr-test-build.yml` e `build-test.yml` com `sign-binaries: false`;
- `handy-live-pt.exe` no Windows: o instalador e o desinstalador NSIS fecham todo processo com o nome do executável, e os dois apps se chamavam `handy.exe`;
- `PORTABLE_RELEASES_URL` apontando para os releases deste repositório;
- erro de transcrição abre a janela principal, que fica oculta no uso pela bandeja;
- README com a seção "Instalar no Windows" e os caminhos `com.rafael.handylive`.

### F2 — Validação num PC Windows limpo

1. Instaladores:
   - de um commit do `main`: artefato `handy-x86_64-pc-windows-msvc` do "Main Branch Build";
   - de um PR: `gh workflow run "PR Test Build" -R devrafaelbrauner/handy-live-pt -f pr_number=<n>` e artefato `handy-pr-<n>-x86_64-pc-windows-msvc`. O workflow monta as 7 plataformas.
2. Roteiro num Windows 10 ou 11 x64 (anotar o resultado de cada item, com prints):
   1. Instalar pelo NSIS como usuário atual, sem admin, passando pelo SmartScreen. Abrir o app.
   2. Liberar o microfone em Configurações → Privacidade → Microfone, incluindo "aplicativos da área de trabalho".
   3. Ditar com `Ctrl+Space`, segurando e tocando, no Bloco de Notas, no Word e no Chrome. O texto precisa ser colado.
   4. Modelo local: baixar o Canary 1B v2, conferir em Avançado se usa Vulkan (GPU) ou CPU e ditar em pt-BR.
   5. Texto ao vivo: em Avançado, Modelo ao vivo = Nemotron; ditar e conferir o texto enquanto fala e o texto final do Canary ao soltar.
   6. Nuvem: API na Nuvem → OpenRouter ou OpenAI → colar a chave → ligar → ditar. Com uma chave inválida e a janela fechada, a janela precisa abrir com o erro.
   7. Conferir sobreposição, ícone na bandeja, "iniciar com o Windows", histórico e "tentar novamente".
   8. Com o Windows Defender ligado, nenhum alerta ao instalar e abrir.
   9. Com o Handy oficial aberto, instalar e depois desinstalar o Handy Live PT. O Handy oficial precisa continuar aberto e intacto.
   10. MSI: instalar como admin em outra máquina ou VM, abrir e desinstalar.

### F3 — Release (você aprova cada passo)

1. PR com a versão `0.9.6-1` em `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` e `src-tauri/Cargo.lock`. Merge e "Main Branch Build" verde.
   - Próximas versões: `0.9.6-2`, `0.9.6-3`…
   - Ao sincronizar com uma versão nova do upstream (ex.: 0.9.7), volta para `0.9.7-1`.
2. Disparar o rascunho: `gh workflow run release.yml -R devrafaelbrauner/handy-live-pt`. O workflow cria o rascunho `v0.9.6-1` e anexa os pacotes das 7 plataformas.
3. Conferir os anexos:
   - baixar o NSIS e o MSI;
   - gerar o hash com `Get-FileHash -Algorithm SHA256`;
   - instalar o NSIS do rascunho num PC limpo;
   - apagar os anexos que não forem publicados.
4. Notas do release em pt-BR:
   - o que é: fork do Handy para pt-BR, texto ao vivo com qualquer modelo final e transcrição na nuvem opcional;
   - aviso do SmartScreen e hashes SHA-256;
   - requisitos: Windows 10/11 x64 e WebView2 (o instalador baixa se faltar, então precisa de internet).
5. Publicar o rascunho.

### F4 — Depois do release (opcional, conforme as decisões)

- **Atualização automática própria:**
  1. gerar o par de chaves com `bun tauri signer generate -w ~/.tauri/handy-live-pt.key`;
  2. colocar a chave pública em `plugins.updater.pubkey` e ligar `createUpdaterArtifacts`;
  3. cadastrar os segredos `TAURI_SIGNING_PRIVATE_KEY` e `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`;
  4. ajustar o build sem assinatura de código para repassar essas duas variáveis. Hoje ele não repassa nenhuma variável de assinatura.

  A partir daí cada release publica um `latest.json` e o app atualiza sozinho.

- **Assinatura de código:**
  - Certum "Open Source Code Signing in the Cloud": US$ 58 por ano em certum.store (preço de 05/10/2026), emitido para pessoa física com o prefixo "Open Source Developer", validade de 1 ano, chave na nuvem SimplySign (app no celular e SimplySign Desktop).
  - A assinatura é feita com `signtool` num Windows com SimplySign conectado, como passo manual entre o rascunho e a publicação. A automação fica para depois.
  - A reputação no SmartScreen cresce com os downloads; o aviso pode continuar nas primeiras versões.
- **winget:** só depois de URLs estáveis e, de preferência, com assinatura.

## Decisões suas

1. **Assinatura**
   - (a) Sem assinatura agora, para uso próprio e nas máquinas da unidade. **Recomendado.**
   - (b) Certum Open Source, US$ 58 por ano.
   - O Azure Artifact Signing (antigo Trusted Signing) não atende pessoa física no Brasil: a confiança pública é só para organizações dos EUA, Canadá, União Europeia e Reino Unido.
   - A SignPath Foundation (gratuita) exige reputação pública do projeto, que este repositório ainda não tem.
2. **Arquiteturas:** só x64 (**recomendado**) ou x64 + ARM64. O ARM64 roda só em CPU.
3. **Instaladores:** publicar NSIS e MSI (**recomendado**) ou só NSIS.
   - O NSIS instala por usuário, sem admin, e oferece modo portátil.
   - O MSI instala para a máquina (exige admin) e serve para implantação gerenciada.
4. **Versão:** esquema `0.9.6-1` (**recomendado**) ou outro sufixo numérico. Ficar em `0.9.6` não é opção, por causa da tag herdada.
5. **Atualização automática:**
   - só manter as URLs já apontadas agora e montar o canal próprio na F4 (**recomendado**);
   - ou montar o canal já no primeiro release.
6. **WebView2:**
   - padrão: o instalador baixa se faltar (**recomendado**);
   - `offlineInstaller`, cerca de +130 MB, para PCs sem internet.

## Riscos

- **Antivírus:** SmartScreen e Defender podem sinalizar um app sem assinatura que instala hook global de teclado e cola texto simulando o teclado. Se acontecer, enviar o arquivo em https://www.microsoft.com/wdsi/filesubmission.
- **Lentidão na GPU:** numa GTX 1060 (Vulkan), o large-v3-turbo e o Granite caem de ~2 s para ~24,6 s por fala depois de 10–60 transcrições (`TAREFAS_PENDENTES.md`, F0a). Não recomendar esses modelos nas notas antes de investigar.
- **Atalho:** `Ctrl+Space` colide com o Handy oficial se os dois estiverem abertos.
- **Atualizar por cima de um build antigo:** builds anteriores ao #23 instalam `handy.exe`, e o instalador novo procura `handy-live-pt.exe`. Feche o app antes de instalar por cima.
- **Prazo dos artefatos:** os artefatos do CI expiram em 30 dias. O que vale para distribuição é o anexo do release.
